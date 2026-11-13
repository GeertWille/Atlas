import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, expect, test, vi } from "vitest";
import type { Settings, SettingsPatch } from "@/bindings";
import { api } from "@/lib/api";
import { settingsFixture } from "@/test/fixtures";
import { useSaveProxyProfile, useSettings, useUpdateSettings } from "./hooks";

const STORED: Settings = { ...settingsFixture, hasTypesafeKey: true };

/** A backend whose saves the test answers by hand, each with the settings as they were when it was applied. */
function controllableBackend() {
  let stored = STORED;
  const answers: Array<() => void> = [];
  const failures: Array<() => void> = [];
  vi.spyOn(api, "getSettings").mockImplementation(async () => stored);
  vi.spyOn(api, "updateSettings").mockImplementation((patch: SettingsPatch) => {
    stored = {
      ...stored,
      ...patch,
      chromePath: patch.chromePath === undefined ? stored.chromePath : patch.chromePath,
      browserbase: { ...stored.browserbase, ...patch.browserbase },
    };
    const applied = stored;
    return new Promise<Settings>((resolve, reject) => {
      answers.push(() => resolve(applied));
      failures.push(() => {
        stored = STORED;
        reject(new Error("the database is locked"));
      });
    });
  });
  return { answer: (save: number) => answers[save]?.(), fail: (save: number) => failures[save]?.() };
}

async function renderSettings() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const view = renderHook(
    () => ({ settings: useSettings().data, update: useUpdateSettings(), saveProxy: useSaveProxyProfile() }),
    { wrapper },
  );
  await waitFor(() => expect(view.result.current.settings).toEqual(STORED));
  return { ...view, savesUnderWay: () => client.isMutating() };
}

afterEach(() => vi.restoreAllMocks());

test("a save's answer never undoes what a later save already shows", async () => {
  const backend = controllableBackend();
  const { result, savesUnderWay } = await renderSettings();

  act(() => result.current.update.mutate({ jevEnabled: true }));
  act(() => result.current.update.mutate({ defaultDevice: "mobile" }));
  await waitFor(() => expect(result.current.settings).toMatchObject({ jevEnabled: true, defaultDevice: "mobile" }));

  // The first answer was written before the second save was applied.
  await act(async () => backend.answer(0));
  await waitFor(() => expect(savesUnderWay()).toBe(1));
  expect(result.current.settings).toMatchObject({ jevEnabled: true, defaultDevice: "mobile" });
  await act(async () => backend.answer(1));
  await waitFor(() => expect(savesUnderWay()).toBe(0));
  expect(result.current.settings).toMatchObject({ jevEnabled: true, defaultDevice: "mobile" });
});

test("answers that arrive in the wrong order leave the stored settings on screen", async () => {
  const backend = controllableBackend();
  const { result } = await renderSettings();

  act(() => result.current.update.mutate({ headless: false }));
  act(() => result.current.update.mutate({ analysisEffort: "high" }));
  await waitFor(() => expect(result.current.settings).toMatchObject({ headless: false, analysisEffort: "high" }));

  await act(async () => backend.answer(1));
  // The stale answer comes last and lacks the second save.
  await act(async () => backend.answer(0));
  await waitFor(() => expect(api.getSettings).toHaveBeenCalledTimes(2));
  expect(result.current.settings).toMatchObject({ headless: false, analysisEffort: "high" });
});

test("a save the backend refuses is taken back", async () => {
  const backend = controllableBackend();
  const { result } = await renderSettings();

  act(() => result.current.update.mutate({ budgetUsd: 2 }));
  await waitFor(() => expect(result.current.settings?.budgetUsd).toBe(2));
  await act(async () => backend.fail(0));
  await waitFor(() => expect(result.current.settings?.budgetUsd).toBe(0.5));
});

test("a proxy command answered while a field is being saved does not undo what that field shows", async () => {
  const backend = controllableBackend();
  const belgium = { id: "p1", name: "Belgium", scheme: "http", host: "be.proxy.example", port: 8080 } as const;
  // The proxy is stored before the field's save is applied: its answer still has the old visitor.
  vi.spyOn(api, "saveProxyProfile").mockImplementation(async () => ({
    ...STORED,
    proxies: [{ ...belgium, username: null, hasPassword: false }],
  }));
  const { result, savesUnderWay } = await renderSettings();

  act(() => result.current.update.mutate({ defaultVisitor: "browse" }));
  await waitFor(() => expect(result.current.settings?.defaultVisitor).toBe("browse"));
  await act(async () => {
    await result.current.saveProxy.mutateAsync({ ...belgium, id: undefined, username: null });
  });
  expect(result.current.settings?.defaultVisitor).toBe("browse");

  await act(async () => backend.answer(0));
  await waitFor(() => expect(savesUnderWay()).toBe(0));
  // The reload after the last save brings what is stored: the field's value, whatever else changed meanwhile.
  await waitFor(() => expect(api.getSettings).toHaveBeenCalledTimes(2));
  expect(result.current.settings?.defaultVisitor).toBe("browse");
});

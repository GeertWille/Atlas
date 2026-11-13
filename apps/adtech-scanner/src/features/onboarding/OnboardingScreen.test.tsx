import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import type { BrowserInfo } from "@/bindings";
import { api } from "@/lib/api";
import { browserbaseSettingsFixture, settingsFixture } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { OnboardingScreen } from "./OnboardingScreen";

// Runs against the real api module: outside Tauri that is the fake backend.
vi.hoisted(() => {
  localStorage.setItem("adscan.fake.profile", "fresh");
  localStorage.setItem("adscan.fake.speed", "0.001");
});

const CHROME: BrowserInfo = { path: "/usr/bin/chromium", version: "Chromium 141", name: "Chromium" };
const steps = () => screen.getAllByRole("listitem");

beforeEach(async () => {
  localStorage.removeItem("adscan.onboarding.done");
  await api.deleteAllScans();
});

test("without a local browser there are two ways on: choose one, or set up Browserbase", async () => {
  const user = userEvent.setup();
  const onDone = vi.fn();
  const { router } = await renderWithApp(
    <OnboardingScreen browser={null} settings={settingsFixture} onDone={onDone} />,
  );
  const [browser, , demo] = steps();
  expect(within(browser as HTMLElement).getByRole("heading")).toHaveTextContent("No Chrome or Chromium found");
  expect(browser).toHaveTextContent(
    "Without one, scans can run in Browserbase's cloud browser instead: the scanned pages then pass through Browserbase, a paid third-party service.",
  );
  expect(within(browser as HTMLElement).getByRole("button", { name: "Choose browser…" })).toBeEnabled();
  // The demo site is on this Mac: it needs a local browser whatever else is set up.
  expect(demo).toHaveTextContent("It is served from this Mac and needs a local browser.");
  expect(within(demo as HTMLElement).getByRole("button", { name: "Scan the demo site" })).toBeDisabled();

  await user.click(within(browser as HTMLElement).getByRole("button", { name: "Set up Browserbase instead" }));
  expect(await screen.findByText("settings screen")).toBeInTheDocument();
  expect(router.state.location.pathname).toBe("/settings");
  // Setup is not shown again on the way back.
  expect(onDone).toHaveBeenCalledOnce();
  expect(localStorage.getItem("adscan.onboarding.done")).toBe("1");
});

test("with Browserbase set up, a missing local browser is no longer something to fix", async () => {
  await renderWithApp(<OnboardingScreen browser={null} settings={browserbaseSettingsFixture} onDone={() => {}} />);
  const [browser] = steps();
  expect(within(browser as HTMLElement).getByRole("heading")).toHaveTextContent("Browserbase is set up");
  expect(browser).toHaveTextContent(
    "No Chrome or Chromium was found on this Mac, so scans run in Browserbase's cloud browser.",
  );
  expect(within(browser as HTMLElement).queryByRole("button", { name: "Set up Browserbase instead" })).toBeNull();
});

test("the demo always runs in this Mac's Chrome, and says so when Browserbase is the default", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials("bb_live_0123456789abcdef");
  await api.updateSettings({ defaultBrowser: "browserbase" });
  const cloudDefault = { ...browserbaseSettingsFixture, defaultBrowser: "browserbase" as const };
  await renderWithApp(<OnboardingScreen browser={CHROME} settings={cloudDefault} onDone={() => {}} />);
  const demo = steps()[2] as HTMLElement;
  expect(demo).toHaveTextContent(
    "It is served from this Mac, so this scan runs in this Mac's Chrome, not on Browserbase.",
  );
  await user.click(within(demo).getByRole("button", { name: "Scan the demo site" }));
  expect(await screen.findByText("scan screen")).toBeInTheDocument();
  const [scan] = (await api.listScans({})).items;
  expect((await api.getScan(scan?.id ?? "")).options.browser).toEqual({ kind: "local" });
  await api.clearBrowserbaseCredentials();
});

test("with a local browser and Browserbase not the default, the demo step says nothing about where it runs", async () => {
  await renderWithApp(<OnboardingScreen browser={CHROME} settings={settingsFixture} onDone={() => {}} />);
  expect(steps()[2]).not.toHaveTextContent(/this Mac/);
});

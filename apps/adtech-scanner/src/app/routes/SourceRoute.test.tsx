import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import { ApiError, api } from "@/lib/api";
import { createQueryClient } from "@/lib/query/client";
import { queryKeys } from "@/lib/query/keys";
import { TooltipProvider } from "@/shared/ui/tooltip";
import { router } from "../router";

const SCAN = "seed-sportsdesk-desktop";
const locked = () => new ApiError("storage", "The database is locked");

test("a refresh that fails keeps the script and the file list on screen; only a first load that fails is an error", async () => {
  const user = userEvent.setup();
  const scan = await api.getScan(SCAN);
  const readable: string[] = [];
  for (const script of scan.scripts) {
    const source = await api.getScriptSource(SCAN, script.id, "formatted");
    if (source.availability === "available") readable.push(script.id);
    if (readable.length === 2) break;
  }
  const [first = "", second = ""] = readable;

  const client = createQueryClient();
  await router.navigate({ to: "/scans/$scanId/source/$scriptId", params: { scanId: SCAN, scriptId: first } });
  render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <RouterProvider router={router} />
      </TooltipProvider>
    </QueryClientProvider>,
  );
  const main = within(await screen.findByRole("main"));
  const code = await main.findByTestId("code", {}, { timeout: 5_000 });
  await waitFor(() => expect(code.textContent?.length).toBeGreaterThan(0));
  const shown = code.textContent;
  expect(main.getByRole("navigation", { name: "Scripts" })).toBeInTheDocument();

  // The scan is fetched again and that fails.
  const getScan = vi.spyOn(api, "getScan").mockRejectedValueOnce(locked());
  await client.invalidateQueries({ queryKey: queryKeys.scanDetail(SCAN) });
  let notice = within(await main.findByRole("alert"));
  expect(notice.getByText("Could not refresh this scan")).toBeInTheDocument();
  expect(notice.getByText(/The database is locked/)).toBeInTheDocument();
  // The same editor, not a new one: the reader's place in the file is kept.
  expect(main.getByTestId("code")).toBe(code);
  expect(code.textContent).toBe(shown);
  expect(main.getByRole("navigation", { name: "Scripts" })).toBeInTheDocument();
  expect(main.queryByText("Could not load this scan")).toBeNull();
  await user.click(notice.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(main.queryByRole("alert")).toBeNull());
  expect(getScan).toHaveBeenCalledTimes(2);
  expect(main.getByTestId("code")).toBe(code);

  // The source text is fetched again and that fails.
  const getSource = vi.spyOn(api, "getScriptSource").mockRejectedValueOnce(locked());
  await client.invalidateQueries({ queryKey: queryKeys.scriptSource(SCAN, first, "formatted") });
  notice = within(await main.findByRole("alert"));
  expect(notice.getByText("Could not refresh this script")).toBeInTheDocument();
  expect(main.getByTestId("code")).toBe(code);
  expect(code.textContent).toBe(shown);
  expect(main.queryByText("Could not load this script")).toBeNull();
  await user.click(notice.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(main.queryByRole("alert")).toBeNull());
  expect(main.getByTestId("code")).toBe(code);

  // Another file that cannot be loaded has no text to keep: the error stands where the code would, beside the file list.
  getSource.mockRejectedValueOnce(locked());
  await router.navigate({ to: "/scans/$scanId/source/$scriptId", params: { scanId: SCAN, scriptId: second } });
  expect(await main.findByText("Could not load this script")).toBeInTheDocument();
  expect(main.queryByTestId("code")).toBeNull();
  expect(main.queryByRole("alert")).toBeNull();
  expect(main.getByRole("navigation", { name: "Scripts" })).toBeInTheDocument();
  await user.click(main.getByRole("button", { name: "Try again" }));
  expect(await main.findByTestId("code")).toBeInTheDocument();
});

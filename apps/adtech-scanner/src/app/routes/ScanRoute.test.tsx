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
const OTHER_SCAN = "seed-sportsdesk-mobile";

test("a refresh that fails keeps the loaded report on screen; only a first load that fails is an error page", async () => {
  const user = userEvent.setup();
  const client = createQueryClient();
  await router.navigate({ to: "/scans/$scanId", params: { scanId: SCAN } });
  render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <RouterProvider router={router} />
      </TooltipProvider>
    </QueryClientProvider>,
  );
  const main = within(await screen.findByRole("main"));
  expect(await main.findByRole("tab", { name: /^Findings/ }, { timeout: 5_000 })).toBeInTheDocument();

  const getScan = vi.spyOn(api, "getScan").mockRejectedValueOnce(new ApiError("storage", "The database is locked"));
  await client.invalidateQueries({ queryKey: queryKeys.scanDetail(SCAN) });

  const notice = within(await main.findByRole("alert"));
  expect(notice.getByText("Could not refresh this scan")).toBeInTheDocument();
  expect(notice.getByText(/The database is locked/)).toBeInTheDocument();
  expect(main.getByRole("tab", { name: /^Findings/ })).toBeInTheDocument();
  expect(main.queryByText("Could not load this scan")).toBeNull();

  // The retry reaches the backend again, and a good answer takes the notice away.
  await user.click(notice.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(main.queryByText("Could not refresh this scan")).toBeNull());
  expect(main.getByRole("tab", { name: /^Findings/ })).toBeInTheDocument();
  expect(getScan).toHaveBeenCalledTimes(2);

  // A scan that was never loaded has nothing to keep showing.
  getScan.mockRejectedValueOnce(new ApiError("storage", "The database is locked"));
  await router.navigate({ to: "/scans/$scanId", params: { scanId: OTHER_SCAN } });
  expect(await main.findByText("Could not load this scan")).toBeInTheDocument();
  expect(main.queryByRole("tab", { name: /^Findings/ })).toBeNull();
  await user.click(main.getByRole("button", { name: "Try again" }));
  expect(await main.findByRole("tab", { name: /^Findings/ }, { timeout: 5_000 })).toBeInTheDocument();
});

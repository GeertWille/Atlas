import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import { ApiError, api } from "@/lib/api";
import { createQueryClient } from "@/lib/query/client";
import { queryKeys } from "@/lib/query/keys";
import { TooltipProvider } from "@/shared/ui/tooltip";
import { router } from "../router";

const SCAN = "seed-sportsdesk-desktop";
const OTHER_SCAN = "seed-sportsdesk-mobile";
const locked = () => new ApiError("storage", "The database is locked");

afterEach(() => {
  vi.restoreAllMocks();
});

async function openNetwork(scanId: string, search: { req?: string; dt?: "headers" } = {}) {
  const client = createQueryClient();
  // Not awaited: with no router on screen (the test before this one took it away) a navigation only
  // settles once the next one is rendered.
  void router.navigate({ to: "/scans/$scanId/network", params: { scanId }, search });
  render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <RouterProvider router={router} />
      </TooltipProvider>
    </QueryClientProvider>,
  );
  return { client, main: within(await screen.findByRole("main", {}, { timeout: 5_000 })) };
}

const GRID = { name: "Requests" };

test("a refresh of the scan that fails keeps the network view on screen; only a first load that fails is an error page", async () => {
  const user = userEvent.setup();
  const { client, main } = await openNetwork(SCAN);
  await main.findByRole("treegrid", GRID, { timeout: 5_000 });

  const getScan = vi.spyOn(api, "getScan").mockRejectedValueOnce(locked());
  await client.invalidateQueries({ queryKey: queryKeys.scanDetail(SCAN) });

  const notice = within(await main.findByRole("alert"));
  expect(notice.getByText("Could not refresh this scan")).toBeInTheDocument();
  expect(notice.getByText(/The database is locked/)).toBeInTheDocument();
  expect(main.getByRole("treegrid", GRID)).toBeInTheDocument();
  expect(main.queryByText("Could not load this scan")).toBeNull();

  await user.click(notice.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(main.queryByText("Could not refresh this scan")).toBeNull());
  expect(main.getByRole("treegrid", GRID)).toBeInTheDocument();
  expect(getScan).toHaveBeenCalledTimes(2);

  getScan.mockRejectedValueOnce(locked());
  await router.navigate({ to: "/scans/$scanId/network", params: { scanId: OTHER_SCAN } });
  expect(await main.findByText("Could not load this scan")).toBeInTheDocument();
  expect(main.queryByRole("treegrid")).toBeNull();
  await user.click(main.getByRole("button", { name: "Try again" }));
  expect(await main.findByRole("treegrid", GRID, { timeout: 5_000 })).toBeInTheDocument();
});

test("a later chunk of requests that fails keeps the rows that were loaded and says the list is incomplete", async () => {
  const user = userEvent.setup();
  const total = (await api.listNetworkRequests(SCAN, { limit: 1 })).total;
  const chunk = Math.ceil(total / 3);
  const listRows = api.listNetworkRequests;
  let failing = true;
  const list = vi.spyOn(api, "listNetworkRequests").mockImplementation((scanId, query) => {
    if (failing && query?.cursor) return Promise.reject(locked());
    return listRows(scanId, { ...query, limit: chunk });
  });
  const { main } = await openNetwork(SCAN);

  const notice = within(await main.findByRole("alert"));
  expect(notice.getByText("The request list is incomplete")).toBeInTheDocument();
  expect(notice.getByText(/The database is locked/)).toBeInTheDocument();
  expect(notice.getByText(`The first ${chunk} of ${total} requests are shown.`)).toBeInTheDocument();
  expect(main.getByRole("treegrid", GRID)).toBeInTheDocument();
  expect(main.getByTestId("request-count")).toHaveTextContent(`${chunk} requests`);
  expect(main.queryByText("Could not load the requests")).toBeNull();
  expect(main.queryByText(/loading more/)).toBeNull();
  // The chunk that failed is not asked for again until the reader says so.
  await new Promise((resolve) => setTimeout(resolve, 200));
  expect(list).toHaveBeenCalledTimes(2);

  failing = false;
  await user.click(notice.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(main.getByTestId("request-count")).toHaveTextContent(`${total} requests`));
  expect(main.queryByRole("alert")).toBeNull();
  // The retry went on from the chunk that failed: the first one was read once.
  expect(list.mock.calls.filter(([, query]) => !query?.cursor)).toHaveLength(1);
});

test("a first chunk of requests that fails is an error in the list, and a refresh that fails keeps the list", async () => {
  const user = userEvent.setup();
  const list = vi.spyOn(api, "listNetworkRequests").mockRejectedValueOnce(locked());
  const { client, main } = await openNetwork(SCAN);
  expect(await main.findByText("Could not load the requests", {}, { timeout: 5_000 })).toBeInTheDocument();
  expect(main.queryByRole("alert")).toBeNull();
  await user.click(main.getByRole("button", { name: "Try again" }));
  expect(await main.findByRole("treegrid", GRID)).toBeInTheDocument();
  const count = main.getByTestId("request-count").textContent;

  list.mockRejectedValueOnce(locked());
  await client.invalidateQueries({ queryKey: queryKeys.networkRows(SCAN) });
  const notice = within(await main.findByRole("alert"));
  expect(notice.getByText("Could not refresh the requests")).toBeInTheDocument();
  expect(main.getByRole("treegrid", GRID)).toBeInTheDocument();
  expect(main.getByTestId("request-count")).toHaveTextContent(count ?? "");

  await user.click(notice.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(main.queryByRole("alert")).toBeNull());
  expect(main.getByRole("treegrid", GRID)).toBeInTheDocument();
});

test("the details of a newly selected request that fail to load are an error inside the panel; a refresh that fails keeps them", async () => {
  const user = userEvent.setup();
  const { rows } = await api.listNetworkRequests(SCAN, { limit: 1 });
  const requestId = rows[0]?.id ?? "";
  const getRequest = vi.spyOn(api, "getNetworkRequest").mockRejectedValueOnce(locked());
  const { client, main } = await openNetwork(SCAN, { req: requestId, dt: "headers" });

  const panel = within(await main.findByRole("region", { name: "Request details" }));
  expect(await panel.findByText("Could not load this request")).toBeInTheDocument();
  expect(panel.getByText(/The database is locked/)).toBeInTheDocument();
  expect(main.getByRole("treegrid", GRID)).toBeInTheDocument();
  // The facts the list holds are still there to read.
  await user.click(panel.getByRole("tab", { name: "Summary" }));
  expect(within(panel.getByRole("alert")).getByText("What the request list holds is shown.")).toBeInTheDocument();
  expect(panel.getByText("General")).toBeInTheDocument();

  await user.click(panel.getByRole("tab", { name: "Headers" }));
  await user.click(panel.getByRole("button", { name: "Try again" }));
  expect(await panel.findByText(/^Request headers/)).toBeInTheDocument();
  expect(panel.queryByText("Could not load this request")).toBeNull();

  getRequest.mockRejectedValueOnce(locked());
  await client.invalidateQueries({ queryKey: queryKeys.networkRequest(SCAN, requestId) });
  const notice = within(await panel.findByRole("alert"));
  expect(notice.getByText("Could not refresh this request")).toBeInTheDocument();
  expect(panel.getByText(/^Request headers/)).toBeInTheDocument();

  await user.click(notice.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(panel.queryByRole("alert")).toBeNull());
  expect(panel.getByText(/^Request headers/)).toBeInTheDocument();
});

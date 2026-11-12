import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import type { LogEntry, ScanDetail, ScanOptions } from "@/bindings";
import { api } from "@/lib/api";
import { useScan, useScanLogs } from "@/lib/query/hooks";
import { makeDetail, makeLog, makeStep, makeSummary } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { LiveScanView } from "./LiveScanView";

// The session bar talks to the real api module: outside Tauri that is the fake backend.
vi.hoisted(() => {
  localStorage.setItem("adscan.fake.profile", "fresh");
  localStorage.setItem("adscan.fake.speed", "400");
});

beforeEach(async () => {
  await api.deleteAllScans();
});

const STARTED = Date.now() - 134_000;
const baseOptions: ScanOptions = { ...makeDetail().options, visitor: "browse", extraPages: 2 };

function running(options: ScanOptions, captureStatus: "running" | "completed" = "running"): ScanDetail {
  return makeDetail({
    summary: makeSummary({
      status: "running",
      finishedAt: null,
      durationMs: null,
      healthScore: null,
      startedAt: STARTED,
    }),
    options,
    steps: [
      makeStep({ id: "s-launch", node: "launchBrowser" }),
      makeStep({ id: "s-capture", node: "capture", status: captureStatus, startedAt: STARTED, durationMs: null }),
      makeStep({
        id: "s-telemetry",
        node: "telemetry",
        status: captureStatus === "running" ? "pending" : "running",
        startedAt: captureStatus === "running" ? null : Date.now(),
        durationMs: null,
      }),
    ],
  });
}

let sequence = 0;
const visitor = (activity: string, message: string, detail: unknown, page = 0): LogEntry => {
  sequence += 1;
  return makeLog({ id: `v${sequence}`, kind: "visitor", message, data: { activity, page, detail } });
};
const browseLogs = [
  visitor("pageView", "Page 1: /news (entry page)", {
    index: 0,
    url: "https://www.example.com/news",
    title: "News",
    reachedBy: "entry",
  }),
  visitor("action", "Scrolled to 60%", { action: "scroll", scrollDepth: 0.6 }),
  visitor("linkChoice", "Following link: /sport/final (best of 5)", {
    chosen: { text: "Final", href: "https://www.example.com/sport/final", score: 6, reasons: [] },
    alternatives: [],
    skipped: { "off-site": 9, "inside-ad": 3, sensitive: 2 },
  }),
  visitor(
    "pageView",
    "Page 2: /sport/final (followed link)",
    {
      index: 1,
      url: "https://www.example.com/sport/final",
      // The backend announces a page before its title is known.
      title: null,
      reachedBy: "linkClick",
      link: { text: "Final", href: "https://www.example.com/sport/final", reason: "" },
    },
    1,
  ),
  visitor("action", "Moved the pointer over a heading", { action: "pointerMove" }, 1),
];

const view = (detail: ScanDetail, logs: LogEntry[] | undefined) => (
  <LiveScanView detail={detail} logs={logs} budgetUsd={0.5} activity={<p>activity feed</p>} />
);

test("while the capture runs, the visit panel shows the current page, the pages so far and the latest actions", async () => {
  await renderWithApp(view(running(baseOptions), browseLogs));
  const visit = screen.getByRole("region", { name: "Visit" });
  expect(within(visit).getByText("Now on page 2: /sport/final")).toBeInTheDocument();

  const pages = within(within(visit).getByRole("list", { name: "Pages visited" })).getAllByRole("listitem");
  expect(pages.map((page) => page.textContent)).toEqual(["1/news", "2/sport/final"]);
  expect(within(pages[0] as HTMLElement).getByRole("img", { name: "Entry page" })).toBeInTheDocument();
  expect(within(pages[1] as HTMLElement).getByRole("img", { name: "Followed link" })).toBeInTheDocument();

  const actions = within(within(visit).getByRole("list", { name: "Latest actions" })).getAllByRole("listitem");
  expect(actions.map((action) => action.textContent)).toEqual([
    "Scrolled to 60%",
    "Following link: /sport/final (best of 5)",
    "Moved the pointer over a heading",
  ]);
  expect(
    within(visit).getByText("Skipped 14 links: 9 off-site, 3 inside ads, 2 sign-in, checkout and the like"),
  ).toBeInTheDocument();
  // A scripted visit has no session to finish.
  expect(screen.queryByRole("region", { name: "Manual session" })).toBeNull();
  expect(screen.getByRole("button", { name: "Cancel scan" })).toBeInTheDocument();
});

test("there is no visit panel before the first page view, without logs, or once the capture is over", async () => {
  const { unmount } = await renderWithApp(view(running(baseOptions), [makeLog({ message: "Navigating" })]));
  expect(screen.queryByRole("region", { name: "Visit" })).toBeNull();
  unmount();
  const second = await renderWithApp(view(running(baseOptions), undefined));
  expect(screen.queryByRole("region", { name: "Visit" })).toBeNull();
  second.unmount();
  await renderWithApp(view(running(baseOptions, "completed"), browseLogs));
  expect(screen.queryByRole("region", { name: "Visit" })).toBeNull();
});

test("a manual session shows how long it has run, how long is left and what it has recorded", async () => {
  const manual = running({ ...baseOptions, visitor: "manual", extraPages: 0 });
  await renderWithApp(view(manual, browseLogs.slice(0, 1)));
  const bar = screen.getByRole("region", { name: "Manual session" });
  expect(within(bar).getByRole("heading", { name: "You are browsing www.example.com" })).toBeInTheDocument();
  expect(bar).toHaveTextContent("Pages of this site are recorded as you open them; other sites are not.");
  const reading = (label: string) => within(bar).getByText(label).nextElementSibling?.textContent;
  expect(reading("Elapsed")).toMatch(/^02:1[4-6]$/);
  expect(reading("Time left")).toMatch(/^12:4[4-6]$/);
  expect(reading("Recorded")).toBe("1 page");
  expect(within(bar).getByRole("button", { name: "Finish capture" })).toBeEnabled();
  // Cancelling lives in the bar while the session runs, and nowhere else.
  expect(screen.getAllByRole("button", { name: "Cancel scan" })).toHaveLength(1);
  expect(within(bar).getByRole("button", { name: "Cancel scan" })).toBeInTheDocument();
});

test("the manual session bar is gone once the capture step has finished", async () => {
  await renderWithApp(view(running({ ...baseOptions, visitor: "manual" }, "completed"), browseLogs));
  expect(screen.queryByRole("region", { name: "Manual session" })).toBeNull();
  expect(screen.getByRole("button", { name: "Cancel scan" })).toBeInTheDocument();
});

function ConnectedLive({ scanId }: { scanId: string }) {
  const scan = useScan(scanId);
  const logs = useScanLogs(scanId);
  return scan.data?.summary.status === "running" ? view(scan.data, logs.data) : <p>scan {scan.data?.summary.status}</p>;
}

test("Finish capture ends the recording, keeps the pages seen and lets the scan complete", async () => {
  const user = userEvent.setup();
  const scan = await api.startScan({ ...baseOptions, visitor: "manual", aiEnabled: false });
  // The scripted session reaches its third page and then waits for the user.
  await waitFor(
    async () => expect((await api.getLogs(scan.id)).filter((entry) => /^Page \d+:/.test(entry.message)).length).toBe(3),
    { timeout: 5000 },
  );
  const { queryClient } = await renderWithApp(<ConnectedLive scanId={scan.id} />);
  const bar = await screen.findByRole("region", { name: "Manual session" });
  // The count comes from the audit trail, which loads after the scan itself.
  await waitFor(() => expect(within(bar).getByText("Recorded").nextElementSibling).toHaveTextContent("3 pages"));

  await user.click(within(bar).getByRole("button", { name: "Finish capture" }));
  expect(await within(bar).findByRole("button", { name: "Finishing…" })).toBeDisabled();

  await waitFor(async () => expect((await api.getScan(scan.id)).summary.status).toBe("completed"), { timeout: 5000 });
  // In the app the scan's events update the caches; this render has no event subscription.
  await queryClient.invalidateQueries();
  expect(await screen.findByText("scan completed")).toBeInTheDocument();
  const detail = await api.getScan(scan.id);
  expect(detail.pages).toHaveLength(3);
  expect(detail.environment?.visitor).toBe("manual");
});

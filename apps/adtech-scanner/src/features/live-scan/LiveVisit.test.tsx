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
  // Nothing is checked during the capture: the empty findings panel makes room for the visit.
  expect(screen.queryByRole("region", { name: "Findings so far" })).toBeNull();
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
  expect(screen.getByRole("region", { name: "Findings so far" })).toBeInTheDocument();
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

test("Finish capture waits until Chrome has opened the page, because the backend ignores it before that", async () => {
  const manual = running({ ...baseOptions, visitor: "manual", extraPages: 0 });
  const { unmount } = await renderWithApp(view(manual, [makeLog({ message: "Launching Chrome (visible)" })]));
  const bar = screen.getByRole("region", { name: "Manual session" });
  expect(within(bar).getByRole("heading", { name: "Opening Chrome for www.example.com" })).toBeInTheDocument();
  expect(within(bar).getByRole("button", { name: "Finish capture" })).toBeDisabled();
  // Cancelling is always possible.
  expect(within(bar).getByRole("button", { name: "Cancel scan" })).toBeEnabled();
  unmount();

  // Without an audit trail to tell from, the button is not held back.
  await renderWithApp(view(manual, undefined));
  expect(screen.getByRole("button", { name: "Finish capture" })).toBeEnabled();
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

const frankfurt: ScanOptions = { ...baseOptions, browser: { kind: "browserbase", region: "eu-central-1" } };
const remoteEvent = (event: string, message: string, detail: Record<string, unknown> = {}, ts = Date.now()) => {
  sequence += 1;
  return makeLog({
    id: `r${sequence}`,
    ts,
    kind: "visitor",
    level: event === "connectionLost" ? "warn" : "info",
    message,
    data: { activity: "browser", page: null, detail: { event, ...detail } },
  });
};

test("a Browserbase scan says that it is starting a browser there, until that browser exists", async () => {
  await renderWithApp(view(running(frankfurt), [makeLog({ message: "Using Browserbase in eu-central-1" })]));
  expect(screen.getByText("Step 2 of 3: Starting a Browserbase browser (Frankfurt)…")).toBeInTheDocument();
  const visit = screen.getByRole("region", { name: "Visit" });
  expect(within(visit).getByText("Starting a Browserbase browser (Frankfurt)…")).toBeInTheDocument();
  expect(within(visit).queryByText(/Now on page/)).toBeNull();
  // The one step that differs from a scan on this Mac.
  const pipeline = screen.getByRole("list", { name: "Pipeline" });
  expect(pipeline).toHaveTextContent("Start a Browserbase browser in Frankfurt");
  expect(pipeline).not.toHaveTextContent("Start local Chrome over CDP");
});

test("once the browser runs, the visit panel names the region and counts its billed time", async () => {
  const started = Date.now() - 72_000;
  await renderWithApp(
    view(running(frankfurt), [
      remoteEvent(
        "sessionCreated",
        "Browserbase started a browser in eu-central-1",
        { region: "eu-central-1" },
        started,
      ),
      ...browseLogs,
    ]),
  );
  expect(screen.getByText("Step 2 of 3: Capture page")).toBeInTheDocument();
  const visit = screen.getByRole("region", { name: "Visit" });
  expect(within(visit).getByText("Browserbase Frankfurt")).toBeInTheDocument();
  expect(within(visit).getByText(/· running 01:1[2-4]/)).toBeInTheDocument();
  expect(within(visit).getByText("Now on page 2: /sport/final")).toBeInTheDocument();
});

test("waiting at Browserbase's limit and a lost connection show the backend's own message", async () => {
  const waiting = await renderWithApp(
    view(running(frankfurt), [
      remoteEvent("rateLimited", "Browserbase is at its limit; trying again in 5 s", { waitSeconds: 5, attempt: 1 }),
    ]),
  );
  const visit = screen.getByRole("region", { name: "Visit" });
  expect(within(visit).getByText("Browserbase is at its limit; trying again in 5 s")).toBeInTheDocument();
  expect(within(visit).getByText("Starting a Browserbase browser (Frankfurt)…")).toBeInTheDocument();
  waiting.unmount();

  await renderWithApp(
    view(running(frankfurt), [
      remoteEvent("sessionCreated", "started", { region: "eu-central-1" }, Date.now() - 30_000),
      remoteEvent("connectionLost", "The connection to the Browserbase browser was lost", {}, Date.now() - 4_000),
    ]),
  );
  const lost = screen.getByRole("region", { name: "Visit" });
  expect(within(lost).getByText("The connection to the Browserbase browser was lost")).toBeInTheDocument();
  expect(within(lost).getByText(/· ran 00:2[5-7]/)).toBeInTheDocument();
});

test("a scan on this Mac shows none of it", async () => {
  await renderWithApp(view(running(baseOptions), browseLogs));
  expect(screen.queryByText(/Browserbase/)).toBeNull();
  expect(screen.getByRole("list", { name: "Pipeline" })).toHaveTextContent("Start local Chrome over CDP");
});

test("a manual Browserbase visit is used through the live view, which can be opened again by scan id", async () => {
  const user = userEvent.setup();
  const openLiveView = vi.spyOn(api, "openBrowserbaseLiveView").mockResolvedValue();
  const manual: ScanOptions = { ...frankfurt, visitor: "manual", extraPages: 0 };
  const session = remoteEvent("sessionCreated", "started", { region: "eu-central-1" });
  const entry = browseLogs.slice(0, 1);

  // The page has opened, but the capture has not said yet that a person can take over.
  const early = await renderWithApp(view(running(manual), [session, ...entry]));
  const waiting = screen.getByRole("region", { name: "Manual session" });
  expect(within(waiting).getByRole("heading")).toHaveTextContent("Starting a Browserbase browser for www.example.com");
  expect(within(waiting).getByRole("button", { name: "Open live view" })).toBeDisabled();
  expect(within(waiting).getByRole("button", { name: "Finish capture" })).toBeDisabled();
  early.unmount();

  await renderWithApp(
    view(running(manual), [
      session,
      ...entry,
      remoteEvent("liveView", "The browser is ready for you in Browserbase's live view"),
    ]),
  );
  const bar = screen.getByRole("region", { name: "Manual session" });
  expect(within(bar).getByRole("heading")).toHaveTextContent("You are browsing www.example.com");
  expect(bar).toHaveTextContent(
    "Use Browserbase's live view, which opened in your own browser. Pages of this site are recorded as you open them; other sites are not. Browserbase bills the session until you finish.",
  );
  expect(within(bar).getByRole("button", { name: "Finish capture" })).toBeEnabled();
  expect(within(bar).getByRole("button", { name: "Cancel scan" })).toBeInTheDocument();

  await user.click(within(bar).getByRole("button", { name: "Open live view" }));
  // Only the scan's id goes out: the address of the live view never reaches this side.
  expect(openLiveView).toHaveBeenCalledWith("scan-1");
  expect(await screen.findByText("Live view opened in your browser")).toBeInTheDocument();

  openLiveView.mockRejectedValueOnce({
    code: "invalidInput",
    message: "This scan has no Browserbase browser to show right now.",
  });
  await user.click(within(bar).getByRole("button", { name: "Open live view" }));
  expect(await screen.findByText("This scan has no Browserbase browser to show right now.")).toBeInTheDocument();
  openLiveView.mockRestore();
});

test("a manual visit on this Mac has no live view to open", async () => {
  await renderWithApp(view(running({ ...baseOptions, visitor: "manual", extraPages: 0 }), browseLogs.slice(0, 1)));
  const bar = screen.getByRole("region", { name: "Manual session" });
  expect(within(bar).queryByRole("button", { name: "Open live view" })).toBeNull();
  expect(bar).toHaveTextContent("Use the Chrome window that opened.");
});

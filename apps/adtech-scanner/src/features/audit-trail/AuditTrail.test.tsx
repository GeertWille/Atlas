import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import { makeLog, makeStep } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { AuditTrail } from "./AuditTrail";

const steps = [
  makeStep({ id: "step-capture", node: "capture", summary: "36 scripts, 1 auction" }),
  makeStep({
    id: "step-analyze",
    node: "analyzeLogic",
    model: "claude-sonnet-5-5",
    inputTokens: 144_000,
    outputTokens: 6300,
    costUsd: 0.351,
    durationMs: 21_400,
  }),
];

const logs = [
  makeLog({ id: "l1", stepId: "step-capture", node: "capture", message: "Navigating to the page" }),
  makeLog({
    id: "l2",
    stepId: "step-capture",
    node: "capture",
    level: "warn",
    message: "Consent banner left untouched",
  }),
  makeLog({
    id: "l3",
    stepId: "step-analyze",
    node: "analyzeLogic",
    kind: "thinking",
    message: "The lazy units never auction",
  }),
  makeLog({
    id: "l4",
    stepId: "step-analyze",
    node: "analyzeLogic",
    kind: "toolCall",
    message: "read_script",
    data: { url: "hb-wrapper.js", startLine: 205 },
  }),
  makeLog({
    id: "l5",
    stepId: "step-analyze",
    node: "analyzeLogic",
    kind: "toolResult",
    message: "read_script returned 2 lines",
    data: { output: "var IS_MOBILE = true;\nrequestBids(lazy);" },
  }),
];

const entries = () => within(screen.getByRole("region", { name: "Log entries" })).queryAllByRole("time").length;

test("shows every entry with per-step counts, cost and tokens", async () => {
  await renderWithApp(<AuditTrail steps={steps} logs={logs} onOpenFinding={() => {}} />);

  expect(entries()).toBe(5);
  expect(screen.getByText("5 entries")).toBeInTheDocument();
  const nav = screen.getByRole("navigation", { name: "Pipeline steps" });
  expect(within(nav).getByRole("button", { name: /All steps/ })).toHaveAttribute("aria-current", "true");
  expect(within(nav).getByRole("button", { name: /Capture page/ })).toHaveTextContent("36 scripts, 1 auction");
  expect(within(nav).getByRole("button", { name: /Analyse logic/ })).toHaveTextContent(
    "21.4 s · 150.3K tokens · $0.35",
  );
});

test("selecting a step, searching and filtering by kind narrow the list", async () => {
  const user = userEvent.setup();
  await renderWithApp(<AuditTrail steps={steps} logs={logs} onOpenFinding={() => {}} />);

  await user.click(screen.getByRole("button", { name: /Analyse logic/ }));
  expect(entries()).toBe(3);
  expect(screen.getByText("3 of 5 entries")).toBeInTheDocument();

  await user.type(screen.getByRole("searchbox", { name: "Search this audit trail" }), "is_mobile");
  expect(entries()).toBe(1);
  expect(screen.getByText("read_script returned 2 lines")).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Clear search" }));
  await user.click(screen.getByRole("button", { name: "Filter by kind" }));
  await user.click(await screen.findByRole("menuitemcheckbox", { name: /^Thinking/ }));
  await user.keyboard("{Escape}");
  expect(entries()).toBe(1);
  expect(screen.getByText("The lazy units never auction")).toBeInTheDocument();
});

test("level filter and the no-match state with a way back", async () => {
  const user = userEvent.setup();
  await renderWithApp(<AuditTrail steps={steps} logs={logs} onOpenFinding={() => {}} />);

  await user.click(screen.getByRole("button", { name: "Filter by level" }));
  await user.click(await screen.findByRole("menuitemcheckbox", { name: /^Warning/ }));
  await user.keyboard("{Escape}");
  expect(entries()).toBe(1);

  await user.click(screen.getByRole("button", { name: /Analyse logic/ }));
  expect(screen.getByText("No entries match these filters")).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Clear filters" }));
  expect(entries()).toBe(5);
  expect(screen.getByRole("button", { name: /All steps/ })).toHaveAttribute("aria-current", "true");
});

test("expanding an entry reveals its tool output and structured data", async () => {
  const user = userEvent.setup();
  await renderWithApp(<AuditTrail steps={steps} logs={logs} onOpenFinding={() => {}} />);

  expect(screen.queryByText(/requestBids\(lazy\)/)).toBeNull();
  await user.click(screen.getByRole("button", { name: /read_script returned 2 lines/, expanded: false }));
  expect(screen.getByText(/requestBids\(lazy\)/)).toBeInTheDocument();

  expect(screen.getByText('(url: "hb-wrapper.js", startLine: 205)')).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: /read_script\(url/, expanded: false }));
  expect(screen.getByText('"startLine"', { exact: false })).toBeInTheDocument();
});

test("a deep-linked entry is highlighted and already expanded", async () => {
  await renderWithApp(<AuditTrail steps={steps} logs={logs} focusedLogId="l5" onOpenFinding={() => {}} />);
  expect(screen.getByRole("button", { name: /read_script returned 2 lines/, expanded: true })).toBeInTheDocument();
  expect(screen.getByText(/requestBids\(lazy\)/)).toBeInTheDocument();
});

test("a long trail mounts only the rows near the viewport and stays searchable", async () => {
  const user = userEvent.setup();
  const long = Array.from({ length: 600 }, (_, index) =>
    makeLog({ id: `m${index}`, stepId: "step-capture", node: "capture", ts: index, message: `Request ${index} seen` }),
  );
  await renderWithApp(<AuditTrail steps={steps} logs={long} onOpenFinding={() => {}} />);

  expect(screen.getByText("600 entries")).toBeInTheDocument();
  expect(entries()).toBeGreaterThan(10);
  expect(entries()).toBeLessThan(60);
  expect(screen.queryByText("Request 571 seen")).toBeNull();

  await user.type(screen.getByRole("searchbox", { name: "Search this audit trail" }), "request 571");
  expect(screen.getByText("Request 571 seen")).toBeInTheDocument();
  expect(entries()).toBe(1);
});

test("loading and empty trails have their own states", async () => {
  const loading = await renderWithApp(<AuditTrail steps={steps} logs={undefined} onOpenFinding={() => {}} />);
  expect(entries()).toBe(0);
  expect(screen.queryByText("No log entries")).toBeNull();
  loading.unmount();

  await renderWithApp(<AuditTrail steps={steps} logs={[]} onOpenFinding={() => {}} />);
  expect(screen.getByText("No log entries")).toBeInTheDocument();
});

test("a finding entry links to the finding it reported", async () => {
  const user = userEvent.setup();
  const onOpenFinding = vi.fn();
  const finding = makeLog({
    id: "l6",
    stepId: "step-analyze",
    node: "analyzeLogic",
    kind: "finding",
    level: "warn",
    message: "Desktop is treated as mobile",
    data: { findingId: "scan-1-f5", severity: "critical" },
  });
  await renderWithApp(<AuditTrail steps={steps} logs={[...logs, finding]} onOpenFinding={onOpenFinding} />);

  await user.click(screen.getByRole("button", { name: /Desktop is treated as mobile/, expanded: false }));
  await user.click(screen.getByRole("button", { name: "Open finding" }));
  expect(onOpenFinding).toHaveBeenCalledWith("scan-1-f5");
});

const triageStep = makeStep({ id: "step-filter", node: "filterScripts", model: "jev-1.14.0" });
const decision = makeLog({
  id: "c1",
  stepId: "step-filter",
  node: "filterScripts",
  kind: "classification",
  message:
    "s7 https://www.example.com/ads/lazy.js: ad orchestration 0.93 (analyse >= 0.50, drop < 0.15) over 3 of 12 parts -> ad-related, Phase 2 candidate [jev-yes]",
  data: {
    scriptId: "s7",
    classifier: "jev",
    model: "jev-1.14.0",
    answers: [
      { questionId: "ad_orchestration", pYes: 0.93 },
      { questionId: "publisher_specific", pYes: 0.4 },
    ],
    thresholds: { analyse: 0.5, drop: 0.15, publisherSpecific: 0.5 },
    decision: { adRelated: true, category: "wrapper", route: "jev-yes" },
    parts: { total: 12, chosen: 3 },
    excerpt: "pbjs.requestBids({ timeout: PREBID_TIMEOUT });",
  },
});

test("a script kept only because it could not be ruled out is marked unsure, not ad-related", async () => {
  const unsure = makeLog({
    ...decision,
    id: "l-unsure",
    message:
      "s9 https://www.example.com/js/tags.js: ad orchestration 0.19 -> kept for analysis, ranked last [jev-unsure]",
    data: {
      answers: [{ questionId: "ad_orchestration", pYes: 0.19 }],
      thresholds: { analyse: 0.5, drop: 0.15 },
      decision: { adRelated: true, category: "unknown", route: "jev-unsure" },
    },
  });
  await renderWithApp(<AuditTrail steps={[triageStep]} logs={[...logs, decision, unsure]} onOpenFinding={() => {}} />);

  const row = screen.getByRole("button", { name: /tags\.js/ });
  expect(row).toHaveTextContent(/Unsure, kepts9 /);
  expect(row).not.toHaveTextContent("Ad-related");
  expect(screen.getByRole("button", { name: /lazy\.js/ })).toHaveTextContent(/Ad-related/);
});

test("a classification entry shows the probabilities, thresholds, routing decision and excerpt", async () => {
  const user = userEvent.setup();
  await renderWithApp(<AuditTrail steps={[triageStep]} logs={[...logs, decision]} onOpenFinding={() => {}} />);

  expect(screen.getByRole("img", { name: "Classification, Info" })).toBeInTheDocument();
  // The verdict is readable without opening the row.
  const row = screen.getByRole("button", { name: /lazy\.js/, expanded: false });
  expect(row).toHaveTextContent(/Ad-related.*s7 https:\/\/www\.example\.com\/ads\/lazy\.js/);
  await user.click(row);

  const answers = screen
    .getAllByRole("term")
    .map((term) => [term.textContent, term.parentElement?.querySelector("dd:last-child")?.textContent]);
  expect(answers).toEqual([
    ["ad_orchestration", "0.93"],
    ["publisher_specific", "0.40"],
  ]);
  expect(screen.getByText("Wrapper")).toBeInTheDocument();
  expect(screen.getByText("jev-yes", { selector: "span" })).toBeInTheDocument();
  expect(screen.getByText("Thresholds analyse 0.50, drop 0.15, publisherSpecific 0.50")).toBeInTheDocument();
  // Each bar carries the cut-offs its question was held against.
  expect(
    screen.getAllByRole("term").map((term) => term.parentElement?.querySelectorAll("[data-threshold]").length),
  ).toEqual([2, 1]);
  expect(screen.getByText("pbjs.requestBids({ timeout: PREBID_TIMEOUT });")).toBeInTheDocument();
  // The raw data stays available underneath, without repeating the excerpt.
  expect(screen.getByText('"scriptId"', { exact: false })).toBeInTheDocument();
  expect(screen.queryByText('"excerpt"', { exact: false })).toBeNull();

  await user.click(screen.getByRole("button", { name: "Filter by kind" }));
  await user.click(await screen.findByRole("menuitemcheckbox", { name: /^Classification/ }));
  await user.keyboard("{Escape}");
  expect(entries()).toBe(1);
});

test("a classification entry in a shape this version does not know still shows its message and raw data", async () => {
  const user = userEvent.setup();
  const unfamiliar = makeLog({
    id: "c2",
    stepId: "step-filter",
    node: "filterScripts",
    kind: "classification",
    message: "s9 app.js -> DROP by jev-no",
    data: { script: "s9", scores: { ad: 0.04 }, verdict: "drop" },
  });
  await renderWithApp(<AuditTrail steps={[triageStep]} logs={[unfamiliar]} onOpenFinding={() => {}} />);

  await user.click(screen.getByRole("button", { name: /s9 app\.js -> DROP by jev-no/, expanded: false }));
  expect(screen.queryByRole("term")).toBeNull();
  expect(screen.getByText('"verdict"', { exact: false })).toBeInTheDocument();
});

test("a fallback warning stands out and is found by the level filter", async () => {
  const user = userEvent.setup();
  const fallback = makeLog({
    id: "w1",
    stepId: "step-filter",
    node: "filterScripts",
    level: "warn",
    message: "Jev failed (HTTP 429); 9 scripts fell back to claude-haiku-5-5",
    data: { from: "jev-1.14.0", to: "claude-haiku-5-5", reason: "HTTP 429", remainingScripts: 9 },
  });
  await renderWithApp(<AuditTrail steps={[triageStep]} logs={[decision, fallback]} onOpenFinding={() => {}} />);

  expect(screen.getByText(/Jev failed/)).toHaveClass("text-amber-fg");
  await user.click(screen.getByRole("button", { name: "Filter by level" }));
  await user.click(await screen.findByRole("menuitemcheckbox", { name: /^Warning/ }));
  await user.keyboard("{Escape}");
  expect(entries()).toBe(1);
  expect(screen.getByText(/Jev failed/)).toBeInTheDocument();
});

import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import { makeFinding } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { FindingsTab } from "./FindingsTab";

const findings = [
  makeFinding({ id: "f-low", severity: "low", category: "hygiene", source: "heuristic", title: "Prebid is outdated" }),
  makeFinding({
    id: "f-crit",
    severity: "critical",
    category: "logicBug",
    source: "ai",
    title: "Desktop is treated as mobile",
    description: "The wrapper checks `screen.width`.",
    estimatedRevenueImpactPct: 34,
    evidence: [
      {
        kind: "code",
        label: "Device check",
        body: "var IS_MOBILE = screen.width < 1440;",
        scriptUrl: "https://x.example/ads/hb-wrapper.js",
        line: 211,
        endLine: null,
        scriptId: null,
        requestId: null,
      },
    ],
    affectedSlots: ["div-gpt-sidebar-1"],
  }),
  makeFinding({ id: "f-high", severity: "high", category: "timing", source: "heuristic", title: "Bidders time out" }),
];

const titles = () => screen.getAllByRole("article").map((card) => card.getAttribute("aria-label"));

async function chooseFilter(user: ReturnType<typeof userEvent.setup>, menu: string, option: string) {
  // Once something is chosen the button also says how many: "Filter by severity 1 selected".
  await user.click(screen.getByRole("button", { name: new RegExp(`^Filter by ${menu}( \\d+ selected)?$`) }));
  await user.click(await screen.findByRole("menuitemcheckbox", { name: new RegExp(`^${option}`) }));
  await user.keyboard("{Escape}");
}

test("lists findings most severe first with the top one expanded", async () => {
  await renderWithApp(<FindingsTab findings={findings} onOpenScript={() => {}} />);

  expect(titles()).toEqual(["Desktop is treated as mobile", "Bidders time out", "Prebid is outdated"]);
  expect(screen.getByText("3 findings")).toBeInTheDocument();
  const top = screen.getByRole("article", { name: "Desktop is treated as mobile" });
  expect(within(top).getByText("Recommendation")).toBeInTheDocument();
  expect(within(top).getByText("211")).toBeInTheDocument();
  expect(within(top).getByText("div-gpt-sidebar-1")).toBeInTheDocument();
  expect(within(screen.getByRole("article", { name: "Bidders time out" })).queryByText("Recommendation")).toBeNull();
});

test("filters by severity, category and source, and reports the count", async () => {
  const user = userEvent.setup();
  await renderWithApp(<FindingsTab findings={findings} onOpenScript={() => {}} />);

  await chooseFilter(user, "severity", "High");
  expect(titles()).toEqual(["Bidders time out"]);
  expect(screen.getByText("1 of 3 findings")).toBeInTheDocument();

  await chooseFilter(user, "severity", "Critical");
  expect(titles()).toEqual(["Desktop is treated as mobile", "Bidders time out"]);

  await user.click(screen.getByRole("radio", { name: "Rules" }));
  expect(titles()).toEqual(["Bidders time out"]);

  await chooseFilter(user, "category", "Logic bug");
  expect(screen.queryByRole("article")).toBeNull();
  expect(screen.getByText("No findings match these filters")).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Clear filters" }));
  expect(titles()).toHaveLength(3);
  expect(screen.getByRole("radio", { name: "All sources" })).toBeChecked();
});

test("the severity menu only offers severities that occur", async () => {
  const user = userEvent.setup();
  await renderWithApp(<FindingsTab findings={findings} onOpenScript={() => {}} />);

  await user.click(screen.getByRole("button", { name: "Filter by severity" }));
  const options = (await screen.findAllByRole("menuitemcheckbox")).map((item) => item.textContent);
  expect(options).toEqual(["Critical1", "High1", "Low1"]);
});

test("cards expand and collapse, and evidence links to its script", async () => {
  const user = userEvent.setup();
  const onOpenScript = vi.fn();
  await renderWithApp(<FindingsTab findings={findings} onOpenScript={onOpenScript} />);

  const card = screen.getByRole("article", { name: "Bidders time out" });
  const toggle = within(card).getByRole("button", { expanded: false });
  await user.click(toggle);
  expect(within(card).getByText("How to fix it.")).toBeInTheDocument();
  await user.click(within(card).getByRole("button", { expanded: true }));
  expect(within(card).queryByText("How to fix it.")).toBeNull();

  await user.click(screen.getByRole("button", { name: /hb-wrapper\.js:211/ }));
  expect(onOpenScript).toHaveBeenCalledWith("https://x.example/ads/hb-wrapper.js");
});

test("a deep-linked finding is expanded even when it is not the most severe", async () => {
  await renderWithApp(<FindingsTab findings={findings} focusedFindingId="f-low" onOpenScript={() => {}} />);
  const card = screen.getByRole("article", { name: "Prebid is outdated" });
  expect(within(card).getByText("Recommendation")).toBeInTheDocument();
});

test("copies a finding as Markdown", async () => {
  const user = userEvent.setup();
  await renderWithApp(<FindingsTab findings={findings} onOpenScript={() => {}} />);
  const card = screen.getByRole("article", { name: "Bidders time out" });
  await user.click(within(card).getByRole("button", { name: "Copy finding as Markdown" }));
  expect(await navigator.clipboard.readText()).toContain("## Bidders time out\n\n*High · Timing · Deterministic rule");
});

test("no findings shows an empty state instead of filters", async () => {
  await renderWithApp(<FindingsTab findings={[]} onOpenScript={() => {}} />);
  expect(screen.getByText("No findings")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Filter by severity" })).toBeNull();
});

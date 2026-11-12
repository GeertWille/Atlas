import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import { makeLog, makeStep } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { AuditTrail } from "./AuditTrail";

const steps = [makeStep({ id: "step-capture", node: "capture" })];
const visitor = (id: string, message: string, activity: string, detail: unknown, level: "info" | "warn" = "info") =>
  makeLog({ id, stepId: "step-capture", kind: "visitor", level, message, data: { activity, page: 0, detail } });

const logs = [
  makeLog({ id: "l1", stepId: "step-capture", message: "Navigating to the page" }),
  visitor("l2", "Page 1: / (entry page)", "pageView", { index: 0, url: "https://site.example/", reachedBy: "entry" }),
  visitor("l3", "Scrolled to 60%", "action", { action: "scroll", scrollDepth: 0.6 }),
  visitor("l4", "Following link: /sport/final (best of 3)", "linkChoice", {
    chosen: {
      text: "Cup final preview",
      href: "https://site.example/sport/final",
      score: 6,
      reasons: ["content link", "article-like path"],
    },
    alternatives: [
      { text: "Rail strike called off", href: "https://site.example/news/rail-strike", score: 5 },
      { text: "", href: "https://site.example/sport", score: 2 },
    ],
    skipped: { "off-site": 9, "inside-ad": 3, sensitive: 2 },
  }),
  visitor(
    "l5",
    "Left the scanned site (partner.example): not recorded",
    "blocked",
    { reason: "off-site", host: "partner.example" },
    "warn",
  ),
  visitor("l6", "No link to follow", "linkChoice", { chosen: null, alternatives: [], skipped: { hidden: 4 } }),
];

const entries = () => screen.getByRole("region", { name: "Log entries" });

test("visitor entries have their own kind, icon and filter", async () => {
  const user = userEvent.setup();
  await renderWithApp(<AuditTrail steps={steps} logs={logs} onOpenFinding={() => {}} />);
  expect(within(entries()).getAllByRole("img", { name: "Visitor, Info" })).toHaveLength(4);
  expect(within(entries()).getByRole("img", { name: "Visitor, Warning" })).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Filter by kind" }));
  const option = await screen.findByRole("menuitemcheckbox", { name: /^Visitor/ });
  expect(option).toHaveTextContent("5");
  await user.click(option);
  await user.keyboard("{Escape}");
  expect(screen.getByText("5 of 6 entries")).toBeInTheDocument();
  expect(within(entries()).queryByText("Navigating to the page")).toBeNull();
});

test("a link choice opens into the link followed, why, the runners-up and what was not considered", async () => {
  const user = userEvent.setup();
  await renderWithApp(<AuditTrail steps={steps} logs={logs} onOpenFinding={() => {}} />);
  await user.click(within(entries()).getByRole("button", { name: /Following link: \/sport\/final/ }));

  const links = within(within(entries()).getByRole("list", { name: "Links considered" })).getAllByRole("listitem");
  expect(links.map((link) => link.textContent)).toEqual([
    "FollowedCup final previewscore 6https://site.example/sport/final",
    "Rail strike called offscore 5https://site.example/news/rail-strike",
    "(no text)score 2https://site.example/sport",
  ]);
  expect(within(entries()).getByText("Chosen for: content link, article-like path")).toBeInTheDocument();
  expect(within(entries()).getByText("Not considered:")).toHaveTextContent(
    "Not considered: 9 off-site, 3 inside ads, 2 sign-in, checkout and the like",
  );
  // The raw data stays available below the summary.
  expect(within(entries()).getByText(/"activity"/)).toBeInTheDocument();
});

test("a page without a link to follow says so instead of showing an empty list", async () => {
  const user = userEvent.setup();
  await renderWithApp(<AuditTrail steps={steps} logs={logs} onOpenFinding={() => {}} />);
  await user.click(within(entries()).getByRole("button", { name: /No link to follow/ }));
  expect(within(entries()).queryByRole("list", { name: "Links considered" })).toBeNull();
  expect(within(entries()).getByText("No link on this page was safe and worth following.")).toBeInTheDocument();
  expect(within(entries()).getByText("Not considered:")).toHaveTextContent("Not considered: 4 hidden");
});

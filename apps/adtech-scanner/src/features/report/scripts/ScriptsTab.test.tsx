import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import { makeScript } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { ScriptsTab } from "./ScriptsTab";

const scripts = [
  makeScript({
    id: "s1",
    url: "https://www.example.com/ads/hb-wrapper.min.js",
    category: "wrapper",
    adRelated: true,
    classifiedBy: "ai",
    classifierModel: "jev-1.14.0",
    adProbability: 0.93,
    classificationConfidence: 0.93,
    classificationReason: "Jev: ad-related probability 0.93, at or above 0.50 (rule jev-yes).",
    analyzed: true,
  }),
  makeScript({
    id: "s2",
    url: "https://www.example.com/static/main.js",
    classifiedBy: "ai",
    classifierModel: "jev-1.14.0",
    adProbability: 0.04,
    classificationConfidence: 0.96,
  }),
  makeScript({
    id: "s3",
    url: "https://www.example.com/static/paywall.js",
    classifiedBy: "ai",
    classifierModel: "claude-haiku-5-5",
    classificationConfidence: 0.9,
  }),
  makeScript({
    id: "s4",
    url: "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
    hostname: "securepubads.g.doubleclick.net",
    category: "adServer",
    adRelated: true,
    classifiedBy: "heuristic",
    classifierModel: null,
    classificationConfidence: 1,
  }),
  // Stored before the classifier model was recorded.
  makeScript({ id: "s5", url: "https://www.example.com/static/legacy.js", classifierModel: null }),
];

const classifiedBy = (name: RegExp) => within(screen.getByRole("row", { name })).getAllByRole("cell").at(-1);

test("each row says who classified the script: Jev with its ad probability, Claude or a rule with a confidence", async () => {
  await renderWithApp(<ScriptsTab scripts={scripts} />);

  expect(classifiedBy(/hb-wrapper\.min\.js/)).toHaveTextContent("Jev · 93% ad");
  expect(classifiedBy(/main\.js/)).toHaveTextContent("Jev · 4% ad");
  expect(classifiedBy(/paywall\.js/)).toHaveTextContent("AI · 90%");
  expect(classifiedBy(/gpt\.js/)).toHaveTextContent("Rule · 100%");
  expect(classifiedBy(/legacy\.js/)).toHaveTextContent("AI · 90%");
});

test("script details name the classifier model and show Jev's ad probability instead of a confidence", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScriptsTab scripts={scripts} />);
  const fact = (label: string) =>
    within(screen.getByRole("region", { name: "Script details" })).getByText(label).nextElementSibling;

  await user.click(screen.getByRole("row", { name: /main\.js/ }));
  expect(fact("Classified by")).toHaveTextContent("Jev System One");
  expect(fact("Ad probability")).toHaveTextContent("4%");
  expect(screen.queryByText("Confidence")).toBeNull();

  await user.click(screen.getByRole("row", { name: /paywall\.js/ }));
  expect(fact("Classified by")).toHaveTextContent("Claude Haiku 5.5");
  expect(fact("Confidence")).toHaveTextContent("90%");
  expect(screen.queryByText("Ad probability")).toBeNull();

  await user.click(screen.getByRole("row", { name: /gpt\.js/ }));
  expect(fact("Classified by")).toHaveTextContent("Deterministic rule");

  await user.click(screen.getByRole("row", { name: /legacy\.js/ }));
  expect(fact("Classified by")).toHaveTextContent("AI analysis");
});

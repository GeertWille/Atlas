import { screen, within } from "@testing-library/react";
import { expect, test } from "vitest";
import { makeFinding, makePage } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { FindingsTab } from "./FindingsTab";
import { findingPages, findingToMarkdown } from "./finding-markdown";

const pages = [
  makePage(),
  makePage({ index: 1, url: "https://www.example.com/climate/kelp-carbon" }),
  makePage({ index: 2, url: "https://www.example.com/sport/final?ref=home" }),
];
const later = makeFinding({ id: "later", severity: "high", title: "Later pages do not bid", pages: [1, 2] });
const entry = makeFinding({ id: "entry", severity: "medium", title: "Slow bidder", pages: [0] });
const everywhere = makeFinding({ id: "all", severity: "low", title: "Old Prebid", pages: [] });

test("a finding is placed on the page views it names, in visit order, and only in a multi-page visit", () => {
  expect(findingPages(later, pages).map((page) => page.index)).toEqual([1, 2]);
  expect(findingPages(everywhere, pages)).toEqual([]);
  expect(findingPages(entry, [makePage()])).toEqual([]);
  expect(findingPages(entry, [])).toEqual([]);
});

test("a multi-page visit shows page chips on the card and the pages in full when it is open", async () => {
  await renderWithApp(<FindingsTab findings={[later, entry, everywhere]} pages={pages} onOpenScript={() => {}} />);

  const laterCard = screen.getByRole("article", { name: "Later pages do not bid" });
  expect(within(laterCard).getByText("Pages 2, 3")).toBeInTheDocument();
  // The most severe finding starts expanded.
  expect(within(laterCard).getByText("Seen on")).toBeInTheDocument();
  expect(within(laterCard).getByText("2 /climate/kelp-carbon")).toBeInTheDocument();
  expect(within(laterCard).getByText("3 /sport/final?ref=home")).toBeInTheDocument();

  expect(within(screen.getByRole("article", { name: "Slow bidder" })).getByText("Page 1")).toBeInTheDocument();
  // A finding about the whole visit names no page.
  expect(within(screen.getByRole("article", { name: "Old Prebid" })).queryByText(/^Pages? \d/)).toBeNull();
});

test("a single-page scan shows no page chips at all", async () => {
  await renderWithApp(<FindingsTab findings={[later, entry]} pages={[makePage()]} onOpenScript={() => {}} />);
  expect(screen.queryByText(/^Pages? \d/)).toBeNull();
  expect(screen.queryByText("Seen on")).toBeNull();
});

test("the copied markdown says where a finding was seen, and stays as it was for a single page", () => {
  expect(findingToMarkdown(later, pages)).toMatch(
    /Seen on: page 2 \(\/climate\/kelp-carbon\), page 3 \(\/sport\/final\?ref=home\)$/,
  );
  expect(findingToMarkdown(everywhere, pages)).not.toContain("Seen on");
  expect(findingToMarkdown(later, [makePage()])).toBe(findingToMarkdown(later));
  expect(findingToMarkdown(later)).not.toContain("Seen on");
});

import { expect, test } from "vitest";
import { makeConfiguration, makeProxy, settingsFixture } from "@/test/fixtures";
import { groupBySite, searchConfigurations, siteOf, sortConfigurations } from "./configuration-list";

const base = makeConfiguration().options;
const configuration = (
  name: string,
  url: string,
  times: { lastRunAt?: number | null; updatedAt?: number } = {},
  options: Partial<typeof base> = {},
) =>
  makeConfiguration({
    id: name,
    name,
    options: { ...base, url, ...options },
    lastRunAt: times.lastRunAt ?? null,
    updatedAt: times.updatedAt ?? 0,
  });

const names = (configurations: { name: string }[]) => configurations.map((item) => item.name);

test("the site is the hostname without www", () => {
  expect(siteOf(configuration("a", "https://www.example.com/news"))).toBe("example.com");
  expect(siteOf(configuration("a", "https://m.example.com/"))).toBe("m.example.com");
});

test("recently run sorts by the last run, and a configuration that never ran by its last change", () => {
  const sorted = sortConfigurations(
    [
      configuration("ran long ago", "https://a.example/", { lastRunAt: 100, updatedAt: 900 }),
      configuration("never ran, changed lately", "https://a.example/", { updatedAt: 500 }),
      configuration("ran just now", "https://a.example/", { lastRunAt: 800 }),
    ],
    "recent",
  );
  expect(names(sorted)).toEqual(["ran just now", "never ran, changed lately", "ran long ago"]);
});

test("by name sorts without case and with numbers in order", () => {
  const sorted = sortConfigurations(
    [
      configuration("site 10", "https://a.example/"),
      configuration("Site 2", "https://a.example/"),
      configuration("alpha", "https://a.example/"),
    ],
    "name",
  );
  expect(names(sorted)).toEqual(["alpha", "Site 2", "site 10"]);
});

test("groups keep the sorted order: sites by their first configuration, rows as sorted", () => {
  const groups = groupBySite([
    configuration("B1", "https://www.b.example/x"),
    configuration("A1", "https://a.example/"),
    configuration("B2", "https://b.example/y"),
  ]);
  expect(groups.map((group) => [group.site, names(group.configurations)])).toEqual([
    ["b.example", ["B1", "B2"]],
    ["a.example", ["A1"]],
  ]);
});

test("search matches every word against name, address, device, proxy and visit", () => {
  const belgium = makeProxy({ id: "proxy-be", name: "Belgium" });
  const settings = { ...settingsFixture, proxies: [belgium] };
  const all = [
    configuration("Sports front page", "https://sportsdesk.example/", {}, { device: "mobile" }),
    configuration("Ledger", "https://ledger.example/markets", {}, { proxy: { kind: "profile", id: "proxy-be" } }),
    configuration("Ledger browse", "https://ledger.example/", {}, { visitor: "browse", extraPages: 2 }),
  ];
  const found = (search: string) => names(searchConfigurations(all, search, settings));
  expect(found("")).toEqual(["Sports front page", "Ledger", "Ledger browse"]);
  expect(found("  LEDGER ")).toEqual(["Ledger", "Ledger browse"]);
  expect(found("markets")).toEqual(["Ledger"]);
  expect(found("mobile")).toEqual(["Sports front page"]);
  expect(found("belgium ledger")).toEqual(["Ledger"]);
  expect(found("3 pages")).toEqual(["Ledger browse"]);
  expect(found("ledger mobile")).toEqual([]);
});

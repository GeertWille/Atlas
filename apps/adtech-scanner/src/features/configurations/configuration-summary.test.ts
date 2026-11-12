import { expect, test } from "vitest";
import { makeConfiguration, makeProxy } from "@/test/fixtures";
import {
  configurationProblem,
  duplicateName,
  healthChange,
  proxySummary,
  visitorSummary,
} from "./configuration-summary";

const settings = { proxies: [makeProxy()] };
const withProxy = (proxy: NonNullable<ReturnType<typeof makeConfiguration>["options"]["proxy"]>) =>
  makeConfiguration({ options: { ...makeConfiguration().options, proxy } });

test("a configuration can run unless the saved proxy it names is gone", () => {
  expect(configurationProblem(makeConfiguration(), settings)).toBeNull();
  expect(configurationProblem(withProxy({ kind: "none" }), settings)).toBeNull();
  expect(configurationProblem(withProxy({ kind: "profile", id: "proxy-be" }), settings)).toBeNull();
  expect(configurationProblem(withProxy({ kind: "profile", id: "proxy-de", name: "Germany" }), settings)).toBe(
    "The proxy “Germany” was deleted. Edit the configuration and choose another proxy.",
  );
  expect(configurationProblem(withProxy({ kind: "profile", id: "proxy-de" }), settings)).toBe(
    "The proxy it used was deleted. Edit the configuration and choose another proxy.",
  );
});

test("the proxy is named as a run would use it, and a deleted one is marked", () => {
  expect(proxySummary(makeConfiguration(), settings)).toBe("Settings default");
  expect(proxySummary(withProxy({ kind: "system" }), settings)).toBe("System proxy");
  // The live name wins over the one stored with the configuration.
  expect(proxySummary(withProxy({ kind: "profile", id: "proxy-be", name: "Old name" }), settings)).toBe("Belgium");
  expect(proxySummary(withProxy({ kind: "profile", id: "proxy-de", name: "Germany" }), settings)).toBe(
    "Germany (deleted)",
  );
});

test("the visitor reads with its page count for a browse visit", () => {
  const visit = (visitor: "scroll" | "browse" | "manual", extraPages?: number) =>
    visitorSummary(makeConfiguration({ options: { ...makeConfiguration().options, visitor, extraPages } }));
  expect(visit("scroll", 0)).toBe("Scroll");
  expect(visit("browse", 2)).toBe("Browse like a visitor, 3 pages");
  expect(visit("browse", 0)).toBe("Browse like a visitor, 1 page");
  // Left to the settings: no number to state.
  expect(visit("browse")).toBe("Browse like a visitor");
  expect(visit("manual", 0)).toBe("I'll browse myself");
});

test("a duplicate gets the first free copy name, compared without case", () => {
  expect(duplicateName("Sportsdesk", ["Sportsdesk"])).toBe("Sportsdesk copy");
  expect(duplicateName("Sportsdesk", ["Sportsdesk", "sportsdesk COPY", "Sportsdesk copy 2"])).toBe("Sportsdesk copy 3");
});

test("the health change compares the two newest runs, and only when both completed with a score", () => {
  const run = (healthScore: number | null, status: "completed" | "failed" | "running" = "completed") => ({
    healthScore,
    status,
  });
  expect(healthChange([run(84), run(72)])).toBe(12);
  expect(healthChange([run(40), run(72)])).toBe(-32);
  expect(healthChange([run(72), run(72)])).toBe(0);
  expect(healthChange([run(84)])).toBeNull();
  expect(healthChange([])).toBeNull();
  expect(healthChange([run(null, "running"), run(72)])).toBeNull();
  expect(healthChange([run(84), run(null, "failed")])).toBeNull();
});

import AxeBuilder from "@axe-core/playwright";
import { test } from "@playwright/test";
test("axe", async ({ page }) => {
  await page.goto("http://localhost:1420/?fakeProfile=seeded&fakeSpeed=40#/scans/seed-sportsdesk-browse?tab=audit");
  await page.getByRole("button", { name: "Filter by kind" }).click();
  await page.getByRole("menuitemcheckbox", { name: /^Visitor/ }).click();
  await page.keyboard.press("Escape");
  const results = await new AxeBuilder({ page }).withRules(["label-content-name-mismatch"]).analyze();
  for (const v of results.violations) for (const n of v.nodes) console.log(n.html, JSON.stringify(n.any.concat(n.all, n.none).map((c) => [c.id, c.message, c.data])));
  console.log("violations", results.violations.length);
});

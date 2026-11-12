import { execFileSync } from "node:child_process";
import { afterAll, beforeAll, test } from "vitest";
import { type App, button, launchApp, seeText } from "./support/app";

let app: App;
const OUT = "/tmp/claude-0/-home-user/ee93eb55-ef69-5991-bc2c-57c8a322dcdf/scratchpad/explore";
beforeAll(async () => { app = await launchApp({ ADSCAN_ANTHROPIC_BASE_URL: "http://127.0.0.1:9" }); });
afterAll(async () => { await app?.close(); });
const x = (...a: string[]) => { try { return execFileSync("xdotool", a, { encoding: "utf8" }).trim(); } catch (e) { return "ERR " + String(e).slice(0, 200); } };
const wins = () => x("search", "--onlyvisible", "--class", "adtech-scanner").split("\n");
const snap = (n: string) => execFileSync("import", ["-window", "root", `${OUT}/dlg-${n}.png`]);

test("dialog", async () => {
  const { browser } = app;
  await browser.$('//nav//a[normalize-space(.)="Settings"]').click();
  await seeText(browser, "(detected automatically)");
  const before = wins();
  console.log("before", before, x("getwindowfocus"), x("getmouselocation"));
  await browser.$(button("Choose executable")).click();
  await browser.waitUntil(async () => wins().length > before.length, { timeout: 15000 });
  const dialog = wins().filter((w) => !before.includes(w));
  console.log("dialog", dialog, dialog.map((d) => x("getwindowname", d) + " " + x("getwindowgeometry", d)), "focus", x("getwindowfocus"));
  await browser.pause(300);
  snap("1-open");
  x("key", "ctrl+l");
  await browser.pause(300);
  snap("2-ctrl-l");
  x("type", "--delay", "20", "/opt/pw-browsers/chromium-1243/chrome-linux64/chrome");
  await browser.pause(300);
  snap("3-typed");
  x("key", "Return");
  await browser.pause(1500);
  snap("4-return");
  console.log("after", wins(), x("getwindowfocus"));
});

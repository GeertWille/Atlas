import { appendFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Browser } from "./app";
export const OUT = "/tmp/claude-0/-home-user/ee93eb55-ef69-5991-bc2c-57c8a322dcdf/scratchpad/real";
export const LOG = join(OUT, "log.txt");
export function note(line: string) { appendFileSync(LOG, `${line}\n`); }
export async function shot(browser: Browser, name: string) { await browser.saveScreenshot(join(OUT, "shots", `${name}.png`)); }
export async function bodyText(browser: Browser): Promise<string> {
  return browser.execute(() => (document.querySelector("body") as HTMLElement).innerText);
}
export async function step(browser: Browser, name: string, fn: () => Promise<void>) {
  try { await fn(); note(`PASS ${name}`); }
  catch (error) {
    note(`FAIL ${name}: ${(error as Error).message.slice(0, 1500)}`);
    try { await shot(browser, `FAIL-${name.replace(/\W+/g, "-")}`); note(`  body: ${(await bodyText(browser)).slice(0, 3000).replace(/\n+/g, " | ")}`); } catch {}
  }
}
export const q = (t: string) => JSON.stringify(t);
export const sw = (label: string) => `//button[@role="switch"][@id=//label[normalize-space(.)=${q(label)}]/@for]`;
export const radio = (name: string) => `//button[@role="radio"][normalize-space(.)=${q(name)}]`;
export const menuitem = (name: string) => `//*[@role="menuitem" or @role="option"][starts-with(normalize-space(.), ${q(name)})]`;
export function reset() { writeFileSync(LOG, ""); }

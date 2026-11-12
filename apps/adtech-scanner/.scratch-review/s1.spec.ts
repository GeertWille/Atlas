import { afterAll, beforeAll, test } from "vitest";
import type { LogEntry, ScanConfiguration, ScanDetail, ScanList, ScanProfileInfo, Settings } from "../src/bindings";
import { type App, button, field, goTo, hashRoute, invoke, invokeOk, labelled, launchApp, reportTab, seeText, textOf } from "./app";
import { MOCK_API_KEY, type MockAnthropic, startMockAnthropic } from "../e2e-tauri/support/mock-anthropic";
import { bodyText, menuitem, note, q, radio, reset, shot, step, sw } from "./lib";

let api: MockAnthropic;
let app: App;
beforeAll(async () => {
  reset();
  api = await startMockAnthropic();
  app = await launchApp({ ADSCAN_ANTHROPIC_BASE_URL: api.baseUrl, ADSCAN_API_KEY: MOCK_API_KEY });
});
afterAll(async () => { await app?.close(); await api?.close(); });

const DIALOG = '//*[@role="dialog" or @role="alertdialog"]';
async function waitScan(scanId: string, timeout = 200_000): Promise<ScanDetail> {
  const { browser } = app;
  let detail!: ScanDetail;
  await browser.waitUntil(async () => {
    detail = await invokeOk<ScanDetail>(browser, "get_scan", { scanId });
    return !["queued", "running"].includes(detail.summary.status);
  }, { timeout, interval: 500 });
  return detail;
}

test("real app", async () => {
  const { browser } = app;
  const url = await invokeOk<string>(browser, "get_demo_url");
  note(`demo url ${url}`);
  note(`settings ${JSON.stringify(await invokeOk<Settings>(browser, "get_settings"))}`);

  await step(browser, "settings: scan profile empty + add proxy with password", async () => {
    await goTo(browser, "/settings");
    await seeText(browser, "No scan profile is stored yet.");
    await browser.$(button("Add proxy")).scrollIntoView();
    await shot(browser, "settings-proxies-empty");
    await browser.$(button("Add proxy")).click();
    await browser.$(DIALOG).waitForDisplayed();
    await browser.$(`${DIALOG}${field("Name")}`).setValue("Belgium");
    await browser.$(`${DIALOG}${field("Host")}`).setValue("be.proxy.example");
    await browser.$(`${DIALOG}${field("User name (optional)")}`).setValue("scanner");
    await browser.$(`${DIALOG}${field("Password (optional)")}`).setValue("s3cret");
    await shot(browser, "proxy-dialog-add");
    await browser.$(`${DIALOG}${button("Add proxy")}`).click();
    await browser.$(DIALOG).waitForExist({ reverse: true, timeout: 10_000 });
    await seeText(browser, "Password stored", { within: labelled("Saved proxies") });
    const settings = await invokeOk<Settings>(browser, "get_settings");
    note(`after add: ${JSON.stringify(settings.proxies)}`);
    await shot(browser, "settings-proxy-added");
  });

  await step(browser, "settings: socks5 + user name refused inline", async () => {
    await browser.$(button("Add proxy")).click();
    await browser.$(DIALOG).waitForDisplayed();
    await browser.$(`${DIALOG}${field("Name")}`).setValue("belgium");
    await browser.$(`${DIALOG}${field("Host")}`).setValue("http://x.example:80");
    await browser.$(`${DIALOG}${button("Add proxy")}`).click();
    const text = await seeText(browser, "already exists", { within: DIALOG });
    note(`dialog errors: ${text.replace(/\n+/g, " | ")}`);
    await shot(browser, "proxy-dialog-errors");
    await browser.$(`${DIALOG}${button("Cancel")}`).click();
    await browser.$(DIALOG).waitForExist({ reverse: true, timeout: 10_000 });
  });

  await step(browser, "settings: edit proxy, remove password, change port", async () => {
    await browser.$(button("Edit Belgium")).click();
    await browser.$(DIALOG).waitForDisplayed();
    await seeText(browser, "A password for this proxy is stored in your keychain.", { within: DIALOG });
    await shot(browser, "proxy-dialog-edit");
    await browser.$(`${DIALOG}${button("Remove password")}`).click();
    await browser.waitUntil(async () => !((await textOf(browser, DIALOG)) ?? "").includes("A password for this proxy is stored"), { timeout: 10_000 });
    await browser.$(`${DIALOG}${field("Port")}`).setValue("3128");
    await browser.$(`${DIALOG}${button("Save proxy")}`).click();
    await browser.$(DIALOG).waitForExist({ reverse: true, timeout: 10_000 });
    const list = await seeText(browser, "http://be.proxy.example:3128", { within: labelled("Saved proxies") });
    if (list.includes("Password stored")) throw new Error("password badge still shown");
    note(`after edit: ${JSON.stringify((await invokeOk<Settings>(browser, "get_settings")).proxies)}`);
  });

  await step(browser, "settings: default proxy select + delete resets default", async () => {
    await browser.$(labelled("Default proxy")).click();
    await browser.$(menuitem("Belgium")).click();
    await browser.waitUntil(async () => (await invokeOk<Settings>(browser, "get_settings")).defaultProxy.kind === "profile", { timeout: 10_000 });
    note(`default: ${JSON.stringify((await invokeOk<Settings>(browser, "get_settings")).defaultProxy)}`);
    await browser.$(button("Delete Belgium")).click();
    await browser.$(DIALOG).waitForDisplayed();
    await browser.$(`${DIALOG}${button("Delete")}`).click();
    await browser.waitUntil(async () => (await invokeOk<Settings>(browser, "get_settings")).proxies.length === 0, { timeout: 10_000 });
    const settings = await invokeOk<Settings>(browser, "get_settings");
    note(`after delete: default ${JSON.stringify(settings.defaultProxy)}`);
    await seeText(browser, "System proxy", { within: labelled("Default proxy") });
  });

  let configurationId = "";
  await step(browser, "composer: browse headless, save as configuration", async () => {
    await goTo(browser, "/");
    await browser.$(labelled("Page URL")).waitForDisplayed();
    await browser.$(labelled("Page URL")).setValue(url);
    await browser.$(button("Options")).click();
    await browser.$(radio("Browse like a visitor")).click();
    await seeText(browser, "Pages to visit");
    if ((await browser.$(sw("Show the browser")).getAttribute("aria-checked")) === "true") await browser.$(sw("Show the browser")).click();
    if ((await browser.$(sw("AI logic analysis")).getAttribute("aria-checked")) === "true") await browser.$(sw("AI logic analysis")).click();
    await shot(browser, "composer-browse");
    await browser.$(button("Save as configuration")).click();
    await browser.$(DIALOG).waitForDisplayed();
    await browser.$(`${DIALOG}${field("Name")}`).setValue("Demo browse");
    await browser.$(`${DIALOG}${button("Save configuration")}`).click();
    await browser.$(DIALOG).waitForExist({ reverse: true, timeout: 10_000 });
    await browser.waitUntil(async () => (await hashRoute(browser)).includes("configuration="), { timeout: 10_000 });
    const configs = await invokeOk<ScanConfiguration[]>(browser, "list_scan_configurations");
    note(`configs: ${JSON.stringify(configs)}`);
    configurationId = configs[0]?.id ?? "";
    await seeText(browser, "Demo browse", { within: labelled("New scan") });
    const text = (await textOf(browser, labelled("New scan"))) ?? "";
    note(`composer with config: ${text.replace(/\n+/g, " | ").slice(0, 800)}`);
    if (text.includes("Edited")) throw new Error("freshly saved configuration shows as Edited");
    await shot(browser, "composer-config-loaded");
  });

  let browseId = "";
  await step(browser, "run configuration from composer; live Visit panel; report", async () => {
    await browser.$(`${labelled("New scan")}//button[@type="submit"]`).click();
    await browser.waitUntil(async () => (await hashRoute(browser)).startsWith("/scans/"), { timeout: 10_000 });
    browseId = (await hashRoute(browser)).replace("/scans/", "").split("?")[0] ?? "";
    const visit = await seeText(browser, "Now on page", { within: labelled("Visit"), timeout: 60_000 });
    note(`visit panel first: ${visit.replace(/\n+/g, " | ")}`);
    await shot(browser, "live-visit-1");
    await browser.waitUntil(async () => ((await textOf(browser, labelled("Pages visited"))) ?? "").includes("2"), { timeout: 90_000 }).catch(() => note("  (never saw page 2 in panel)"));
    note(`visit panel later: ${((await textOf(browser, labelled("Visit"))) ?? "gone").replace(/\n+/g, " | ")}`);
    await shot(browser, "live-visit-2");
    await browser.$(reportTab("Findings")).waitForDisplayed({ timeout: 200_000 });
    const detail = await waitScan(browseId);
    note(`browse summary: ${JSON.stringify(detail.summary)}`);
    note(`browse options: ${JSON.stringify(detail.options)}`);
    note(`browse environment: ${JSON.stringify(detail.environment)}`);
    note(`browse pages: ${JSON.stringify(detail.pages)}`);
    note(`browse findings: ${JSON.stringify(detail.findings.map((f) => ({ t: f.title, s: f.severity, c: f.category, src: f.source, pages: f.pages, bidders: f.affectedBidders })))}`);
    const logs = await invokeOk<LogEntry[]>(browser, "get_logs", { scanId: browseId });
    note(`visitor logs: ${JSON.stringify(logs.filter((e) => e.kind === "visitor").map((e) => ({ m: e.message, l: e.level, d: e.data })))}`);
    note(`capture msgs: ${JSON.stringify(logs.filter((e) => e.kind === "message").slice(0, 12).map((e) => e.message))}`);
    await shot(browser, "report-browse-top");
    note(`report text: ${(await bodyText(browser)).replace(/\n+/g, " | ").slice(0, 3500)}`);
    const configs = await invokeOk<ScanConfiguration[]>(browser, "list_scan_configurations");
    note(`config after run: ${JSON.stringify(configs.map((c) => ({ lastScanId: c.lastScanId, lastRunAt: c.lastRunAt })))}`);
  });

  await step(browser, "report: page switcher, tabs keep page, audit trail visitor entries", async () => {
    await browser.$(`${labelled("Page views")}${button("2")}`).click();
    await seeText(browser, "into the visit", { within: labelled("Page views") });
    note(`hash after page 2: ${await hashRoute(browser)}`);
    await browser.$(reportTab("Auctions")).click();
    note(`hash auctions: ${await hashRoute(browser)}`);
    await shot(browser, "report-browse-auctions-p2");
    await browser.$(reportTab("Ad slots")).click();
    await shot(browser, "report-browse-slots-p2");
    await browser.$(`${labelled("Page views")}${button("All pages")}`).click();
    await shot(browser, "report-browse-slots-all");
    await browser.$(reportTab("Audit trail")).click();
    await browser.$(button("Filter by kind")).click();
    await browser.$('//*[@role="menuitemcheckbox"][starts-with(normalize-space(.), "Visitor")]').click();
    await browser.keys("Escape");
    await shot(browser, "report-browse-audit-visitor");
    note(`audit: ${((await textOf(browser, "//main")) ?? "").replace(/\n+/g, " | ").slice(-2500)}`);
  });

  await step(browser, "configurations screen; edit -> Edited -> update", async () => {
    await goTo(browser, "/configurations");
    const text = await seeText(browser, "Demo browse", { within: "//main" });
    note(`configs screen: ${text.replace(/\n+/g, " | ")}`);
    await shot(browser, "configurations");
    await goTo(browser, `/?configuration=${configurationId}`);
    await seeText(browser, "Demo browse", { within: labelled("New scan") });
    await browser.$(button("More pages")).click();
    await seeText(browser, "Edited", { within: labelled("New scan") });
    await shot(browser, "composer-config-edited");
    await browser.$(button("Update configuration")).click();
    await browser.waitUntil(async () => !((await textOf(browser, labelled("New scan"))) ?? "").includes("Edited"), { timeout: 10_000 });
    const configs = await invokeOk<ScanConfiguration[]>(browser, "list_scan_configurations");
    note(`config after update: ${JSON.stringify(configs[0]?.options)}`);
  });

  await step(browser, "proxy in use by a configuration cannot be deleted; missing-proxy start error", async () => {
    const saved = await invokeOk<Settings>(browser, "save_proxy_profile", { profile: { name: "Germany", scheme: "https", host: "de.proxy.example", port: 443, username: null } });
    const id = saved.proxies[0]?.id ?? "";
    await invokeOk(browser, "save_scan_configuration", { configuration: { name: "Demo via Germany", options: { url, device: "mobile", observeSeconds: 5, autoScroll: true, acceptConsent: true, aiEnabled: false, visitor: "scroll", proxy: { kind: "profile", id } } } });
    note(`delete in use: ${JSON.stringify(await invoke(browser, "delete_proxy_profile", { id }))}`);
    note(`bad proxy start: ${JSON.stringify(await invoke(browser, "start_scan", { options: { url, device: "desktop", observeSeconds: 5, autoScroll: true, acceptConsent: true, aiEnabled: false, proxy: { kind: "profile", id: "nope" } } }))}`);
    note(`dup config name: ${JSON.stringify(await invoke(browser, "save_scan_configuration", { configuration: { name: "demo BROWSE", options: { url, device: "mobile", observeSeconds: 5, autoScroll: true, acceptConsent: true, aiEnabled: false } } }))}`);
    note(`dup proxy name: ${JSON.stringify(await invoke(browser, "save_proxy_profile", { profile: { name: "germany", scheme: "https", host: "de.proxy.example", port: 443, username: null } }))}`);
    note(`extra pages 9: ${JSON.stringify(await invoke(browser, "start_scan", { options: { url, device: "desktop", observeSeconds: 5, autoScroll: true, acceptConsent: true, aiEnabled: false, visitor: "browse", extraPages: 9 } }))}`);
    note(`budget 50: ${JSON.stringify(await invoke(browser, "start_scan", { options: { url, device: "desktop", observeSeconds: 5, autoScroll: true, acceptConsent: true, aiEnabled: false, budgetUsd: 50 } }))}`);
    note(`budget 0.01: ${JSON.stringify(await invoke(browser, "start_scan", { options: { url, device: "desktop", observeSeconds: 5, autoScroll: true, acceptConsent: true, aiEnabled: false, budgetUsd: 0.01 } }))}`);
    note(`settings budget 0.01: ${JSON.stringify(await invoke(browser, "update_settings", { patch: { budgetUsd: 0.01 } }))}`);
    await goTo(browser, "/settings");
    await browser.$(button("Delete Germany")).waitForDisplayed();
    await browser.$(button("Delete Germany")).click();
    await browser.$(DIALOG).waitForDisplayed();
    await browser.$(`${DIALOG}${button("Delete")}`).click();
    const toast = await seeText(browser, "Could not delete the proxy");
    await shot(browser, "proxy-delete-refused");
    note(`toast: ${toast.replace(/\n+/g, " | ").slice(-600)}`);
    await goTo(browser, "/configurations");
    await seeText(browser, "Demo via Germany");
    await shot(browser, "configurations-two");
    await goTo(browser, "/history");
    await seeText(browser, "Demo browse");
    await shot(browser, "history");
    await goTo(browser, "/");
    await seeText(browser, "Demo via Germany");
    await shot(browser, "home-with-configs");
  });

  await step(browser, "scan profile: kept profile scan, info, clear", async () => {
    const started = await invokeOk<{ id: string }>(browser, "start_scan", { options: { url, device: "desktop", observeSeconds: 5, autoScroll: true, acceptConsent: true, aiEnabled: false, visitor: "scroll", headless: true, persistentProfile: true, proxy: { kind: "none" } } });
    await goTo(browser, "/settings");
    await new Promise((r) => setTimeout(r, 4000));
    note(`profile during: ${JSON.stringify(await invokeOk<ScanProfileInfo>(browser, "get_scan_profile"))}`);
    note(`clear during: ${JSON.stringify(await invoke(browser, "clear_scan_profile"))}`);
    note(`settings text during: ${((await textOf(browser, "//main")) ?? "").match(/Stored scan profile[^|]{0,200}/s)?.[0]?.replace(/\n+/g, " | ")}`);
    const detail = await waitScan(started.id);
    note(`profile scan: ${detail.summary.status} env ${JSON.stringify(detail.environment)}`);
    note(`profile after: ${JSON.stringify(await invokeOk<ScanProfileInfo>(browser, "get_scan_profile"))}`);
    await seeText(browser, "on this Mac", { timeout: 15_000 });
    await browser.$(button("Clear scan profile")).scrollIntoView();
    await shot(browser, "settings-profile-stored");
    await browser.$(button("Clear scan profile")).click();
    await browser.$(DIALOG).waitForDisplayed();
    await browser.$(`${DIALOG}${button("Clear profile")}`).click();
    await seeText(browser, "No scan profile is stored yet.", { timeout: 15_000 });
    note(`profile cleared: ${JSON.stringify(await invokeOk<ScanProfileInfo>(browser, "get_scan_profile"))}`);
  });

  await step(browser, "manual session started and finished from the UI", async () => {
    await goTo(browser, "/");
    await browser.$(labelled("Page URL")).waitForDisplayed();
    await browser.$(labelled("Page URL")).setValue(url);
    await browser.$(button("Options")).click();
    await browser.$(radio("I'll browse myself")).click();
    await shot(browser, "composer-manual");
    await browser.$(`${labelled("New scan")}//button[@type="submit"]`).click();
    await browser.waitUntil(async () => (await hashRoute(browser)).startsWith("/scans/"), { timeout: 10_000 });
    const id = (await hashRoute(browser)).replace("/scans/", "").split("?")[0] ?? "";
    await seeText(browser, "You are browsing", { within: labelled("Manual session"), timeout: 30_000 });
    await shot(browser, "manual-bar-early");
    note(`manual bar early: ${((await textOf(browser, labelled("Manual session"))) ?? "").replace(/\n+/g, " | ")}`);
    await seeText(browser, "1 page", { within: labelled("Manual session"), timeout: 60_000 });
    await new Promise((r) => setTimeout(r, 6000));
    await shot(browser, "manual-bar");
    note(`manual bar: ${((await textOf(browser, labelled("Manual session"))) ?? "").replace(/\n+/g, " | ")}`);
    note(`manual visit panel: ${((await textOf(browser, labelled("Visit"))) ?? "none").replace(/\n+/g, " | ")}`);
    await browser.$(button("Finish capture")).click();
    await browser.$(reportTab("Findings")).waitForDisplayed({ timeout: 120_000 });
    const detail = await waitScan(id);
    note(`manual summary: ${JSON.stringify(detail.summary)}`);
    note(`manual options: ${JSON.stringify(detail.options)}`);
    note(`manual env: ${JSON.stringify(detail.environment)}`);
    note(`manual pages: ${JSON.stringify(detail.pages)}`);
    const logs = await invokeOk<LogEntry[]>(browser, "get_logs", { scanId: id });
    note(`manual logs: ${JSON.stringify(logs.filter((e) => e.node === "capture").map((e) => `${e.kind}:${e.message}`))}`);
    await shot(browser, "report-manual");
    note(`manual report: ${(await bodyText(browser)).replace(/\n+/g, " | ").slice(0, 2500)}`);
  });

  await step(browser, "history filter by configuration; re-scan row", async () => {
    await goTo(browser, "/history");
    await browser.$(labelled("Configuration")).waitForDisplayed();
    await browser.$(labelled("Configuration")).click();
    await browser.$(menuitem("Demo browse")).click();
    await new Promise((r) => setTimeout(r, 1500));
    note(`history filtered: ${((await textOf(browser, "//main")) ?? "").replace(/\n+/g, " | ").slice(0, 1200)}`);
    await shot(browser, "history-filtered");
    const list = await invokeOk<ScanList>(browser, "list_scans", { query: { configurationId } });
    note(`list by config: ${list.total}`);
  });
});

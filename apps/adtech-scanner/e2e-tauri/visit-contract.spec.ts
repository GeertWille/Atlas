import { afterAll, beforeAll, expect, test } from "vitest";
import type { ScanConfiguration, ScanDetail, ScanList, ScanProfileInfo, ScanSummary, Settings } from "../src/bindings";
import { type App, invoke, invokeOk, launchApp } from "./support/app";
import { type MockAnthropic, startMockAnthropic } from "./support/mock-anthropic";

const UNKNOWN = "00000000-0000-4000-8000-000000000000";
const BELGIUM = { name: "Belgium", scheme: "http", host: "be.proxy.example", port: 3128, username: "scanner" };

let api: MockAnthropic;
let app: App;

beforeAll(async () => {
  api = await startMockAnthropic();
  // The password override keeps every secret in memory: this machine has no keychain, and the
  // override answers for each proxy profile.
  app = await launchApp({ ADSCAN_ANTHROPIC_BASE_URL: api.baseUrl, ADSCAN_PROXY_PASSWORD: "e2e-proxy-password" });
});

afterAll(async () => {
  await app?.close();
  await api?.close();
});

const invalid = (message: unknown = expect.any(String)) => ({ rejected: { code: "invalidInput", message } });
const notFound = { rejected: { code: "notFound", message: expect.any(String) } };

test("settings carry the visitor defaults and take patches for them", async () => {
  const { browser } = app;
  expect(await invokeOk<Settings>(browser, "get_settings")).toMatchObject({
    defaultVisitor: "scroll",
    defaultExtraPages: 2,
    persistentProfile: false,
    proxies: [],
    defaultProxy: { kind: "system" },
  });
  const patched = await invokeOk<Settings>(browser, "update_settings", {
    patch: { defaultVisitor: "browse", defaultExtraPages: 5, persistentProfile: true, defaultProxy: { kind: "none" } },
  });
  expect(patched).toMatchObject({
    defaultVisitor: "browse",
    defaultExtraPages: 5,
    persistentProfile: true,
    defaultProxy: { kind: "none" },
  });
  expect(await invoke(browser, "update_settings", { patch: { defaultExtraPages: 6 } })).toEqual(invalid());
  expect(
    await invoke(browser, "update_settings", { patch: { defaultProxy: { kind: "profile", id: UNKNOWN } } }),
  ).toEqual(invalid());
  await invokeOk(browser, "update_settings", {
    patch: {
      defaultVisitor: "scroll",
      defaultExtraPages: 2,
      persistentProfile: false,
      defaultProxy: { kind: "system" },
    },
  });
});

test("the scan profile is reported and cleared", async () => {
  const { browser } = app;
  const info = await invokeOk<ScanProfileInfo>(browser, "get_scan_profile");
  expect(info).toEqual({ exists: false, sizeBytes: 0, lastUsedAt: null, inUse: false });
  // Clearing a profile that does not exist is not an error.
  expect(await invokeOk<ScanProfileInfo>(browser, "clear_scan_profile")).toEqual(info);
});

test("proxy profiles are saved, given a password, used as the default and deleted", async () => {
  const { browser } = app;
  const saved = await invokeOk<Settings>(browser, "save_proxy_profile", { profile: BELGIUM });
  expect(saved.proxies).toEqual([{ ...BELGIUM, id: expect.any(String), hasPassword: true }]);
  const [{ id }] = saved.proxies;

  const updated = await invokeOk<Settings>(browser, "save_proxy_profile", { profile: { ...BELGIUM, id, port: 8080 } });
  expect(updated.proxies).toEqual([{ ...BELGIUM, id, port: 8080, hasPassword: true }]);

  expect(await invoke(browser, "save_proxy_profile", { profile: { ...BELGIUM, name: "belgium" } })).toEqual(
    invalid('A proxy named "belgium" already exists'),
  );
  expect(
    await invoke(browser, "save_proxy_profile", { profile: { ...BELGIUM, name: "Socks", scheme: "socks5" } }),
  ).toEqual(invalid(expect.stringContaining("SOCKS5")));
  expect(
    await invoke(browser, "save_proxy_profile", {
      profile: { ...BELGIUM, name: "Url", host: "http://proxy.example:80" },
    }),
  ).toEqual(invalid(expect.stringContaining("host name or IP address only")));
  expect(await invoke(browser, "save_proxy_profile", { profile: { ...BELGIUM, id: UNKNOWN } })).toEqual(notFound);

  // Only whether a password exists ever comes back.
  const withPassword = await invokeOk<Settings>(browser, "set_proxy_password", { id, password: "another-secret" });
  expect(JSON.stringify(withPassword)).not.toContain("another-secret");
  expect(JSON.stringify(withPassword)).not.toContain("e2e-proxy-password");
  expect(await invoke(browser, "set_proxy_password", { id, password: "" })).toEqual(invalid());
  expect(await invoke(browser, "set_proxy_password", { id: UNKNOWN, password: "x" })).toEqual(notFound);
  expect((await invokeOk<Settings>(browser, "clear_proxy_password", { id })).proxies).toHaveLength(1);
  expect(await invoke(browser, "clear_proxy_password", { id: UNKNOWN })).toEqual(notFound);

  // The default names the profile with a snapshot; deleting the profile puts the default back.
  const asDefault = await invokeOk<Settings>(browser, "update_settings", {
    patch: { defaultProxy: { kind: "profile", id } },
  });
  expect(asDefault.defaultProxy).toEqual({ kind: "profile", id, name: "Belgium", host: "be.proxy.example" });
  const deleted = await invokeOk<Settings>(browser, "delete_proxy_profile", { id });
  expect(deleted).toMatchObject({ proxies: [], defaultProxy: { kind: "system" } });
  expect(await invoke(browser, "delete_proxy_profile", { id })).toEqual(notFound);
});

test("a scan configuration is saved, run, found in the history and deleted without its scans", async () => {
  const { browser } = app;
  const url = await invokeOk<string>(browser, "get_demo_url");
  const proxyId = (await invokeOk<Settings>(browser, "save_proxy_profile", { profile: BELGIUM })).proxies[0].id;
  const options = {
    url,
    device: "mobile",
    observeSeconds: 30,
    autoScroll: false,
    acceptConsent: false,
    aiEnabled: false,
    visitor: "none",
    headless: true,
    seed: 7,
    budgetUsd: 0.2,
  };

  const saved = await invokeOk<ScanConfiguration>(browser, "save_scan_configuration", {
    configuration: { name: "  Demo site  ", options },
  });
  const { seed: _seed, ...stored } = options;
  expect(saved).toEqual({
    id: expect.any(String),
    name: "Demo site",
    options: stored,
    createdAt: expect.any(Number),
    updatedAt: saved.createdAt,
    lastScanId: null,
    lastRunAt: null,
  });
  expect(await invokeOk<ScanConfiguration[]>(browser, "list_scan_configurations")).toEqual([saved]);
  expect(await invoke(browser, "save_scan_configuration", { configuration: { name: "demo SITE", options } })).toEqual(
    invalid('A scan configuration named "demo SITE" already exists'),
  );
  expect(
    await invoke(browser, "save_scan_configuration", {
      configuration: { name: "Bad", options: { ...options, url: "file:///etc/passwd" } },
    }),
  ).toEqual(invalid());

  // An update keeps the id, and a proxy the configuration uses cannot be deleted.
  const withProxy = await invokeOk<ScanConfiguration>(browser, "save_scan_configuration", {
    configuration: {
      id: saved.id,
      name: "Demo site",
      options: { ...options, proxy: { kind: "profile", id: proxyId } },
    },
  });
  expect(withProxy.id).toBe(saved.id);
  expect(withProxy.options.proxy).toEqual({ kind: "profile", id: proxyId, name: "Belgium", host: "be.proxy.example" });
  expect(await invoke(browser, "delete_proxy_profile", { id: proxyId })).toEqual(
    invalid(expect.stringContaining('"Demo site"')),
  );
  await invokeOk(browser, "save_scan_configuration", { configuration: { id: saved.id, name: "Demo site", options } });
  expect((await invokeOk<Settings>(browser, "delete_proxy_profile", { id: proxyId })).proxies).toEqual([]);

  // Exactly one of options and configuration starts a scan.
  expect(await invoke(browser, "start_scan", {})).toEqual(invalid());
  expect(await invoke(browser, "start_scan", { options, configurationId: saved.id })).toEqual(invalid());
  expect(await invoke(browser, "start_scan", { configurationId: UNKNOWN })).toEqual(notFound);

  const started = await invokeOk<ScanSummary>(browser, "start_scan", { configurationId: saved.id });
  expect(started).toMatchObject({
    url,
    device: "mobile",
    status: "running",
    configuration: { id: saved.id, name: "Demo site" },
    proxy: { kind: "system", name: null, host: null },
  });
  const [ran] = await invokeOk<ScanConfiguration[]>(browser, "list_scan_configurations");
  expect(ran).toMatchObject({ lastScanId: started.id, lastRunAt: started.startedAt });

  // Finishing only means something to a manual visit; for this scan it changes nothing.
  expect(await invokeOk<null>(browser, "finish_capture", { scanId: started.id })).toBeNull();
  expect(await invoke(browser, "finish_capture", { scanId: UNKNOWN })).toEqual(notFound);
  expect((await invokeOk<ScanDetail>(browser, "get_scan", { scanId: started.id })).summary.status).toBe("running");
  await invokeOk(browser, "cancel_scan", { scanId: started.id });
  await browser.waitUntil(
    async () =>
      (await invokeOk<ScanDetail>(browser, "get_scan", { scanId: started.id })).summary.status === "cancelled",
    { timeout: 20_000 },
  );

  // The stored scan holds the options as they were settled, seed included.
  const detail = await invokeOk<ScanDetail>(browser, "get_scan", { scanId: started.id });
  expect(detail.options).toMatchObject({
    visitor: "none",
    extraPages: 0,
    seed: expect.any(Number),
    headless: true,
    persistentProfile: false,
    proxy: { kind: "system" },
    budgetUsd: 0.2,
  });
  expect(detail.pages).toEqual([]);
  expect(detail.environment).toBeNull();

  const ofConfiguration = await invokeOk<ScanList>(browser, "list_scans", { query: { configurationId: saved.id } });
  expect(ofConfiguration.items.map((scan) => scan.id)).toEqual([started.id]);
  expect((await invokeOk<ScanList>(browser, "list_scans", { query: { configurationId: UNKNOWN } })).total).toBe(0);
  const markdown = await invokeOk<string>(browser, "export_scan", { scanId: started.id, format: "markdown" });
  expect(markdown).toContain("- **Scan configuration:** Demo site");

  expect(await invokeOk<null>(browser, "delete_scan_configuration", { id: saved.id })).toBeNull();
  expect(await invoke(browser, "delete_scan_configuration", { id: saved.id })).toEqual(notFound);
  expect(await invokeOk<ScanConfiguration[]>(browser, "list_scan_configurations")).toEqual([]);
  const kept = await invokeOk<ScanDetail>(browser, "get_scan", { scanId: started.id });
  expect(kept.summary.configuration).toEqual({ id: saved.id, name: "Demo site" });
  await invokeOk(browser, "delete_scan", { scanId: started.id });
});

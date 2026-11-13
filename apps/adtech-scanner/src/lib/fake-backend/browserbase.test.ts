import { beforeEach, describe, expect, test } from "vitest";
import type { LogEntry, ScanEvent, ScanOptions, ScanStatus } from "@/bindings";
import { ApiError } from "@/lib/api/error";
import type { ApiPort } from "@/lib/api/port";
import { visitorActivity } from "@/shared/domain/visitor-log";
import type { BrowserbaseFailure } from "./browserbase";
import { createFakeBackend } from "./index";

const page: ScanOptions = {
  url: "https://www.morningledger.example/",
  device: "desktop",
  observeSeconds: 20,
  autoScroll: true,
  acceptConsent: true,
  aiEnabled: false,
};
const onBrowserbase: ScanOptions = { ...page, browser: { kind: "browserbase" } };

const fresh = (browserbaseFailure?: BrowserbaseFailure, profile: "fresh" | "no-browser" = "fresh") =>
  createFakeBackend({ profile, speed: 5000, browserbaseFailure });

async function connected(api: ApiPort = fresh()) {
  expect(await api.setBrowserbaseCredentials("bb_live_0123456789abcdef")).toEqual({
    valid: true,
    message: null,
    projects: [],
  });
  return api;
}

/** Resolves with the scan's final status; later summary events (the late usage) keep being collected. */
async function untilFinished(api: ApiPort) {
  let finish: (status: ScanStatus) => void = () => {};
  const finished = new Promise<ScanStatus>((resolve) => {
    finish = resolve;
  });
  const events: ScanEvent[] = [];
  await api.subscribeScanEvents((event) => {
    events.push(event);
    if (event.type === "scanUpdated" && event.summary.status !== "running") finish(event.summary.status);
  });
  return { finished, events };
}

async function run(api: ApiPort, options: ScanOptions, outcome: ScanStatus = "completed") {
  const { finished, events } = await untilFinished(api);
  const summary = await api.startScan(options);
  expect(await finished).toBe(outcome);
  return { detail: await api.getScan(summary.id), logs: await api.getLogs(summary.id), events, id: summary.id };
}

const remoteEvents = (logs: LogEntry[]) =>
  logs.flatMap((entry) => {
    const activity = visitorActivity(entry);
    return activity?.activity === "browser" ? [activity.event] : [];
  });
const rejects = (promise: Promise<unknown>, code: string, message?: string) =>
  expect(promise).rejects.toSatisfy(
    (error) => error instanceof ApiError && error.code === code && (message === undefined || error.message === message),
  );

beforeEach(() => {
  localStorage.clear();
});

describe("credentials", () => {
  test("a plausible key is accepted and its one project adopted; settings then say so", async () => {
    const api = fresh();
    expect((await api.getSettings()).browserbase).toMatchObject({
      connected: false,
      projectName: null,
      concurrency: null,
    });
    await connected(api);
    expect((await api.getSettings()).browserbase).toMatchObject({
      connected: true,
      projectName: "Local",
      concurrency: 3,
    });
    // Stored: a restart still has it.
    expect((await fresh().getSettings()).browserbase.connected).toBe(true);
  });

  test.each([
    ["", "Enter a Browserbase API key."],
    ["bb_live_invalid_key", "Browserbase did not accept the API key."],
    ["has a space", "Browserbase did not accept the API key."],
  ])("the key %j is refused and nothing is stored", async (key, message) => {
    const api = fresh();
    expect(await api.setBrowserbaseCredentials(key)).toEqual({ valid: false, message, projects: [] });
    expect((await api.getSettings()).browserbase.connected).toBe(false);
  });

  test("a key with several projects asks which one, and takes only one of its own", async () => {
    const api = fresh();
    const asked = await api.setBrowserbaseCredentials("bb_live_multi_project");
    expect(asked).toMatchObject({
      valid: false,
      message: "This key has more than one Browserbase project. Choose the one to use.",
    });
    expect(asked.projects.map((project) => project.name)).toEqual(["Local", "Staging"]);
    expect((await api.getSettings()).browserbase.connected).toBe(false);
    expect(await api.setBrowserbaseCredentials("bb_live_multi_project", "prj-unknown")).toMatchObject({
      valid: false,
      message: "This Browserbase project was not found for this key.",
    });
    expect(await api.setBrowserbaseCredentials("bb_live_multi_project", asked.projects[1]?.id)).toMatchObject({
      valid: true,
    });
    expect((await api.getSettings()).browserbase.projectName).toBe("Staging");
  });

  test("removing them puts the default back to this Mac and forgets the kept profile", async () => {
    const api = await connected();
    await api.updateSettings({ defaultBrowser: "browserbase", browserbase: { persistentProfile: true } });
    await run(api, page);
    expect((await api.getBrowserbaseProfile()).exists).toBe(true);
    const settings = await api.clearBrowserbaseCredentials();
    expect(settings.defaultBrowser).toBe("local");
    expect(settings.browserbase).toMatchObject({ connected: false, projectName: null, concurrency: null });
    expect((await api.getBrowserbaseProfile()).exists).toBe(false);
  });

  test("the account checks need credentials", async () => {
    const api = fresh();
    const notSetUp = "Browserbase is not set up. Add the API key in Settings.";
    await rejects(api.testBrowserbase(), "browserbaseAuth", notSetUp);
    await rejects(api.probeBrowserbase(), "browserbaseAuth", notSetUp);
    await rejects(api.clearBrowserbaseProfile(), "browserbaseAuth", notSetUp);
  });

  test("the connection test reports the shared project, and the probe what a blank page sees", async () => {
    const api = await connected();
    expect(await api.testBrowserbase()).toEqual({
      projectName: "Local",
      concurrency: 3,
      browserMinutes: 483,
      proxyBytes: 1_998_349_077,
      // The other application's browser.
      runningSessions: 1,
    });
    expect(await api.probeBrowserbase("eu-central-1")).toEqual({
      region: "eu-central-1",
      browserVersion: "Chrome/157.0.8103.79",
      userAgent:
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36",
      webdriver: false,
      // Every region reports this until a scan sets the region's own.
      timezone: "America/Los_Angeles",
      language: "en-US",
      roundTripMs: 118.6,
      sessionSeconds: 6,
    });
    expect((await api.probeBrowserbase()).region).toBe("us-west-2");
  });
});

describe("settings", () => {
  test("Browserbase cannot be the default before it is set up", async () => {
    await rejects(
      fresh().updateSettings({ defaultBrowser: "browserbase" }),
      "invalidInput",
      "Add a Browserbase API key before making Browserbase the default.",
    );
  });

  test("a patch changes only what it names, and the default location is stored as a scan would use it", async () => {
    const api = await connected();
    await api.saveProxyProfile({
      name: "Belgium",
      scheme: "http",
      host: "be.proxy.example",
      port: 8080,
      username: null,
    });
    await api.saveProxyProfile({ name: "Office", scheme: "http", host: "10.0.0.7", port: 3128, username: null });
    const [belgium, office] = (await api.getSettings()).proxies;
    let settings = await api.updateSettings({ browserbase: { region: "eu-central-1" } });
    expect(settings.browserbase).toMatchObject({ region: "eu-central-1", connected: true, persistentProfile: false });
    settings = await api.updateSettings({ browserbase: { defaultProxy: { kind: "system" } } });
    expect(settings.browserbase.defaultProxy).toEqual({ kind: "none" });
    settings = await api.updateSettings({
      browserbase: { defaultProxy: { kind: "browserbaseLocation", country: "us", state: "ny", city: "new york" } },
    });
    expect(settings.browserbase.defaultProxy).toEqual({
      kind: "browserbaseLocation",
      country: "US",
      state: "NY",
      city: "NEW_YORK",
    });
    await rejects(
      api.updateSettings({ browserbase: { defaultProxy: { kind: "browserbaseLocation", country: "Belgium" } } }),
      "invalidInput",
      "Enter the country of the Browserbase location as a two-letter code, for example BE",
    );
    await rejects(
      api.updateSettings({ browserbase: { defaultProxy: { kind: "profile", id: office?.id ?? "" } } }),
      "invalidInput",
      'Browserbase cannot reach the proxy "Office": its address is on this Mac or your network. Use a proxy that is reachable from the internet.',
    );
    // A location is no proxy for a browser on this Mac.
    await rejects(
      api.updateSettings({ defaultProxy: { kind: "browserbaseLocation", country: "BE" } }),
      "invalidInput",
      "A Browserbase location can only be used when the browser runs on Browserbase.",
    );

    settings = await api.updateSettings({ browserbase: { defaultProxy: { kind: "profile", id: belgium?.id ?? "" } } });
    expect(settings.browserbase.defaultProxy).toMatchObject({ kind: "profile", name: "Belgium" });
    // Deleting that proxy puts the Browserbase default back to the region.
    expect((await api.deleteProxyProfile(belgium?.id ?? "")).browserbase.defaultProxy).toEqual({ kind: "none" });
  });
});

describe("where a scan runs", () => {
  test("Browserbase needs credentials", async () => {
    await rejects(
      fresh().startScan(onBrowserbase),
      "browserbaseAuth",
      "Browserbase is not set up. Add the API key in Settings.",
    );
  });

  test.each([
    "http://127.0.0.1:47615/demo-publisher/",
    "http://localhost:3000/",
    "http://192.168.1.20/",
    "http://printer.local/",
    "http://intranet/",
  ])("%s is refused for Browserbase, and scanned on this Mac when the settings would have chosen it", async (url) => {
    const api = await connected();
    await rejects(
      api.startScan({ ...onBrowserbase, url }),
      "invalidInput",
      "This address is on this Mac or your network; a cloud browser cannot reach it. Scan it with this Mac's Chrome.",
    );
    await api.updateSettings({ defaultBrowser: "browserbase" });
    const { detail } = await run(api, { ...page, url });
    expect(detail.options.browser).toEqual({ kind: "local" });
    expect(detail.summary.remote).toBeNull();
  });

  test("options that do not say follow the settings default and region", async () => {
    const api = await connected();
    expect((await run(api, page)).detail.options.browser).toEqual({ kind: "local" });
    await api.updateSettings({ defaultBrowser: "browserbase", browserbase: { region: "ap-southeast-1" } });
    expect((await run(api, page)).detail.options.browser).toEqual({ kind: "browserbase", region: "ap-southeast-1" });
    expect((await run(api, { ...page, browser: { kind: "local" } })).detail.options.browser).toEqual({ kind: "local" });
  });

  test("a location is only for a Browserbase browser", async () => {
    const api = await connected();
    await rejects(
      api.startScan({ ...page, browser: { kind: "local" }, proxy: { kind: "browserbaseLocation", country: "BE" } }),
      "invalidInput",
      "A Browserbase location can only be used when the browser runs on Browserbase.",
    );
  });

  test("a Browserbase scan needs no browser on this Mac; a local one still does", async () => {
    const api = await connected(fresh(undefined, "no-browser"));
    await rejects(api.startScan({ ...page, browser: { kind: "local" } }), "browserNotFound");
    expect((await run(api, onBrowserbase)).detail.summary.status).toBe("completed");
  });

  test("on Browserbase the options are settled the backend's way", async () => {
    const api = await connected();
    await api.updateSettings({ persistentProfile: true, headless: false });
    const { detail } = await run(api, { ...onBrowserbase, headless: false, persistentProfile: true });
    expect(detail.options).toMatchObject({
      browser: { kind: "browserbase", region: "us-west-2" },
      // No window, the system proxy means nothing there, and the profile switch for Browserbase is off.
      headless: true,
      proxy: { kind: "none" },
      persistentProfile: false,
    });
    expect((await api.getScanProfile()).exists).toBe(false);

    await api.updateSettings({ browserbase: { persistentProfile: true } });
    expect((await run(api, onBrowserbase)).detail.options.persistentProfile).toBe(true);
    expect((await run(api, { ...onBrowserbase, persistentProfile: false })).detail.options.persistentProfile).toBe(
      false,
    );
    expect(await api.getBrowserbaseProfile()).toMatchObject({ exists: true, inUse: false });
    expect((await api.clearBrowserbaseProfile()).exists).toBe(false);
  });
});

describe("a Browserbase scan", () => {
  test("starts no local browser, tells its story in the audit trail, and records what the page saw", async () => {
    const api = await connected();
    const { detail, logs } = await run(api, {
      ...onBrowserbase,
      browser: { kind: "browserbase", region: "eu-central-1" },
    });

    const launch = detail.steps.find((step) => step.node === "launchBrowser");
    expect(launch?.summary).toBe("Browserbase, eu-central-1");
    expect(logs.map((entry) => entry.message)).toContain("Using Browserbase in eu-central-1 (project Local)");
    expect(logs.some((entry) => /remote debugging|headless\)/.test(entry.message))).toBe(false);
    expect(remoteEvents(logs)).toEqual(["sessionCreated", "connected", "timezone", "released"]);
    expect(logs.map((entry) => entry.message)).toContain(
      "Timezone set to Europe/Berlin to match the region (the browser reported America/Los_Angeles)",
    );
    expect(logs.find((entry) => visitorActivity(entry)?.activity === "environment")?.message).toBe(
      "This visit is not flagged as automated: navigator.webdriver is false (Browserbase starts its browser that way); it runs headless, which its user agent does not show",
    );

    expect(detail.environment).toMatchObject({
      webdriver: false,
      headless: true,
      viewportOverridden: true,
      proxy: { kind: "none", name: null, host: null },
      browser: {
        kind: "browserbase",
        region: "eu-central-1",
        browserVersion: "Chrome/157.0.8103.79",
        proxyKind: "none",
        proxyCountry: null,
        settings: { solveCaptchas: true, recordSession: true, logSession: true, blockAds: false, keepAlive: false },
        originalTimezone: "America/Los_Angeles",
        appliedTimezone: "Europe/Berlin",
        language: "en-US",
        roundTripMs: 118.6,
        proxyBytes: 0,
      },
    });
    expect(detail.summary.remote).toMatchObject({ region: "eu-central-1", proxyBytes: 0 });
    expect(detail.summary.remote?.sessionSeconds).toBeGreaterThan(20);
    // Nothing credential-like is ever part of what the frontend gets.
    expect(JSON.stringify({ detail, logs })).not.toMatch(
      /wss?:\/\/|connectUrl|signingKey|debuggerFullscreenUrl|bb_live_/,
    );
  });

  test("a Browserbase location leaves the timezone to Browserbase, and its traffic count arrives late", async () => {
    const api = await connected();
    const { detail, logs, events, id } = await run(api, {
      ...onBrowserbase,
      visitor: "browse",
      extraPages: 2,
      proxy: { kind: "browserbaseLocation", country: "be" },
    });
    expect(detail.options.proxy).toEqual({ kind: "browserbaseLocation", country: "BE" });
    expect(detail.summary.proxy).toEqual({ kind: "browserbaseLocation", name: "BE", host: null });
    expect(detail.environment?.browser).toMatchObject({
      proxyKind: "browserbase",
      proxyCountry: "BE",
      originalTimezone: "Europe/Brussels",
      appliedTimezone: null,
    });
    expect(logs.map((entry) => entry.message)).toContain(
      "The browser reports the timezone Europe/Brussels, set by Browserbase for its proxy; left as it is",
    );
    await expect
      .poll(async () => (await api.getScan(id)).summary.remote?.proxyBytes, { timeout: 3_000 })
      .toBe(3 * 18_486_000);
    const settled = await api.getScan(id);
    expect(settled.environment?.browser).toMatchObject({ proxyBytes: 3 * 18_486_000 });
    // The first read after the end says nothing went through the proxy yet; the second arrives as an ordinary
    // summary event after the one that finished the scan.
    const summaries = events.flatMap((event) => (event.type === "scanUpdated" ? [event.summary] : []));
    expect(summaries.at(-1)).toMatchObject({ status: "completed", remote: { proxyBytes: 3 * 18_486_000 } });
    expect(summaries.at(-2)).toMatchObject({ status: "completed", remote: { proxyBytes: 0 } });
  });

  test("the user's own proxy is passed on as an external proxy", async () => {
    const api = await connected();
    await api.saveProxyProfile({
      name: "Belgium",
      scheme: "http",
      host: "be.proxy.example",
      port: 8080,
      username: null,
    });
    const [belgium] = (await api.getSettings()).proxies;
    const { detail } = await run(api, { ...onBrowserbase, proxy: { kind: "profile", id: belgium?.id ?? "" } });
    expect(detail.environment?.browser).toMatchObject({
      proxyKind: "external",
      appliedTimezone: "America/Los_Angeles",
    });
    expect(detail.environment?.proxy).toEqual({ kind: "profile", name: "Belgium", host: "be.proxy.example" });
  });

  test("a manual visit waits in the live view until it is finished, and the live view can be opened meanwhile", async () => {
    const api = await connected(createFakeBackend({ profile: "fresh", speed: 5000 }));
    const { finished } = await untilFinished(api);
    const summary = await api.startScan({ ...onBrowserbase, visitor: "manual" });
    await expect.poll(async () => remoteEvents(await api.getLogs(summary.id))).toContain("liveView");
    expect((await api.getScan(summary.id)).summary.status).toBe("running");
    await expect(api.openBrowserbaseLiveView(summary.id)).resolves.toBeUndefined();

    await api.finishCapture(summary.id);
    expect(await finished).toBe("completed");
    const detail = await api.getScan(summary.id);
    expect(detail.environment).toMatchObject({ visitor: "manual", webdriver: false, browser: { kind: "browserbase" } });
    await rejects(
      api.openBrowserbaseLiveView(summary.id),
      "invalidInput",
      "This scan has no Browserbase browser to show right now.",
    );
  });

  test("a scan on this Mac has no live view", async () => {
    const api = fresh();
    const { id } = await run(api, page);
    await rejects(api.openBrowserbaseLiveView(id), "invalidInput");
  });

  test("a cancelled visit is still billed for the time its browser ran", async () => {
    const api = await connected(createFakeBackend({ profile: "fresh", speed: 5000 }));
    const { finished } = await untilFinished(api);
    const summary = await api.startScan({ ...onBrowserbase, visitor: "manual" });
    await expect.poll(async () => remoteEvents(await api.getLogs(summary.id))).toContain("liveView");
    await api.cancelScan(summary.id);
    expect(await finished).toBe("cancelled");
    expect((await api.getScan(summary.id)).summary.remote).toMatchObject({ region: "us-west-2", proxyBytes: 0 });
  });
});

describe("simulated failures", () => {
  test("at the project's limit the capture waits, then fails with the backend's sentence and no browser", async () => {
    const api = await connected(fresh("limit"));
    const { detail, logs } = await run(api, onBrowserbase, "failed");
    expect(remoteEvents(logs)).toEqual(["rateLimited", "rateLimited", "rateLimited"]);
    expect(detail.summary.error).toBe(
      "Browserbase is at its limit of browsers running at once (3 of 3, shared by everything that uses this project). Try again when one has finished.",
    );
    expect(detail.steps.find((step) => step.node === "capture")?.status).toBe("failed");
    // No browser was started, so nothing was used.
    expect(detail.summary.remote).toBeNull();
    expect(detail.environment).toBeNull();
  });

  test("a lost connection ends the scan with nothing kept, and the browser's time is still recorded", async () => {
    const api = await connected(fresh("lost"));
    const { detail, logs } = await run(api, onBrowserbase, "failed");
    expect(remoteEvents(logs)).toEqual(["sessionCreated", "connected", "timezone", "connectionLost"]);
    expect(logs.find((entry) => entry.message === "The connection to the Browserbase browser was lost")?.level).toBe(
      "warn",
    );
    expect(detail.summary.error).toBe(
      "The connection to the Browserbase browser was lost. The browser has ended. Nothing was kept.",
    );
    expect(detail.findings).toEqual([]);
    expect(detail.summary.remote).toMatchObject({ region: "us-west-2" });
  });

  test("scans on this Mac are not affected", async () => {
    const api = await connected(fresh("limit"));
    expect((await run(api, { ...page, browser: { kind: "local" } })).detail.summary.status).toBe("completed");
  });
});

describe("seeded history", () => {
  const seeded = () => createFakeBackend({ profile: "seeded", speed: 5000 });

  test("is set up for Browserbase, with one scan and one configuration that ran there", async () => {
    const api = seeded();
    expect(await api.getSettings()).toMatchObject({
      defaultBrowser: "local",
      browserbase: { connected: true, projectName: "Local", concurrency: 3, region: "us-west-2" },
    });
    const scan = await api.getScan("seed-morningledger-browserbase");
    expect(scan.summary).toMatchObject({
      status: "completed",
      proxy: { kind: "browserbaseLocation", name: "BE" },
      // History is old: the count has long settled.
      remote: { region: "eu-central-1", proxyBytes: 18_486_000 },
    });
    expect(scan.environment?.browser).toMatchObject({
      kind: "browserbase",
      region: "eu-central-1",
      proxyCountry: "BE",
    });
    const configuration = (await api.listScanConfigurations()).find(
      (candidate) => candidate.id === "config-gardenweekly-browserbase",
    );
    expect(configuration).toMatchObject({
      name: "Garden Weekly from Belgium, Browserbase",
      lastRunAt: null,
      options: {
        url: "https://www.gardenweekly.example/spring-planting",
        browser: { kind: "browserbase", region: "eu-central-1" },
        proxy: { kind: "browserbaseLocation", country: "BE" },
      },
    });
    // Every other scan of the history ran on this Mac.
    const all = await api.listScans({ limit: 100 });
    expect(all.items.filter((summary) => summary.remote !== null).map((summary) => summary.id)).toEqual([
      "seed-morningledger-browserbase",
    ]);
  });

  test("a configuration that uses Browserbase runs, and cannot once the credentials are gone", async () => {
    const api = seeded();
    const { finished } = await untilFinished(api);
    const summary = await api.runScanConfiguration("config-gardenweekly-browserbase");
    expect(await finished).toBe("completed");
    expect((await api.getScan(summary.id)).options.browser).toEqual({ kind: "browserbase", region: "eu-central-1" });

    await api.clearBrowserbaseCredentials();
    await rejects(
      api.runScanConfiguration("config-gardenweekly-browserbase"),
      "browserbaseAuth",
      "Browserbase is not set up. Add the API key in Settings.",
    );
    // Saving one is validated like a scan start.
    await rejects(
      api.saveScanConfiguration({ name: "Another on Browserbase", options: onBrowserbase }),
      "browserbaseAuth",
    );
  });
});

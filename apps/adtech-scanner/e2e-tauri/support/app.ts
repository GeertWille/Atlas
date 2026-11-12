import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { connect, createServer } from "node:net";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { remote } from "webdriverio";
import type { ScanList } from "../../src/bindings";

export type Browser = Awaited<ReturnType<typeof remote>>;

// A copy `pnpm test:tauri` makes of the binary `tauri build` produced. Any
// other `cargo build` or `cargo test` rewrites target/debug/adtech-scanner
// without the frontend embedded; the copy is out of its way.
const APP_BINARY = resolve(import.meta.dirname, "../../target/e2e/adtech-scanner");
const DEFAULT_CHROME = "/opt/pw-browsers/chromium-1243/chrome-linux64/chrome";

export type App = {
  browser: Browser;
  close(): Promise<void>;
};

function freePort(): Promise<number> {
  return new Promise((done, fail) => {
    const server = createServer();
    server.once("error", fail);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      server.close(() => (address && typeof address === "object" ? done(address.port) : fail(new Error("no port"))));
    });
  });
}

async function accepting(port: number, process: ChildProcess): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (process.exitCode !== null) throw new Error(`tauri-driver exited with code ${process.exitCode}`);
    const open = await new Promise<boolean>((done) => {
      const socket = connect(port, "127.0.0.1");
      socket.once("connect", () => done(true)).once("error", () => done(false));
      socket.once("close", () => socket.destroy());
    });
    if (open) return;
    await new Promise((wait) => setTimeout(wait, 100));
  }
  throw new Error("tauri-driver did not start listening");
}

/** Whether a running process was started with `directory` in its environment (Linux `/proc`). */
function isUsedByAProcess(directory: string): boolean {
  return readdirSync("/proc")
    .filter((entry) => /^\d+$/.test(entry))
    .some((pid) => {
      try {
        return readFileSync(`/proc/${pid}/environ`, "utf8").includes(directory);
      } catch {
        return false;
      }
    });
}

function tauriDriverPath(): string {
  const installed = join(homedir(), ".cargo", "bin", "tauri-driver");
  return existsSync(installed) ? installed : "tauri-driver";
}

/**
 * Starts the built app (`pnpm tauri build --debug --no-bundle`) with empty
 * storage and connects to its webview through tauri-driver. The app inherits
 * `env`, which is how a spec points it at a mock of the Anthropic API.
 */
export async function launchApp(env: Record<string, string>): Promise<App> {
  if (!existsSync(APP_BINARY)) throw new Error(`${APP_BINARY} is missing; run \`pnpm test:tauri\`, which builds it`);
  const home = mkdtempSync(join(tmpdir(), "adscan-e2e-"));
  const [port, nativePort] = [await freePort(), await freePort()];
  const driver = spawn(tauriDriverPath(), ["--port", String(port), "--native-port", String(nativePort)], {
    env: {
      ...process.env,
      ADSCAN_DATA_DIR: join(home, "data"),
      // The webview keeps localStorage (theme, onboarding) under these, outside the app's data directory.
      XDG_DATA_HOME: join(home, "webview-data"),
      XDG_CACHE_HOME: join(home, "webview-cache"),
      ADSCAN_CHROME_PATH: process.env.ADSCAN_CHROME_PATH ?? DEFAULT_CHROME,
      ...env,
    },
    stdio: "ignore",
  });
  const stop = async () => {
    driver.kill();
    // The webview's helper processes outlive the window for a moment and still write their caches.
    for (let attempt = 0; attempt < 100 && isUsedByAProcess(home); attempt += 1) {
      await new Promise((wait) => setTimeout(wait, 100));
    }
    rmSync(home, { recursive: true, force: true });
  };
  try {
    await accepting(port, driver);
    // `tauri:options` is tauri-driver's own capability, unknown to WebdriverIO's types.
    const capabilities = {
      browserName: "wry",
      // WebKitWebDriver has no BiDi; without this the session is refused.
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP_BINARY },
    };
    const browser = await remote({ hostname: "127.0.0.1", port, logLevel: "error", capabilities });
    await browser.$(button("New scan")).waitForDisplayed({ timeout: 20_000 });
    return {
      browser,
      close: async () => {
        await cancelRunningScans(browser).catch(() => undefined);
        await browser.deleteSession().catch(() => undefined);
        await stop();
      },
    };
  } catch (error) {
    await stop();
    throw error;
  }
}

/**
 * Ending the WebDriver session kills the app outright, unlike quitting it, so
 * a scan a failed test left running would leave its browser behind.
 */
async function cancelRunningScans(browser: Browser) {
  const active = async () => {
    const lists = await Promise.all(
      ["running", "queued"].map((status) => invokeOk<ScanList>(browser, "list_scans", { query: { status } })),
    );
    return lists.flatMap((list) => list.items);
  };
  for (const scan of await active()) await invokeOk(browser, "cancel_scan", { scanId: scan.id });
  await browser.waitUntil(async () => (await active()).length === 0, { timeout: 15_000 });
}

// Not `error`: WebdriverIO takes a script result with that key for a WebDriver failure.
type CommandOutcome<T> = { ok: T } | { rejected: unknown };

/** Calls a backend command exactly as the webview does, without going through the app's adapter. */
export function invoke<T>(browser: Browser, command: string, args: Record<string, unknown> = {}) {
  return browser.execute(
    (name: string, payload: Record<string, unknown>) => {
      const internals = (
        window as unknown as { __TAURI_INTERNALS__: { invoke(name: string, payload: unknown): Promise<unknown> } }
      ).__TAURI_INTERNALS__;
      return internals.invoke(name, payload).then(
        (ok): CommandOutcome<unknown> => ({ ok }),
        (rejected): CommandOutcome<unknown> => ({ rejected }),
      );
    },
    command,
    args,
  ) as Promise<CommandOutcome<T>>;
}

export async function invokeOk<T>(browser: Browser, command: string, args: Record<string, unknown> = {}): Promise<T> {
  const outcome = await invoke<T>(browser, command, args);
  if ("rejected" in outcome) throw new Error(`${command} failed: ${JSON.stringify(outcome.rejected)}`);
  return outcome.ok;
}

const quoted = (text: string) => JSON.stringify(text);

/** A button by its visible text or accessible label; shortcut hints after the text are ignored. */
export const button = (name: string) =>
  `//button[@aria-label=${quoted(name)} or starts-with(normalize-space(.), ${quoted(name)})]`;
export const labelled = (label: string) => `//*[@aria-label=${quoted(label)}]`;
/** The input a visible `<label>` belongs to. */
export const field = (label: string) => `//input[@id=//label[normalize-space(.)=${quoted(label)}]/@for]`;
export const reportTab = (name: string) => `//*[@role="tab"][starts-with(normalize-space(.), ${quoted(name)})]`;
export const withRole = (role: string, name: string) =>
  `//*[@role=${quoted(role)}][@aria-label=${quoted(name)} or starts-with(normalize-space(.), ${quoted(name)})]`;

/** The rendered text of the first element matching the XPath, or `null` when nothing matches. */
export function textOf(browser: Browser, xpath: string): Promise<string | null> {
  return browser.execute((path: string) => {
    const node = document.evaluate(path, document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue;
    // biome-ignore lint/performance/useDomNodeTextContent: the tests assert what is rendered, and WebKitWebDriver's own text command drops line breaks and some headings.
    return node instanceof HTMLElement ? node.innerText : null;
  }, xpath);
}

/** Waits until the rendered text of `within` contains `expected`, and returns that text. */
export async function seeText(
  browser: Browser,
  expected: string | RegExp,
  { within = "//body", timeout = 10_000 }: { within?: string; timeout?: number } = {},
): Promise<string> {
  let text: string | null = null;
  await browser
    .waitUntil(
      async () => {
        text = await textOf(browser, within);
        if (text === null) return false;
        return typeof expected === "string" ? text.includes(expected) : expected.test(text);
      },
      { timeout, interval: 100, timeoutMsg: `"${expected}" did not appear in ${within}` },
    )
    .catch((error: Error) => {
      throw new Error(`${error.message}. It showed:\n${text}`);
    });
  return text ?? "";
}

export async function hashRoute(browser: Browser): Promise<string> {
  return browser.execute(() => window.location.hash.slice(1));
}

/** Goes to a route the way a deep link does. */
export async function goTo(browser: Browser, route: string) {
  await browser.execute((target: string) => {
    window.location.hash = target;
  }, route);
}

const PALETTE = '//*[@role="dialog"]';

export async function openCommandPalette(browser: Browser) {
  await browser.keys(["Control", "k"]);
  await browser.$(`${PALETTE}//input`).waitForDisplayed();
}

export async function closeCommandPalette(browser: Browser) {
  await browser.keys("Escape");
  // Its backdrop keeps covering the window until the exit animation ends, which software rendering makes slow.
  await browser.$(PALETTE).waitForExist({ reverse: true, timeout: 15_000 });
  await browser.$(".fixed.inset-0").waitForExist({ reverse: true, timeout: 15_000 });
}

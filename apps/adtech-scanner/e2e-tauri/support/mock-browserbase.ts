import { type ChildProcess, spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { tmpdir, userInfo } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";

export const MOCK_BROWSERBASE_KEY = "bb_live_e2e_key";
export const MOCK_BROWSERBASE_PROJECT = {
  id: "5d1c0a7e-9b2f-4c3d-8e6a-1f0b2c3d4e5f",
  name: "E2E project",
  concurrency: 3,
};
/** Names under it look public to the app and lead to this machine in the mock's browsers. */
export const FIXTURE_DOMAIN = "adscan-fixture.example";

const CHROME = process.env.ADSCAN_CHROME_PATH ?? "/opt/pw-browsers/chromium-1243/chrome-linux64/chrome";
/** What a session may be created with; Browserbase refuses any other top-level key. */
const SESSION_KEYS = ["projectId", "region", "timeout", "keepAlive", "browserSettings", "proxies", "userMetadata"];

export type SessionBody = {
  projectId: string;
  region: string;
  timeout: number;
  keepAlive: boolean;
  browserSettings: Record<string, unknown>;
  proxies?: unknown;
  userMetadata: Record<string, string>;
};

type Session = {
  id: string;
  body: SessionBody;
  browser: ChildProcess;
  profile: string;
  connectUrl: string;
  startedAt: number;
  endedAt: number | null;
};

export type MockBrowserbase = {
  baseUrl: string;
  /** Method and path of every request that carried the right key, in order. */
  requests: string[];
  /** The body of every session that was created, in order. */
  created: SessionBody[];
  /** Ids of the sessions whose browser is still running. */
  running(): string[];
  close(): Promise<void>;
};

function readBody(request: IncomingMessage): Promise<string> {
  return new Promise((done, fail) => {
    let body = "";
    request.setEncoding("utf8");
    request.on("data", (chunk) => {
      body += chunk;
    });
    request.on("end", () => done(body));
    request.on("error", fail);
  });
}

function send(response: ServerResponse, status: number, body?: unknown) {
  if (body === undefined) {
    response.writeHead(status).end();
    return;
  }
  response.writeHead(status, { "content-type": "application/json; charset=utf-8" });
  response.end(JSON.stringify(body));
}

function refuse(response: ServerResponse, statusCode: number, error: string, message: string) {
  send(response, statusCode, { statusCode, error, message });
}

/**
 * The browser behind one session: a local Chromium that listens for DevTools
 * clients on a WebSocket, started the way Browserbase starts its own as far
 * as a page can tell (no automation marker, no out-of-process iframes).
 */
async function startBrowser(): Promise<Pick<Session, "browser" | "profile" | "connectUrl">> {
  const profile = mkdtempSync(join(tmpdir(), "adscan-e2e-bb-"));
  const browser = spawn(
    CHROME,
    [
      "--headless=new",
      "--remote-debugging-port=0",
      "--remote-debugging-address=127.0.0.1",
      `--user-data-dir=${profile}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-dev-shm-usage",
      "--disable-background-networking",
      "--disable-component-update",
      "--disable-sync",
      "--disable-extensions",
      "--mute-audio",
      "--disable-blink-features=AutomationControlled",
      "--disable-site-isolation-trials",
      "--disable-features=IsolateOrigins,site-per-process,Translate,MediaRouter,OptimizationHints",
      "--window-size=2560,1440",
      `--host-resolver-rules=MAP *.${FIXTURE_DOMAIN} 127.0.0.1`,
      ...(userInfo().uid === 0 ? ["--no-sandbox"] : []),
      "about:blank",
    ],
    // Its own process group, so that its helper processes can be ended with it.
    { stdio: ["ignore", "ignore", "pipe"], detached: true },
  );
  // Chromium blocks when nobody reads what it prints, so the reader stays for the browser's life.
  const lines = createInterface({ input: browser.stderr as NodeJS.ReadableStream });
  const connectUrl = await new Promise<string>((done, fail) => {
    const timer = setTimeout(() => fail(new Error("the mock's browser did not start listening")), 20_000);
    lines.on("line", (line) => {
      const listening = line.match(/^DevTools listening on (ws:\/\/127\.0\.0\.1:\d+\/\S+)/);
      if (listening?.[1]) {
        clearTimeout(timer);
        done(listening[1]);
      }
    });
    browser.once("exit", () => {
      clearTimeout(timer);
      fail(new Error("the mock's browser exited before it listened"));
    });
  });
  return { browser, profile, connectUrl };
}

/**
 * A loopback stand-in for Browserbase's REST API that accepts only
 * `MOCK_BROWSERBASE_KEY`. Every session it creates is a local Chromium, and
 * its `connectUrl` is that browser's DevTools WebSocket: the routes and
 * shapes are those of `02-contract.md`, section 7, which the browser crate's
 * own mock implements as well.
 */
export async function startMockBrowserbase(): Promise<MockBrowserbase> {
  const requests: string[] = [];
  const created: SessionBody[] = [];
  const sessions = new Map<string, Session>();
  const contexts = new Set<string>();
  const timestamp = (ms: number) => new Date(ms).toISOString();
  const project = {
    ...MOCK_BROWSERBASE_PROJECT,
    ownerId: "owner_e2e",
    defaultTimeout: 1800,
    createdAt: "2026-11-01T10:00:00.000Z",
    updatedAt: "2026-11-01T10:04:00.000Z",
  };

  const end = (session: Session) => {
    if (session.endedAt !== null) return;
    session.endedAt = Date.now();
    try {
      // The whole group: a renderer that outlives the browser would keep writing into the profile.
      if (session.browser.pid !== undefined) process.kill(-session.browser.pid, "SIGKILL");
    } catch {
      // Already gone.
    }
  };

  const shown = (session: Session, withAddress: boolean) => ({
    id: session.id,
    projectId: project.id,
    status: session.endedAt === null ? "RUNNING" : "COMPLETED",
    region: session.body.region,
    keepAlive: session.body.keepAlive,
    proxyBytes: 0,
    createdAt: timestamp(session.startedAt),
    startedAt: timestamp(session.startedAt),
    updatedAt: timestamp(session.endedAt ?? session.startedAt),
    expiresAt: timestamp(session.startedAt + session.body.timeout * 1000),
    ...(session.endedAt === null ? {} : { endedAt: timestamp(session.endedAt) }),
    userMetadata: session.body.userMetadata,
    ...(withAddress
      ? { connectUrl: session.connectUrl, seleniumRemoteUrl: "http://127.0.0.1:9/wd/hub", signingKey: randomUUID() }
      : {}),
  });

  const createSession = async (response: ServerResponse, body: SessionBody) => {
    const unknown = Object.keys(body).filter((key) => !SESSION_KEYS.includes(key));
    if (unknown.length > 0) {
      const named = unknown.map((key) => `'${key}'`).join(", ");
      return refuse(response, 400, "Bad Request", `Validation error: Unrecognized key(s) in object: ${named}`);
    }
    if (body.projectId !== project.id) return refuse(response, 401, "Unauthorized", "Unauthorized Project ID");
    if (!(body.timeout >= 60 && body.timeout <= 21_600)) {
      return refuse(response, 400, "Bad Request", "body/timeout must be >= 60");
    }
    if ([...sessions.values()].filter((session) => session.endedAt === null).length >= project.concurrency) {
      return refuse(
        response,
        429,
        "Too Many Requests",
        `You've exceeded your max concurrent sessions limit (limit ${project.concurrency}, currently ${project.concurrency}). Please contact support to increase your limit.`,
      );
    }
    const session: Session = {
      id: randomUUID(),
      body,
      startedAt: Date.now(),
      endedAt: null,
      ...(await startBrowser()),
    };
    // Without keepAlive a session ends with its browser, however that goes. The profile goes once the
    // browser has: its helper processes write into it for a moment longer.
    session.browser.once("exit", () => {
      end(session);
      rmSync(session.profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
    });
    sessions.set(session.id, session);
    created.push(body);
    send(response, 201, shown(session, true));
  };

  const server = createServer(async (request, response) => {
    const url = new URL(request.url ?? "/", "http://mock");
    if (request.headers["x-bb-api-key"] !== MOCK_BROWSERBASE_KEY) {
      return refuse(response, 401, "Unauthorized", "Unauthorized");
    }
    requests.push(`${request.method} ${url.pathname}`);
    const [, version, resource, id, action] = url.pathname.split("/");
    if (version !== "v1") return refuse(response, 404, "Not Found", "Not Found");
    const text = request.method === "POST" ? await readBody(request) : "";
    const body = text ? JSON.parse(text) : {};

    if (resource === "projects") {
      if (!id) return send(response, 200, [project]);
      if (id !== project.id) return refuse(response, 404, "Not Found", "Project not found");
      return send(response, 200, action === "usage" ? { browserMinutes: 12, proxyBytes: 0 } : project);
    }
    if (resource === "contexts") {
      if (request.method === "POST") {
        const contextId = randomUUID();
        contexts.add(contextId);
        return send(response, 201, {
          id: contextId,
          uploadUrl: "http://127.0.0.1:9/upload",
          publicKey: "mock",
          cipherAlgorithm: "AES-256-CBC",
          initializationVectorSize: 16,
        });
      }
      if (!id || !contexts.has(id)) return refuse(response, 404, "Not Found", "Not Found");
      if (request.method === "DELETE") {
        contexts.delete(id);
        return send(response, 204);
      }
      return send(response, 200, { id, projectId: project.id, createdAt: timestamp(0), updatedAt: timestamp(0) });
    }
    if (resource === "sessions") {
      if (!id) {
        if (request.method === "POST") return createSession(response, body);
        const install = url.searchParams.get("q")?.match(/user_metadata\['install'\]:'([^']+)'/)?.[1];
        const listed = [...sessions.values()]
          .filter((session) => url.searchParams.get("status") !== "RUNNING" || session.endedAt === null)
          .filter((session) => install === undefined || session.body.userMetadata.install === install);
        return send(
          response,
          200,
          listed.map((session) => shown(session, false)),
        );
      }
      const session = sessions.get(id);
      if (!session) return refuse(response, 404, "Not Found", "Not Found");
      if (action === "debug") {
        if (session.endedAt !== null) return refuse(response, 410, "Gone", "Session stopped");
        const view = `http://127.0.0.1:9/devtools-fullscreen/inspector.html?session=${id}`;
        return send(response, 200, { debuggerFullscreenUrl: view, debuggerUrl: view, wsUrl: "", pages: [] });
      }
      if (request.method === "POST" && body.status === "REQUEST_RELEASE") end(session);
      return send(response, 200, shown(session, false));
    }
    return refuse(response, 404, "Not Found", "Not Found");
  });
  await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
  const address = server.address();
  if (!address || typeof address !== "object") throw new Error("the mock of Browserbase has no port");

  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    requests,
    created,
    running: () => [...sessions.values()].filter((session) => session.endedAt === null).map((session) => session.id),
    close: async () => {
      const gone = [...sessions.values()].map(
        (session) =>
          new Promise<void>((exited) => {
            if (session.browser.exitCode !== null || session.browser.signalCode !== null) exited();
            else session.browser.once("exit", () => exited());
            end(session);
          }),
      );
      await Promise.all(gone);
      await new Promise<void>((done) => {
        server.closeAllConnections();
        server.close(() => done());
      });
    },
  };
}

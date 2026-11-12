import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { UNDECIDED_SCRIPT } from "./publisher-page";

export const MOCK_API_KEY = "test";
export const AI_CLASSIFICATION_REASON = "Mock: decides when ad units load";
export const AI_FINDING_TITLE = "Scroll gate checks min-width, so desktop waits for a scroll before bidding";
/** Long enough for the live view to show a model turn before the next one lands. */
const MODEL_LATENCY_MS = 800;
export const AI_OVERVIEW = "Mock analysis: the wrapper applies its mobile scroll gate to desktop.";

type Message = { role: string; content: Array<{ type: string; text?: string; content?: string }> };
type MessagesRequest = { model: string; messages: Message[] };

export type MockAnthropic = {
  baseUrl: string;
  /** Bodies of every `/v1/messages` call, in order. */
  requests: MessagesRequest[];
  close(): Promise<void>;
};

function reply(content: unknown[], stopReason: string) {
  return { content, stop_reason: stopReason, usage: { input_tokens: 1500, output_tokens: 150 } };
}

function toolUse(id: string, name: string, input: unknown) {
  return { type: "tool_use", id, name, input };
}

/** Phase 1: the fixture page's undecided script is ad-related, anything else it is asked about is not. */
function classify(request: MessagesRequest) {
  const prompt = request.messages[0]?.content[0]?.text ?? "";
  const scripts = [...prompt.matchAll(/<script id="([^"]+)">\nurl: (\S+)/g)].map(([, id, url]) => {
    const wrapper = url?.includes(UNDECIDED_SCRIPT) ?? false;
    return {
      id,
      category: wrapper ? "wrapper" : "application",
      adRelated: wrapper,
      confidence: 0.9,
      reason: wrapper ? AI_CLASSIFICATION_REASON : "Mock: no ad logic in the excerpt",
    };
  });
  return reply([{ type: "text", text: JSON.stringify({ scripts }) }], "end_turn");
}

/** Phase 2: search for the planted device check, report it citing the line the search returned, finish. */
function analyze(request: MessagesRequest) {
  const turns = request.messages.length;
  if (turns === 1) {
    return reply(
      [
        { type: "text", text: "Mock: looking for the device check behind the scroll gate." },
        toolUse("toolu_search", "search_scripts", { pattern: "min-width", script_id: null }),
      ],
      "tool_use",
    );
  }
  if (turns === 3) {
    const searchResult = request.messages[2]?.content[0]?.content ?? "";
    const hit = /^--- (\S+) line (\d+)$/m.exec(searchResult);
    if (!hit) throw new Error(`the search for the planted bug found nothing:\n${searchResult}`);
    const [, scriptId, line] = hit;
    const source = new RegExp(`^\\s*${line}\\| (.*)$`, "m").exec(searchResult)?.[1];
    if (!source) throw new Error(`line ${line} is missing from the search result:\n${searchResult}`);
    return reply(
      [
        toolUse("toolu_report", "report_finding", {
          title: AI_FINDING_TITLE,
          severity: "critical",
          category: "logicBug",
          description: "`isMobileViewport()` tests `min-width` where `max-width` was meant.",
          recommendation: "Test `(max-width: 767px)` instead.",
          impact: "Desktop readers who do not scroll never trigger an auction",
          confidence: 0.9,
          estimated_revenue_impact_pct: 25,
          evidence: [
            { script_id: scriptId, line: Number(line), excerpt: source.trim(), explanation: "Inverted media query" },
          ],
          affected_slots: ["ad-leaderboard"],
          affected_bidders: [],
        }),
      ],
      "tool_use",
    );
  }
  return reply([toolUse("toolu_finish", "finish", { overview: AI_OVERVIEW })], "tool_use");
}

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

function send(response: ServerResponse, status: number, body: unknown) {
  response.writeHead(status, { "content-type": "application/json" });
  response.end(JSON.stringify(body));
}

/** A loopback stand-in for the Anthropic API that accepts only `MOCK_API_KEY`. */
export async function startMockAnthropic(): Promise<MockAnthropic> {
  const requests: MessagesRequest[] = [];
  const server = createServer(async (request, response) => {
    if (request.headers["x-api-key"] !== MOCK_API_KEY) {
      send(response, 401, { type: "error", error: { type: "authentication_error", message: "invalid x-api-key" } });
      return;
    }
    if (request.method === "GET" && request.url?.startsWith("/v1/models")) {
      send(response, 200, { data: [], has_more: false });
      return;
    }
    if (request.method === "POST" && request.url === "/v1/messages") {
      const body = JSON.parse(await readBody(request)) as MessagesRequest;
      requests.push(body);
      await new Promise((wait) => setTimeout(wait, MODEL_LATENCY_MS));
      try {
        send(response, 200, body.model.includes("haiku") ? classify(body) : analyze(body));
      } catch (error) {
        send(response, 400, { type: "error", error: { type: "invalid_request_error", message: String(error) } });
      }
      return;
    }
    send(response, 404, { type: "error", error: { type: "not_found_error", message: "not found" } });
  });
  await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
  const address = server.address();
  if (!address || typeof address !== "object") throw new Error("the mock API has no port");
  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    requests,
    close: () =>
      new Promise((done) => {
        server.closeAllConnections();
        server.close(() => done());
      }),
  };
}

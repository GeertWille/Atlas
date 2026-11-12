import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { UNDECIDED_SCRIPT } from "./publisher-page";

export const MOCK_TYPESAFE_KEY = "typesafe-test";
export const JEV_MODEL = "jev-1.14.0";
/** What the mock answers to "is this ad orchestration code" for the fixture page's script. */
export const JEV_AD_PROBABILITY = 0.93;

type Question = { type: string; instructions: string; criteria: Record<string, string> };
export type SystemOneRequest = {
  model: string;
  state: { script: { url: string; firstParty: boolean; lines?: string }; code: string };
  questions: Record<string, Question>;
};

export type MockTypesafe = {
  baseUrl: string;
  /** Bodies of every `/v1/systemone` call, in order, including the ones answered with a failure. */
  requests: SystemOneRequest[];
  /** From now on every `/v1/systemone` call is answered with this HTTP status instead of answers. */
  failWith(status: number): void;
  close(): Promise<void>;
};

/** The fixture page's script is the publisher's ad code; nothing else it is asked about is. */
function probability(request: SystemOneRequest, question: string): number {
  if (!request.state.script.url.includes(UNDECIDED_SCRIPT)) return 0.02;
  if (question === "ad_orchestration") return JEV_AD_PROBABILITY;
  return question === "publisher_specific" ? 0.8 : 0.6;
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

function send(response: ServerResponse, status: number, body: unknown, headers: Record<string, string> = {}) {
  response.writeHead(status, { "content-type": "application/json", ...headers });
  response.end(JSON.stringify(body));
}

/** A loopback stand-in for the TypeSafe API that accepts only `MOCK_TYPESAFE_KEY`. */
export async function startMockTypesafe(): Promise<MockTypesafe> {
  const requests: SystemOneRequest[] = [];
  let failure: number | null = null;
  const server = createServer(async (request, response) => {
    if (request.headers.authorization !== `Bearer ${MOCK_TYPESAFE_KEY}`) {
      send(response, 401, {
        detail: {
          error_type: "authentication_error",
          message: "Cannot authenticate with the server. Please check your API key and try again.",
        },
      });
      return;
    }
    if (request.method === "GET" && request.url === "/v1/models") {
      send(response, 200, {
        models: [{ name: "jev-latest", description: "General-purpose system one model.", release_date: "2026-09-15" }],
      });
      return;
    }
    if (request.method === "POST" && request.url === "/v1/systemone") {
      const body = JSON.parse(await readBody(request)) as SystemOneRequest;
      requests.push(body);
      if (failure !== null) {
        send(response, failure, { detail: "mock failure" }, { "retry-after": "0" });
        return;
      }
      const answers = Object.fromEntries(
        Object.keys(body.questions).map((id) => [id, { type: "noul", noul: probability(body, id) }]),
      );
      send(response, 200, {
        model: JEV_MODEL,
        answers,
        usage: { input_tokens: Math.ceil(JSON.stringify(body).length / 3), output_tokens: 4 },
      });
      return;
    }
    send(response, 404, { detail: "Not Found" });
  });
  await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
  const address = server.address();
  if (!address || typeof address !== "object") throw new Error("the mock API has no port");
  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    requests,
    failWith: (status) => {
      failure = status;
    },
    close: () =>
      new Promise((done) => {
        server.closeAllConnections();
        server.close(() => done());
      }),
  };
}

import type { NetworkRequestDetail } from "@/bindings";

/** Headers curl works out for itself, or that would make the copy fail when sent as they were captured. */
const SKIPPED_HEADERS = new Set(["host", "content-length", "accept-encoding", "connection"]);

const REDACTED_PLACEHOLDER = "<redacted: not stored>";
const MISSING_BODY_PLACEHOLDER = "<request body not stored>";

export type CurlCommand = {
  command: string;
  /** Names of the headers whose values were not stored: the copy carries a placeholder for each. */
  redactedHeaders: string[];
  /** The request had a body that is not, or not wholly, in the copy. */
  bodyIncomplete: boolean;
};

/** Single-quoted for a POSIX shell: nothing inside is expanded, and a quote ends, escapes and reopens the string. */
function quote(text: string): string {
  return `'${text.replaceAll("'", "'\\''")}'`;
}

/**
 * The request as a curl command. A redacted header keeps its name and gets a placeholder value, so
 * the command shows what was sent without claiming a credential that was never stored.
 * `requestBody`: the stored body text, or `null` when it could not be read.
 */
export function toCurl(detail: NetworkRequestDetail, requestBody: string | null): CurlCommand {
  const { row } = detail;
  const hasBody = detail.requestBody.state === "kept" || detail.requestBody.state === "truncated";
  const expectsBody = hasBody || detail.requestBody.bytes > 0;
  const parts = [`curl ${quote(row.url)}`];
  // curl sends GET by default, and POST once it has data.
  if (row.method !== "GET" && !(row.method === "POST" && expectsBody)) parts.push(`-X ${row.method}`);

  const redactedHeaders: string[] = [];
  let compressed = false;
  for (const header of detail.requestHeaders) {
    const name = header.name.toLowerCase();
    if (name.startsWith(":")) continue;
    if (name === "accept-encoding") compressed = true;
    if (SKIPPED_HEADERS.has(name)) continue;
    if (header.redaction !== null) redactedHeaders.push(header.name);
    parts.push(`-H ${quote(`${header.name}: ${header.redaction === null ? header.value : REDACTED_PLACEHOLDER}`)}`);
  }

  const bodyIncomplete = expectsBody && (requestBody === null || detail.requestBody.state === "truncated");
  if (expectsBody) parts.push(`--data-raw ${quote(requestBody ?? MISSING_BODY_PLACEHOLDER)}`);
  if (compressed) parts.push("--compressed");
  return { command: parts.join(" \\\n  "), redactedHeaders, bodyIncomplete };
}

/**
 * Whether a copy of this request carries identifiers of the visitor: bid requests do (consent string,
 * user ids), and so does anything whose cookies or credentials were stored in full.
 */
export function carriesIdentifiers(detail: NetworkRequestDetail): boolean {
  const fullCredentials = detail.requestHeaders.some(
    (header) => header.redaction === null && /^(cookie|authorization|proxy-authorization)$/i.test(header.name),
  );
  return detail.row.category === "bid" || fullCredentials;
}

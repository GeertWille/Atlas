export type UrlResult = { ok: true; url: string } | { ok: false; error: string };

const LOCAL_HOST = /^(localhost|\d{1,3}(\.\d{1,3}){3}|\[[0-9a-f:]+\])(:\d+)?(\/|$)/i;

/**
 * Turns what a person types ("example.com/news") into the absolute http(s) URL
 * the backend requires, or explains why it cannot.
 */
export function normalizeScanUrl(input: string): UrlResult {
  const trimmed = input.trim();
  if (!trimmed) return { ok: false, error: "Enter the address of a page to scan." };
  if (/\s/.test(trimmed)) return { ok: false, error: "A URL cannot contain spaces." };

  const hasScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed);
  if (hasScheme && !/^https?:\/\//i.test(trimmed)) {
    return { ok: false, error: "Only http and https pages can be scanned." };
  }
  // Local addresses rarely serve TLS; everything else defaults to https.
  const candidate = hasScheme ? trimmed : `${LOCAL_HOST.test(trimmed) ? "http" : "https"}://${trimmed}`;

  let parsed: URL;
  try {
    parsed = new URL(candidate);
  } catch {
    return { ok: false, error: "That does not look like a valid URL." };
  }
  const isLocal = LOCAL_HOST.test(parsed.host);
  if (!isLocal && !/^[a-z0-9-]+(\.[a-z0-9-]+)+$/i.test(parsed.hostname)) {
    return { ok: false, error: "Enter a full domain, like example.com." };
  }
  parsed.hash = "";
  return { ok: true, url: parsed.toString() };
}

import type { LogEntry, LogSearchHit } from "@/bindings";
import type { StoredScan } from "./runtime";

const SNIPPET_RADIUS = 70;

function escapeHtml(text: string): string {
  return text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

function searchableText(entry: LogEntry): string {
  return entry.data == null ? entry.message : `${entry.message} ${JSON.stringify(entry.data)}`;
}

/** Mirrors the backend contract: escaped text, matches wrapped in `<mark>`. */
function snippetFor(text: string, terms: string[]): string {
  const lower = text.toLowerCase();
  const first = Math.min(...terms.map((term) => lower.indexOf(term)).filter((at) => at >= 0));
  const start = Math.max(0, first - SNIPPET_RADIUS);
  const end = Math.min(text.length, first + SNIPPET_RADIUS * 2);
  const excerpt = text.slice(start, end);
  const pattern = new RegExp(`(${terms.map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`, "gi");
  const marked = excerpt
    .split(pattern)
    .map((part, index) => (index % 2 === 1 ? `<mark>${escapeHtml(part)}</mark>` : escapeHtml(part)))
    .join("");
  return `${start > 0 ? "…" : ""}${marked}${end < text.length ? "…" : ""}`;
}

export function searchLogs(scans: StoredScan[], query: string, limit: number): LogSearchHit[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return [];
  const hits: LogSearchHit[] = [];
  for (const scan of scans) {
    for (const entry of scan.logs) {
      const text = searchableText(entry);
      const lower = text.toLowerCase();
      if (!terms.every((term) => lower.includes(term))) continue;
      hits.push({ entry, scanUrl: scan.detail.summary.url, snippet: snippetFor(text, terms) });
      if (hits.length >= limit) return hits;
    }
  }
  return hits;
}

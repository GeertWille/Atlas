const usd2 = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});
const usd3 = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 3,
  maximumFractionDigits: 3,
});
const integer = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });
const compact = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });
const dateTime = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});
const clock = new Intl.DateTimeFormat("en-US", {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hour12: false,
});

export const EMPTY = "–";

/** LLM spend: amounts under ten cents keep a third decimal so sub-cent steps stay visible. */
export function formatUsd(amount: number): string {
  if (amount > 0 && amount < 0.001) return "<$0.001";
  return amount > 0 && amount < 0.1 ? usd3.format(amount) : usd2.format(amount);
}

export function formatCpm(cpm: number | null, currency: string | null = "USD"): string {
  if (cpm === null) return EMPTY;
  const value = cpm.toFixed(2);
  return !currency || currency === "USD" ? `$${value}` : `${value} ${currency}`;
}

export function formatInteger(value: number): string {
  return integer.format(value);
}

export function formatCompact(value: number): string {
  return value < 1000 ? integer.format(value) : compact.format(value);
}

/** 0-1 ratio as a whole percentage. */
export function formatPercent(ratio: number): string {
  return `${Math.round(ratio * 100)}%`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  // Checked on the rounded value, so 1,048,575 bytes reads "1.0 MB" and never "1024 KB".
  const kilobytes = bytes / 1024;
  if (Math.round(kilobytes) < 1024) return `${kilobytes.toFixed(kilobytes < 10 ? 1 : 0)} KB`;
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

/** A span of time: "840 ms", "1.24 s", "2m 05s". */
export function formatDuration(ms: number | null): string {
  if (ms === null) return EMPTY;
  // Each unit is chosen from the rounded value, so a boundary never prints as "1000 ms" or "60.0 s".
  if (Math.round(ms) < 1000) return `${Math.round(ms)} ms`;
  if (ms < 9995) return `${(ms / 1000).toFixed(2)} s`;
  if (ms < 59_950) return `${(ms / 1000).toFixed(1)} s`;
  const totalSeconds = Math.round(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
}

/** A position on the page timeline (ms since navigation start), always with ms precision. */
export function formatOffset(ms: number | null): string {
  return ms === null ? EMPTY : `${integer.format(Math.round(ms))} ms`;
}

export function formatDateTime(epochMs: number): string {
  return dateTime.format(epochMs);
}

export function formatClock(epochMs: number): string {
  const millis = String(Math.floor(epochMs % 1000)).padStart(3, "0");
  return `${clock.format(epochMs)}.${millis}`;
}

export function formatRelativeTime(epochMs: number, now: number = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - epochMs) / 1000));
  if (seconds < 45) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  if (days === 1) return "yesterday";
  if (days < 30) return `${days} days ago`;
  return formatDateTime(epochMs);
}

export function pluralize(count: number, singular: string, plural = `${singular}s`): string {
  return `${formatInteger(count)} ${count === 1 ? singular : plural}`;
}

/** Size of a filtered list: "12 scripts", or "3 of 12 scripts" while a filter hides some. */
export function countOf(shown: number, total: number, singular: string, plural?: string): string {
  const all = pluralize(total, singular, plural);
  return shown === total ? all : `${formatInteger(shown)} of ${all}`;
}

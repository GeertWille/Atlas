const usd2 = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 2 });
const usd3 = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 3, maximumFractionDigits: 3 });
const integer = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });
const compact = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });
const dateTime = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
const clock = new Intl.DateTimeFormat("en-US", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });

export const EMPTY = "–";

/** LLM spend: sub-dollar amounts keep a third decimal so cents-level steps stay visible. */
export function formatUsd(amount: number): string {
  if (amount === 0) return "$0.00";
  return amount < 1 ? usd3.format(amount) : usd2.format(amount);
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
  if (bytes < 1_048_576) return `${(bytes / 1024).toFixed(bytes < 10_240 ? 1 : 0)} KB`;
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

/** A span of time: "840 ms", "1.24 s", "2m 05s". */
export function formatDuration(ms: number | null): string {
  if (ms === null) return EMPTY;
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.floor((ms % 60_000) / 1000);
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

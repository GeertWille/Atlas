import type { NetworkCategory, ResourceKind } from "@/bindings";
import { CATEGORIES, CATEGORY_META } from "../labels";

export const QUERY_KEYS = [
  "host",
  "url",
  "path",
  "method",
  "status",
  "type",
  "cat",
  "vendor",
  "bidder",
  "unit",
  "page",
  "auction",
  "initiator",
  "mime",
  "larger-than",
  "slower-than",
  "is",
] as const;
export type QueryKey = (typeof QUERY_KEYS)[number];

export function isQueryKey(key: string): key is QueryKey {
  return (QUERY_KEYS as readonly string[]).includes(key);
}

export const IS_VALUES = [
  "failed",
  "blocked",
  "cancelled",
  "pending",
  "slow",
  "bid",
  "late",
  "nobid",
  "won",
  "cached",
  "sw",
  "redirect",
  "post",
  "first-party",
  "third-party",
  "before-auction",
  "no-hb-targeting",
] as const;
export type IsValue = (typeof IS_VALUES)[number];

/** `type:` accepts the names of the Type column and a few everyday spellings. `xhr` covers fetch, as in DevTools. */
export const TYPE_VALUES: Record<string, readonly ResourceKind[]> = {
  doc: ["document"],
  document: ["document"],
  script: ["script"],
  js: ["script"],
  css: ["stylesheet"],
  stylesheet: ["stylesheet"],
  img: ["image"],
  image: ["image"],
  media: ["media"],
  font: ["font"],
  xhr: ["xhr", "fetch"],
  fetch: ["fetch"],
  ping: ["ping"],
  ws: ["webSocket"],
  websocket: ["webSocket"],
  sse: ["eventSource"],
  preflight: ["preflight"],
  other: ["other"],
};

export const CATEGORY_VALUES: Record<string, NetworkCategory> = Object.fromEntries(
  CATEGORIES.map((category) => [CATEGORY_META[category].token, category]),
);

/** `50k`, `1.5m`, `300` (bytes). */
export function parseSize(value: string): number | null {
  const match = /^(\d+(?:\.\d+)?)\s*(b|k|kb|m|mb)?$/i.exec(value);
  if (!match) return null;
  const unit = (match[2] ?? "b").toLowerCase();
  const factor = unit.startsWith("m") ? 1024 * 1024 : unit.startsWith("k") ? 1024 : 1;
  return Number(match[1]) * factor;
}

/** `500ms`, `2s`, `800` (ms). */
export function parseDuration(value: string): number | null {
  const match = /^(\d+(?:\.\d+)?)\s*(ms|s)?$/i.exec(value);
  if (!match) return null;
  return Number(match[1]) * ((match[2] ?? "ms").toLowerCase() === "s" ? 1000 : 1);
}

/** `204` is that status; `4xx` every status from 400 to 499. */
export function parseStatus(value: string): { min: number; max: number } | null {
  if (/^[1-5]xx$/i.test(value)) {
    const hundred = Number(value[0]) * 100;
    return { min: hundred, max: hundred + 99 };
  }
  if (/^\d{3}$/.test(value)) return { min: Number(value), max: Number(value) };
  return null;
}

/** What is wrong with a value for a key, or `null` when it can be used. */
export function valueProblem(key: QueryKey, value: string): string | null {
  const lower = value.toLowerCase();
  switch (key) {
    case "is":
      return (IS_VALUES as readonly string[]).includes(lower) ? null : `Unknown state “${value}”`;
    case "type":
      return lower in TYPE_VALUES ? null : `Unknown type “${value}”`;
    case "cat":
      return lower in CATEGORY_VALUES ? null : `Unknown category “${value}”`;
    case "status":
      return parseStatus(value) ? null : `“${value}” is not a status: use 204 or 4xx`;
    case "page":
      return /^[1-9]\d*$/.test(value) ? null : `“${value}” is not a page number`;
    case "larger-than":
      return parseSize(value) === null ? `“${value}” is not a size: use 50k or 1.5m` : null;
    case "slower-than":
      return parseDuration(value) === null ? `“${value}” is not a duration: use 500ms or 2s` : null;
    default:
      return null;
  }
}

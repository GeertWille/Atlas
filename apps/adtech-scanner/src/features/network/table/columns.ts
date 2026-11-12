import type { NetworkRow } from "@/bindings";
import { KIND_LABEL } from "../labels";

export const COLUMN_KEYS = [
  "status",
  "name",
  "vendor",
  "outcome",
  "type",
  "initiator",
  "size",
  "time",
  "method",
  "host",
  "page",
  "start",
  "auction",
  "bidder",
  "decoded",
  "mime",
  "waterfall",
] as const;
export type ColumnKey = (typeof COLUMN_KEYS)[number];

export type ColumnDef = {
  key: ColumnKey;
  label: string;
  /** CSS grid track. */
  track: string;
  align: "left" | "right";
  /** Shown unless the user turned it off. */
  byDefault: boolean;
  /** A default column that only shows when the list is at least this wide. */
  fromWidth?: number;
  /** Stays when the detail panel is docked beside the list and room is short. */
  beside?: boolean;
  /** Always shown: not in the column chooser. */
  fixed?: boolean;
};

export const COLUMNS: readonly ColumnDef[] = [
  { key: "status", label: "Status", track: "72px", align: "left", byDefault: true, beside: true, fixed: true },
  {
    key: "name",
    label: "Name",
    track: "minmax(168px, 2fr)",
    align: "left",
    byDefault: true,
    beside: true,
    fixed: true,
  },
  { key: "vendor", label: "Vendor", track: "124px", align: "left", byDefault: true },
  { key: "outcome", label: "Ad outcome", track: "108px", align: "left", byDefault: true, beside: true },
  { key: "type", label: "Type", track: "56px", align: "left", byDefault: true, fromWidth: 900 },
  { key: "initiator", label: "Initiator", track: "128px", align: "left", byDefault: true, fromWidth: 1000 },
  { key: "method", label: "Method", track: "64px", align: "left", byDefault: false },
  { key: "host", label: "Host", track: "168px", align: "left", byDefault: false },
  { key: "page", label: "Page", track: "48px", align: "right", byDefault: false },
  { key: "auction", label: "Auction", track: "64px", align: "left", byDefault: false },
  { key: "bidder", label: "Bidder", track: "96px", align: "left", byDefault: false },
  { key: "mime", label: "MIME type", track: "144px", align: "left", byDefault: false },
  { key: "start", label: "Start", track: "72px", align: "right", byDefault: false },
  { key: "size", label: "Size", track: "76px", align: "right", byDefault: true, fromWidth: 900 },
  { key: "decoded", label: "Decoded", track: "76px", align: "right", byDefault: false },
  { key: "time", label: "Time", track: "64px", align: "right", byDefault: true, beside: true },
  { key: "waterfall", label: "Waterfall", track: "minmax(120px, 3fr)", align: "left", byDefault: true },
];

/** The user's own choices: only the columns they switched on or off are in here. */
export type ColumnChoices = Partial<Record<ColumnKey, boolean>>;

export function isColumnChoices(value: unknown): value is ColumnChoices {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  return Object.entries(value).every(
    ([key, on]) => (COLUMN_KEYS as readonly string[]).includes(key) && typeof on === "boolean",
  );
}

/**
 * The columns to draw. A choice the user made always wins; otherwise a column shows by default when
 * the list is wide enough for it. With the detail panel beside the list, only the essential ones stay.
 */
export function visibleColumns(input: {
  choices: ColumnChoices;
  listWidth: number;
  detailBeside: boolean;
}): ColumnDef[] {
  const { choices, listWidth, detailBeside } = input;
  return COLUMNS.filter((column) => {
    if (column.fixed) return true;
    if (detailBeside) return column.beside === true && choices[column.key] !== false;
    const chosen = choices[column.key];
    if (chosen !== undefined) return chosen;
    return column.byDefault && listWidth >= (column.fromWidth ?? 0);
  });
}

export function gridTemplate(columns: ColumnDef[]): string {
  return columns.map((column) => column.track).join(" ");
}

export type SortState = { key: ColumnKey; descending: boolean };

/** `start` ascending is the order the requests were made in, and the default. */
export const DEFAULT_SORT: SortState = { key: "start", descending: false };

export function parseSort(value: string | undefined): SortState {
  if (!value) return DEFAULT_SORT;
  const descending = value.startsWith("-");
  const key = descending ? value.slice(1) : value;
  return (COLUMN_KEYS as readonly string[]).includes(key) && key !== "waterfall"
    ? { key: key as ColumnKey, descending }
    : DEFAULT_SORT;
}

/** The URL form of a sort; `undefined` for the default, which needs no parameter. */
export function formatSort(sort: SortState): string | undefined {
  if (sort.key === DEFAULT_SORT.key && !sort.descending) return undefined;
  return `${sort.descending ? "-" : ""}${sort.key}`;
}

/** Clicking a header sorts by it; clicking it again reverses; a third click goes back to request order. */
export function nextSort(current: SortState, key: ColumnKey): SortState {
  if (current.key !== key) return { key, descending: false };
  if (!current.descending) return { key, descending: true };
  return DEFAULT_SORT;
}

/** The file part of a URL and its query, shown apart: the name in full weight, the query dimmed. */
export function splitName(url: string): { name: string; query: string } {
  try {
    const parsed = new URL(url);
    const segment = parsed.pathname.split("/").filter(Boolean).pop();
    return { name: segment ?? parsed.host, query: parsed.search };
  } catch {
    return { name: url, query: "" };
  }
}

const text = (a: string | null, b: string | null) => (a ?? "").localeCompare(b ?? "");
const number = (a: number | null, b: number | null) => (a ?? -1) - (b ?? -1);

const COMPARE: Record<Exclude<ColumnKey, "waterfall">, (a: NetworkRow, b: NetworkRow) => number> = {
  start: (a, b) => a.page - b.page || a.startedAtMs - b.startedAtMs || a.seq - b.seq,
  status: (a, b) => number(a.status, b.status) || text(a.outcome, b.outcome),
  name: (a, b) => text(splitName(a.url).name, splitName(b.url).name),
  vendor: (a, b) => text(a.vendor ?? a.host, b.vendor ?? b.host),
  outcome: (a, b) => text(a.ad?.verdict ?? null, b.ad?.verdict ?? null) || number(a.ad?.cpm ?? null, b.ad?.cpm ?? null),
  type: (a, b) => text(KIND_LABEL[a.kind], KIND_LABEL[b.kind]),
  initiator: (a, b) => text(a.initiator?.url ?? null, b.initiator?.url ?? null),
  size: (a, b) => a.transferredBytes - b.transferredBytes,
  time: (a, b) => number(a.durationMs, b.durationMs),
  method: (a, b) => text(a.method, b.method),
  host: (a, b) => text(a.host, b.host),
  page: (a, b) => a.page - b.page,
  auction: (a, b) => text(a.ad?.auctionId ?? null, b.ad?.auctionId ?? null),
  bidder: (a, b) => text(a.bidder, b.bidder),
  decoded: (a, b) => a.decodedBytes - b.decodedBytes,
  mime: (a, b) => text(a.mime, b.mime),
};

/** Orders two rows; equal ones keep the order the requests were made in, whichever way the sort runs. */
export function compareRows(sort: SortState): (a: NetworkRow, b: NetworkRow) => number {
  const compare = sort.key === "waterfall" ? COMPARE.start : COMPARE[sort.key];
  const direction = sort.descending ? -1 : 1;
  return (a, b) => compare(a, b) * direction || COMPARE.start(a, b);
}

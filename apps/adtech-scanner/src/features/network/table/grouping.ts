import type { PageView } from "@/bindings";
import { PAGE_REACH_LABEL, pagePath } from "@/shared/domain/page-views";
import { KIND_LABEL } from "../labels";
import type { IndexedRow } from "../query/match";
import type { NetworkGrouping } from "../search";
import { compareRows, type SortState } from "./columns";

export type GridGroup = {
  /** Stable across filters: `page:0`, `vendor:Google`, `type:xhr`. */
  key: string;
  label: string;
  /** Extra words after the label: how a page view was reached. */
  note: string | null;
  count: number;
  transferredBytes: number;
  failed: number;
  /** First start and last end of its requests on the page's clock; `null` end when one never finished. */
  startMs: number;
  endMs: number | null;
  collapsed: boolean;
};

export type GridItem = { kind: "group"; group: GridGroup } | { kind: "row"; indexed: IndexedRow };

/** The registrable part of a host, near enough for a label: the last two labels. */
export function siteOf(host: string): string {
  return host.split(".").slice(-2).join(".");
}

function groupKey(indexed: IndexedRow, grouping: Exclude<NetworkGrouping, "none">): string {
  const { row } = indexed;
  if (grouping === "page") return `page:${row.page}`;
  if (grouping === "vendor") return `vendor:${row.vendor ?? (row.firstParty ? "Site" : siteOf(row.host))}`;
  return `type:${KIND_LABEL[row.kind]}`;
}

function describeGroup(key: string, pages: PageView[]): Pick<GridGroup, "label" | "note"> {
  const value = key.slice(key.indexOf(":") + 1);
  if (!key.startsWith("page:")) return { label: value, note: null };
  const page = pages.find((candidate) => candidate.index === Number(value));
  if (!page) return { label: `Page ${Number(value) + 1}`, note: null };
  // A page reached by an in-page navigation loads no document: its group says so.
  const note =
    page.reachedBy === "routeChange"
      ? `${PAGE_REACH_LABEL[page.reachedBy].toLowerCase()}, no document request`
      : PAGE_REACH_LABEL[page.reachedBy].toLowerCase();
  return { label: `Page ${page.index + 1}  ${pagePath(page.url)}`, note };
}

/**
 * The list as it is drawn: rows sorted, and under group headings when grouped. Sorting applies inside
 * a group. Page groups keep the order of the visit; vendor and type groups put the largest first.
 */
export function buildGrid(
  rows: IndexedRow[],
  options: { grouping: NetworkGrouping; sort: SortState; collapsed: ReadonlySet<string>; pages: PageView[] },
): GridItem[] {
  const { grouping, sort, collapsed, pages } = options;
  const compare = compareRows(sort);
  const sorted = [...rows].sort((a, b) => compare(a.row, b.row));
  if (grouping === "none") return sorted.map((indexed) => ({ kind: "row", indexed }));

  const members = new Map<string, IndexedRow[]>();
  for (const indexed of sorted) {
    const key = groupKey(indexed, grouping);
    const group = members.get(key);
    if (group) group.push(indexed);
    else members.set(key, [indexed]);
  }
  const keys = [...members.keys()].sort((a, b) =>
    grouping === "page"
      ? Number(a.slice(5)) - Number(b.slice(5))
      : (members.get(b)?.length ?? 0) - (members.get(a)?.length ?? 0) || a.localeCompare(b),
  );
  return keys.flatMap((key): GridItem[] => {
    const groupRows = members.get(key) ?? [];
    const isCollapsed = collapsed.has(key);
    const unfinished = groupRows.some((indexed) => indexed.endMs === null);
    const group: GridGroup = {
      key,
      ...describeGroup(key, pages),
      count: groupRows.length,
      transferredBytes: groupRows.reduce((sum, { row }) => sum + row.transferredBytes, 0),
      failed: groupRows.filter(({ row }) => row.outcome === "failed" || row.outcome === "httpError").length,
      startMs: Math.min(...groupRows.map(({ row }) => row.startedAtMs)),
      endMs: unfinished ? null : Math.max(...groupRows.map((indexed) => indexed.endMs ?? 0)),
      collapsed: isCollapsed,
    };
    return [
      { kind: "group", group },
      ...(isCollapsed ? [] : groupRows.map((indexed): GridItem => ({ kind: "row", indexed }))),
    ];
  });
}

/** Above this many rows, page groups other than the one being looked at start folded. */
const AUTO_COLLAPSE_ROWS = 5000;

export function initiallyCollapsed(rowCount: number, pages: PageView[], shownPage: number | undefined): Set<string> {
  if (rowCount <= AUTO_COLLAPSE_ROWS) return new Set();
  const keep = shownPage ?? pages[0]?.index ?? 0;
  return new Set(pages.filter((page) => page.index !== keep).map((page) => `page:${page.index}`));
}

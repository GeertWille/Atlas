import {
  BanIcon,
  CheckIcon,
  CircleAlertIcon,
  CircleDollarSignIcon,
  CircleSlashIcon,
  CornerDownRightIcon,
  FileTextIcon,
  HourglassIcon,
  type LucideIcon,
  MinusIcon,
  TimerIcon,
  TimerOffIcon,
  TriangleAlertIcon,
  XCircleIcon,
} from "lucide-react";
import type { NetworkRow, RequestOutcome } from "@/bindings";
import { cn } from "@/shared/lib/cn";
import { EMPTY, formatBytes, formatDuration } from "@/shared/lib/format";
import { fileNameOf } from "@/shared/lib/url";
import { type OutcomeKind, rowOutcome } from "../ad-outcome";
import { KIND_LABEL, OUTCOME_LABEL, SERVED_FROM_NOTE } from "../labels";
import { type ColumnKey, splitName } from "./columns";
import { siteOf } from "./grouping";

/** A request that did not simply succeed gets an icon and a word: never colour alone. */
const OUTCOME_MARK: Record<Exclude<RequestOutcome, "ok">, { icon: LucideIcon; word: string; tone: string }> = {
  httpError: { icon: CircleAlertIcon, word: "", tone: "text-red-fg" },
  failed: { icon: XCircleIcon, word: "failed", tone: "text-red-fg" },
  blocked: { icon: BanIcon, word: "blocked", tone: "text-orange-fg" },
  cancelled: { icon: CircleSlashIcon, word: "cancel", tone: "text-muted-foreground" },
  pending: { icon: HourglassIcon, word: "pending", tone: "text-muted-foreground" },
};

export function StatusMark({ row }: { row: Pick<NetworkRow, "status" | "outcome"> }) {
  if (row.outcome === "ok") {
    const redirect = row.status !== null && row.status >= 300 && row.status < 400;
    return <span className={cn("tabular-nums", redirect && "text-muted-foreground")}>{row.status ?? EMPTY}</span>;
  }
  const mark = OUTCOME_MARK[row.outcome];
  const Icon = mark.icon;
  return (
    <span className={cn("flex min-w-0 items-center gap-1 tabular-nums", mark.tone)}>
      <Icon role="img" aria-label={OUTCOME_LABEL[row.outcome]} className="size-3.5 shrink-0" />
      <span className="truncate">{row.outcome === "httpError" ? row.status : mark.word}</span>
    </span>
  );
}

export const OUTCOME_STYLE: Record<OutcomeKind, { icon: LucideIcon; tone: string; label: string }> = {
  bid: { icon: CircleDollarSignIcon, tone: "text-blue-fg", label: "Bid" },
  noBid: { icon: MinusIcon, tone: "text-muted-foreground", label: "No bid" },
  late: { icon: TriangleAlertIcon, tone: "text-amber-fg", label: "Late" },
  atDeadline: { icon: TimerIcon, tone: "text-amber-fg", label: "At the deadline" },
  timedOutInPage: { icon: TimerOffIcon, tone: "text-amber-fg", label: "Timed out in the page" },
  noAnswer: { icon: HourglassIcon, tone: "text-muted-foreground", label: "No answer" },
  failed: { icon: XCircleIcon, tone: "text-red-fg", label: "Failed" },
  hbKeys: { icon: CheckIcon, tone: "text-muted-foreground", label: "With Prebid keys" },
  noHbKeys: { icon: CircleSlashIcon, tone: "text-orange-fg", label: "Without Prebid keys" },
};

export function AdOutcome({ row }: { row: NetworkRow }) {
  const outcome = rowOutcome(row);
  if (!outcome) return null;
  const style = OUTCOME_STYLE[outcome.kind];
  const Icon = style.icon;
  return (
    <span className={cn("flex min-w-0 items-center gap-1 tabular-nums", style.tone)}>
      <Icon role="img" aria-label={style.label} className="size-3.5 shrink-0" />
      <span className="truncate">{outcome.text}</span>
    </span>
  );
}

function NameCell({ row }: { row: NetworkRow }) {
  const { name, query } = splitName(row.url);
  const navigation = row.kind === "document" && row.context === "page";
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      {row.redirectOf ? (
        <CornerDownRightIcon
          role="img"
          aria-label="Follows a redirect"
          className="size-3.5 shrink-0 text-subtle-foreground"
        />
      ) : navigation ? (
        <FileTextIcon role="img" aria-label="Page navigation" className="size-3.5 shrink-0 text-subtle-foreground" />
      ) : null}
      <span className="min-w-0 truncate">
        <span className={navigation ? "font-semibold" : "font-medium"}>{name}</span>
        <span className="text-subtle-foreground">{query}</span>
      </span>
    </span>
  );
}

export function vendorLabel(row: Pick<NetworkRow, "vendor" | "firstParty" | "host">): string {
  return row.vendor ?? (row.firstParty ? "Site" : siteOf(row.host));
}

/** What one cell shows. The waterfall is drawn by the grid, which knows the time axis. */
export function CellContent({ column, row }: { column: Exclude<ColumnKey, "waterfall">; row: NetworkRow }) {
  switch (column) {
    case "status":
      return <StatusMark row={row} />;
    case "name":
      return <NameCell row={row} />;
    case "vendor":
      return <span className={cn("truncate", !row.vendor && "text-muted-foreground")}>{vendorLabel(row)}</span>;
    case "outcome":
      return <AdOutcome row={row} />;
    case "type":
      return <span className="truncate text-muted-foreground">{KIND_LABEL[row.kind]}</span>;
    case "initiator":
      return (
        <span className="truncate text-muted-foreground">
          {row.initiator?.url && row.initiator.kind === "script"
            ? fileNameOf(row.initiator.url)
            : (row.initiator?.kind ?? EMPTY)}
        </span>
      );
    case "size":
      return (
        <span className={cn("truncate tabular-nums", row.servedFrom !== "network" && "text-muted-foreground")}>
          {SERVED_FROM_NOTE[row.servedFrom] ?? formatBytes(row.transferredBytes)}
        </span>
      );
    case "decoded":
      return <span className="truncate tabular-nums">{formatBytes(row.decodedBytes)}</span>;
    case "time":
      return <span className="truncate tabular-nums">{formatDuration(row.durationMs)}</span>;
    case "start":
      return <span className="truncate tabular-nums">{formatDuration(row.startedAtMs)}</span>;
    case "method":
      return <span className="truncate text-muted-foreground">{row.method}</span>;
    case "host":
      return <span className="truncate text-muted-foreground">{row.host}</span>;
    case "page":
      return <span className="tabular-nums text-muted-foreground">{row.page + 1}</span>;
    case "auction":
      return <span className="truncate font-mono text-[11px] text-muted-foreground">{row.ad?.auctionId ?? ""}</span>;
    case "bidder":
      return <span className="truncate">{row.bidder ?? (row.ad?.viaServer ? "Prebid Server" : "")}</span>;
    case "mime":
      return <span className="truncate text-muted-foreground">{row.mime ?? ""}</span>;
  }
}

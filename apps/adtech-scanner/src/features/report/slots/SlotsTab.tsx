import {
  CircleCheckIcon,
  CircleDashedIcon,
  CircleSlashIcon,
  HourglassIcon,
  LayoutPanelTopIcon,
  type LucideIcon,
  TriangleAlertIcon,
} from "lucide-react";
import type { AdSlot, PageView, SlotOutcome } from "@/bindings";
import { Panel } from "@/shared/components/panel";
import { SLOT_OUTCOME_META } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { formatDuration, formatOffset, pluralize } from "@/shared/lib/format";
import { Badge } from "@/shared/ui/badge";
import { Empty } from "@/shared/ui/empty";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { PageSectionHeader } from "../pages/PageSectionHeader";
import { type PageMarkers, pageMarkers } from "../pages/page-scope";
import {
  layoutSlotTimeline,
  SLOT_PHASE_LABEL,
  SLOT_PHASES,
  type SlotPhase,
  type SlotRow,
  type SlotTimeline,
} from "./slot-timeline";

const GRID = "grid grid-cols-[300px_minmax(0,1fr)]";
const PLOT_INSET = "left-[300px] right-5";

const OUTCOME_ICON: Record<SlotOutcome, LucideIcon> = {
  filled: CircleCheckIcon,
  empty: CircleSlashIcon,
  neverRequested: CircleDashedIcon,
  pending: HourglassIcon,
};

/** Each phase has its own shape as well as colour, so the sequence reads without colour. */
const PHASE_MARK: Record<SlotPhase, string> = {
  defined: "size-2.5 rounded-full border-2 border-gray bg-card",
  requested: "size-2.5 rounded-full bg-blue",
  response: "size-2.5 rotate-45 rounded-[2px] bg-violet",
  rendered: "size-2.5 rounded-[2px] bg-green",
  viewable: "size-3 rounded-full border-[3px] border-green bg-card",
};

function PhaseMark({ phase, className }: { phase: SlotPhase; className?: string }) {
  return <span className={cn("block ring-2 ring-card", PHASE_MARK[phase], className)} />;
}

/** A refresh is a smaller, hollow "ad requested" mark; a refresh that re-used a bid is amber. */
function RefreshMark({ staleBid, className }: { staleBid: boolean; className?: string }) {
  return (
    <span
      className={cn(
        "block size-2 rounded-full border-2 bg-card ring-2 ring-card",
        staleBid ? "border-amber" : "border-blue",
        className,
      )}
    />
  );
}

function SlotTooltip({ row }: { row: SlotRow }) {
  const { slot, marks, refreshes } = row;
  return (
    <div className="flex flex-col gap-1.5">
      <p className="font-mono font-medium">{slot.elementId}</p>
      {marks.length === 0 ? <p className="text-muted-foreground">No lifecycle events recorded.</p> : null}
      <dl className="grid grid-cols-[auto_auto_auto] items-center gap-x-3 gap-y-0.5 tabular-nums">
        {marks.map((mark, index) => {
          const previous = marks[index - 1];
          return (
            <div key={mark.phase} className="contents">
              <dt className="flex items-center gap-1.5 text-muted-foreground">
                <PhaseMark phase={mark.phase} className="ring-0" />
                {SLOT_PHASE_LABEL[mark.phase]}
              </dt>
              <dd className="text-right">{formatOffset(mark.atMs)}</dd>
              <dd className="text-right text-muted-foreground">
                {previous ? `+${formatDuration(mark.atMs - previous.atMs)}` : ""}
              </dd>
            </div>
          );
        })}
      </dl>
      {refreshes.length > 0 ? (
        <ul className="flex flex-col gap-0.5 tabular-nums">
          {refreshes.map((refresh) => (
            <li key={refresh.atMs} className="flex items-center gap-1.5 text-muted-foreground">
              <RefreshMark staleBid={refresh.staleBid} className="ring-0" />
              Refresh at {formatOffset(refresh.atMs)}
              {refresh.staleBid
                ? ", same hb_adid as before"
                : refresh.hadPrebidTargeting
                  ? ", with Prebid targeting"
                  : ", no Prebid targeting"}
            </li>
          ))}
        </ul>
      ) : null}
      {slot.renderedSize ? (
        <p className="text-muted-foreground">
          Rendered {slot.renderedSize}
          {slot.lineItemId ? `, line item ${slot.lineItemId}` : ""}
        </p>
      ) : null}
      <p className="text-muted-foreground">{SLOT_OUTCOME_META[slot.outcome].description}</p>
    </div>
  );
}

function SlotLabel({ slot, flags }: Pick<SlotRow, "slot" | "flags">) {
  const outcome = SLOT_OUTCOME_META[slot.outcome];
  const OutcomeIcon = OUTCOME_ICON[slot.outcome];
  return (
    <div className="flex min-w-0 flex-col gap-1.5 py-3 pr-4 pl-4">
      <div className="flex items-center justify-between gap-2">
        <span className="selectable truncate font-mono text-xs font-medium">{slot.elementId}</span>
        <Badge tone={outcome.tone}>
          <OutcomeIcon />
          {outcome.label}
        </Badge>
      </div>
      <span className="selectable truncate text-xs text-subtle-foreground">{slot.adUnitPath ?? "No ad unit path"}</span>
      <div className="flex flex-wrap items-center gap-1">
        <span className="mr-1 text-xs text-subtle-foreground tabular-nums">{slot.sizes.join(", ")}</span>
        {flags.map((flag) => (
          <Badge key={flag.key} tone={flag.warning ? "amber" : "outline"} title={flag.description}>
            {flag.warning ? <TriangleAlertIcon /> : null}
            {flag.label}
          </Badge>
        ))}
      </div>
    </div>
  );
}

function SlotTrack({ row }: { row: SlotRow }) {
  return (
    <div className="relative h-full min-h-14">
      {row.unfinishedFromPercent !== null ? (
        <>
          <span
            className="absolute top-1/2 right-0 h-0 -translate-y-1/2 border-t-2 border-dashed border-border-strong"
            style={{ left: `${row.unfinishedFromPercent}%` }}
          />
          <span
            className="absolute top-1/2 ml-3 -translate-y-[150%] rounded-sm bg-card px-1 text-[11px] whitespace-nowrap text-muted-foreground"
            style={{ left: `${row.unfinishedFromPercent}%` }}
          >
            {row.slot.outcome === "neverRequested"
              ? "no ad request during observation"
              : "no response during observation"}
          </span>
        </>
      ) : null}
      {row.span ? (
        <span
          className="absolute top-1/2 h-0.5 -translate-y-1/2 rounded-full bg-gray"
          style={{ left: `${row.span.startPercent}%`, width: `${row.span.widthPercent}%` }}
        />
      ) : null}
      {row.marks.map((mark) => (
        <span
          key={mark.phase}
          className="absolute top-1/2 -translate-x-1/2 -translate-y-1/2"
          style={{ left: `${mark.percent}%` }}
        >
          <PhaseMark phase={mark.phase} />
        </span>
      ))}
      {row.refreshes.map((refresh) => (
        <span
          key={refresh.atMs}
          className="absolute top-1/2 -translate-x-1/2 -translate-y-1/2"
          style={{ left: `${refresh.percent}%` }}
        >
          <RefreshMark staleBid={refresh.staleBid} />
        </span>
      ))}
    </div>
  );
}

function Axis({ timeline }: { timeline: SlotTimeline }) {
  return (
    <div aria-hidden="true" className={cn("pointer-events-none absolute inset-y-0", PLOT_INSET)}>
      {timeline.scale.ticks.map((tick, index) => (
        <div key={tick} className="absolute inset-y-0" style={{ left: `${timeline.scale.toPercent(tick)}%` }}>
          <span className="absolute top-14 bottom-0 w-px bg-border" />
          <span
            className={cn(
              // Above the page-lifecycle lines, which would otherwise strike through the tick labels.
              "absolute top-9 z-10 bg-card px-0.5 text-[11px] whitespace-nowrap text-subtle-foreground tabular-nums",
              index === timeline.scale.ticks.length - 1 ? "-translate-x-full" : "-translate-x-1/2",
            )}
          >
            {tick === 0 ? "0" : formatDuration(tick)}
          </span>
        </div>
      ))}
      {timeline.markers.map((marker) => (
        <div key={marker.label} className="absolute inset-y-0" style={{ left: `${marker.percent}%` }}>
          <span className="absolute top-1 bottom-0 w-px bg-violet/60" />
          <span
            className={cn(
              "absolute z-10 rounded-sm bg-card px-1 text-[11px] font-medium whitespace-nowrap text-violet-fg",
              marker.labelSide === "before" ? "right-1" : "left-1",
              marker.labelRow === 0 ? "top-1" : "top-[18px]",
            )}
          >
            {marker.label} {formatDuration(marker.atMs)}
          </span>
        </div>
      ))}
    </div>
  );
}

function Legend({ refreshes }: { refreshes: boolean }) {
  return (
    <ul
      aria-label="Lifecycle legend"
      className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs text-muted-foreground"
    >
      {SLOT_PHASES.map((phase) => (
        <li key={phase} className="flex items-center gap-1.5">
          <PhaseMark phase={phase} className="ring-0" />
          {SLOT_PHASE_LABEL[phase]}
        </li>
      ))}
      {refreshes ? (
        <li className="flex items-center gap-1.5">
          <RefreshMark staleBid={false} className="ring-0" />
          Refresh request
        </li>
      ) : null}
      <li className="flex items-center gap-1.5">
        <span className="h-3 w-px bg-violet" />
        Page lifecycle
      </li>
    </ul>
  );
}

function SlotTimelinePanel({ slots, markers }: { slots: AdSlot[]; markers: PageMarkers }) {
  const timeline = layoutSlotTimeline(slots, markers);
  return (
    <Panel className="relative overflow-hidden pt-14">
      <Axis timeline={timeline} />
      <ul className="relative">
        {timeline.rows.map((row) => (
          <li key={row.slot.elementId} className="border-t">
            <Tooltip>
              <TooltipTrigger asChild>
                <div
                  // biome-ignore lint/a11y/noNoninteractiveTabindex: rows expose exact timings on focus
                  tabIndex={0}
                  className={cn(GRID, "items-stretch pr-5 outline-offset-[-2px] hover:bg-accent/50")}
                >
                  <SlotLabel slot={row.slot} flags={row.flags} />
                  <SlotTrack row={row} />
                </div>
              </TooltipTrigger>
              <TooltipContent side="top" align="end">
                <SlotTooltip row={row} />
              </TooltipContent>
            </Tooltip>
          </li>
        ))}
      </ul>
    </Panel>
  );
}

/** The slots of one page view under its heading; a page whose slots were never defined says so. */
function PageSlots({ page, slots }: { page: PageView; slots: AdSlot[] }) {
  return (
    <section aria-label={`Ad slots on page ${page.index + 1}`} className="flex flex-col gap-3">
      <PageSectionHeader page={page} summary={pluralize(slots.length, "slot")} />
      {slots.length === 0 ? (
        <p className="rounded-xl border border-dashed px-4 py-3 text-[13px] text-muted-foreground">
          GPT defined no ad slot on this page view.
        </p>
      ) : (
        <SlotTimelinePanel slots={slots} markers={pageMarkers(page)} />
      )}
    </section>
  );
}

export function SlotsTab({
  slots,
  metrics,
  pages = [],
}: {
  slots: AdSlot[];
  /** Page lifecycle times of the page the slots are on. */
  metrics: PageMarkers;
  /** The page views to group the slots under, when a multi-page visit is shown whole. */
  pages?: PageView[];
}) {
  if (slots.length === 0) {
    return (
      <Empty
        icon={LayoutPanelTopIcon}
        title="No ad slots captured"
        description="GPT did not define any slots while the page was observed."
      />
    );
  }
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <p className="text-[13px] text-muted-foreground">
          Every slot from definition to viewable impression, in ms since navigation start.
        </p>
        <Legend refreshes={slots.some((slot) => slot.requests.length > 1)} />
      </div>
      {pages.length > 0 ? (
        pages.map((page) => (
          <PageSlots key={page.index} page={page} slots={slots.filter((slot) => slot.page === page.index)} />
        ))
      ) : (
        <SlotTimelinePanel slots={slots} markers={metrics} />
      )}
    </div>
  );
}

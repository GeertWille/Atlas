import { CircleAlertIcon, InfoIcon, type LucideIcon } from "lucide-react";
import type { BidderDiagnostics, DiagnosticArea, DiagnosticCheck } from "@/bindings";
import { Panel, PanelHeader } from "@/shared/components/panel";
import { cn } from "@/shared/lib/cn";
import { useReportLinks } from "../report-links";
import {
  describeAdsTxt,
  describeBids,
  describeConsent,
  describeRemainder,
  describeSellersJson,
  type FlaggedBidder,
  groupBidders,
} from "./bidder-checks";

const AREA_LABEL: Record<DiagnosticArea, string> = {
  params: "Params",
  response: "Endpoint",
  adsTxt: "ads.txt",
  sellersJson: "sellers.json",
  consent: "Consent",
};

const FLAG_STYLE: Record<"problem" | "notice", { icon: LucideIcon; label: string; tone: string }> = {
  problem: { icon: CircleAlertIcon, label: "Problem", tone: "text-red-fg" },
  notice: { icon: InfoIcon, label: "Notice", tone: "text-amber-fg" },
};

function Fact({ term, children }: { term: string; children: string }) {
  return (
    <div className="flex gap-3">
      <dt className="w-24 shrink-0 text-muted-foreground">{term}</dt>
      <dd className="selectable min-w-0 break-words">{children}</dd>
    </div>
  );
}

function CheckLine({ check }: { check: DiagnosticCheck }) {
  const style = FLAG_STYLE[check.status === "problem" ? "problem" : "notice"];
  const Icon = style.icon;
  const openRequest = useReportLinks()?.openRequest;
  const [requestId] = check.requestIds;
  return (
    <li className="flex gap-2">
      <Icon role="img" aria-label={style.label} className={cn("mt-0.5 size-3.5 shrink-0", style.tone)} />
      <div className="flex min-w-0 flex-col gap-1">
        <p className="selectable">
          <span className="text-muted-foreground">{AREA_LABEL[check.area]}: </span>
          {check.summary}
        </p>
        {check.excerpt ? (
          <p className="selectable rounded-md bg-code px-2 py-1 font-mono text-xs break-words">{check.excerpt}</p>
        ) : null}
        {check.adUnits.length > 0 ? (
          <p className="selectable font-mono text-xs text-muted-foreground">{check.adUnits.join(", ")}</p>
        ) : null}
        {requestId && openRequest ? (
          <button
            type="button"
            onClick={() => openRequest(requestId, "response")}
            className="w-fit rounded-sm text-xs text-primary-text underline-offset-4 hover:underline"
          >
            {check.requestIds.length === 1
              ? "View the response"
              : `View the response (first of ${check.requestIds.length})`}
          </button>
        ) : null}
      </div>
    </li>
  );
}

function BidderBlock({ bidder }: { bidder: FlaggedBidder }) {
  const remainder = describeRemainder(bidder);
  return (
    <li className="flex flex-col gap-2 px-4 py-3" aria-label={bidder.bidder}>
      <div className="flex items-baseline justify-between gap-3">
        <h3 className="font-medium">{bidder.bidder}</h3>
        <span className="text-xs text-muted-foreground tabular-nums">{describeBids(bidder)}</span>
      </div>
      <ul className="flex flex-col gap-2">
        {bidder.flagged.map((check) => (
          <CheckLine key={`${check.code}:${check.summary}`} check={check} />
        ))}
      </ul>
      {remainder ? <p className="text-xs text-muted-foreground">{remainder}</p> : null}
    </li>
  );
}

/** What the scanner verified about each bidder on its own. Only what needs attention is spelled out. */
export function BidderChecks({ diagnostics }: { diagnostics: BidderDiagnostics }) {
  const { flagged, clear } = groupBidders(diagnostics.bidders);
  return (
    <Panel className="overflow-hidden text-[13px]">
      <PanelHeader
        title="Bidder checks"
        description="Adapter params, endpoint answers, ads.txt, sellers.json and consent, verified by the scanner"
      />
      <dl className="flex flex-col gap-1.5 border-b px-4 py-3">
        <Fact term="ads.txt">
          {diagnostics.adsTxt.url
            ? `${diagnostics.adsTxt.url}: ${describeAdsTxt(diagnostics.adsTxt)}`
            : describeAdsTxt(diagnostics.adsTxt)}
        </Fact>
        <Fact term="sellers.json">{describeSellersJson(diagnostics)}</Fact>
        <Fact term="Consent">{describeConsent(diagnostics.consent)}</Fact>
      </dl>
      {flagged.length > 0 ? (
        <ul aria-label="Bidders that need attention" className="divide-y">
          {flagged.map((bidder) => (
            <BidderBlock key={bidder.bidder} bidder={bidder} />
          ))}
        </ul>
      ) : null}
      {clear.length > 0 ? (
        <p className={cn("px-4 py-3 text-muted-foreground", flagged.length > 0 && "border-t")}>
          Nothing found for <span className="selectable text-foreground">{clear.join(", ")}</span>.
        </p>
      ) : null}
    </Panel>
  );
}

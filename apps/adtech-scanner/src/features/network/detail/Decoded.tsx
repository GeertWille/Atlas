import type { ReactNode } from "react";
import type { DecodedBidRequest, DecodedBidResponse } from "@/bindings";
import { EMPTY, formatBytes, formatDuration } from "@/shared/lib/format";
import { revealInvisibles } from "@/shared/lib/invisible";

/** What a decoded body says is the exchange's or the advertiser's text: shown as it is, invisible characters included. */
const shown = (value: ReactNode) => (typeof value === "string" ? revealInvisibles(value) : value);

function Facts({ items }: { items: [label: string, value: ReactNode][] }) {
  return (
    <dl className="flex flex-wrap gap-x-5 gap-y-1 px-3 py-2 text-xs">
      {items.map(([label, value]) => (
        <div key={label} className="flex gap-1.5">
          <dt className="text-muted-foreground">{label}</dt>
          <dd className="selectable tabular-nums">{shown(value)}</dd>
        </div>
      ))}
    </dl>
  );
}

function Table({ label, head, rows }: { label: string; head: string[]; rows: ReactNode[][] }) {
  return (
    <div className="overflow-x-auto border-t">
      <table aria-label={label} className="w-full min-w-max border-collapse text-left text-xs">
        <thead>
          <tr className="text-[11px] text-muted-foreground">
            {head.map((cell) => (
              <th key={cell} scope="col" className="border-b px-3 py-1.5 font-medium whitespace-nowrap">
                {cell}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, rowIndex) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: rows of a decoded body never reorder
            <tr key={rowIndex} className="border-b border-border/60 last:border-b-0">
              {row.map((cell, cellIndex) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: cells follow the fixed column order
                <td key={cellIndex} className="selectable px-3 py-1.5 align-top whitespace-nowrap tabular-nums">
                  {shown(cell)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

const yesNo = (value: boolean | null) => (value === null ? "not stated" : value ? "yes" : "no");

/** A bid request read as OpenRTB. Consent and identity are shown as presence only: the strings are in the raw body. */
export function DecodedRequest({ request }: { request: DecodedBidRequest }) {
  const schain = request.schain;
  return (
    <section aria-label="Decoded bid request" className="flex flex-col">
      <Facts
        items={[
          ["Request id", request.id ?? EMPTY],
          ["tmax", request.tmaxMs === null ? "not set" : formatDuration(request.tmaxMs)],
          ["Site", request.siteDomain ?? EMPTY],
          ["Currencies", request.currencies.join(", ") || EMPTY],
          ["Test", request.test ? "yes" : "no"],
        ]}
      />
      <Table
        label="Impressions"
        head={[
          "Imp",
          "Ad unit",
          "Media",
          "Sizes",
          "Floor",
          ...(request.imps.some((imp) => imp.bidders.length > 0) ? ["Bidders"] : []),
        ]}
        rows={request.imps.map((imp) => [
          <span key="id" className="font-mono">
            {revealInvisibles(imp.id)}
          </span>,
          imp.adUnit ?? imp.tagId ?? EMPTY,
          imp.mediaTypes.join(", ") || EMPTY,
          imp.sizes.join(", ") || EMPTY,
          imp.floor === null ? "none" : `${imp.floor.toFixed(2)} ${imp.floorCurrency ?? ""}`.trim(),
          ...(request.imps.some((candidate) => candidate.bidders.length > 0) ? [imp.bidders.join(", ")] : []),
        ])}
      />
      <div className="border-t">
        <Facts
          items={[
            ["GDPR applies", yesNo(request.gdpr)],
            ["Consent string", request.consentStringPresent ? "present" : "absent"],
            ["US Privacy", request.usPrivacyPresent ? "present" : "absent"],
            ["GPP", request.gppPresent ? "present" : "absent"],
            ["COPPA", yesNo(request.coppa)],
            ["User ids from", request.eidSources.join(", ") || "none"],
          ]}
        />
        <p className="px-3 pb-2 text-[11px] text-muted-foreground">
          The consent string and the ids themselves are not shown here. They are in the raw body.
        </p>
      </div>
      {schain ? (
        <Table
          label="Supply chain"
          head={[
            `Supply chain (${schain.complete === null ? "completeness not stated" : schain.complete ? "complete" : "incomplete"})`,
            "Seller id",
            "Handles payment",
          ]}
          rows={schain.nodes.map((node) => [
            node.asi,
            <span key="sid" className="font-mono">
              {revealInvisibles(node.sid)}
            </span>,
            yesNo(node.hp),
          ])}
        />
      ) : (
        <p className="border-t px-3 py-2 text-xs text-muted-foreground">No supply chain (schain) in this request.</p>
      )}
    </section>
  );
}

export function DecodedResponse({ response }: { response: DecodedBidResponse }) {
  const bids = response.seats.flatMap((seat) => seat.bids.map((bid) => ({ seat: seat.seat, bid })));
  return (
    <section aria-label="Decoded bid response" className="flex flex-col">
      <Facts
        items={[
          ["Response id", response.id ?? EMPTY],
          ["Currency", response.currency ?? EMPTY],
          ["Bids", String(bids.length)],
        ]}
      />
      {response.nbr !== null ? (
        <p className="border-t px-3 py-2 text-xs">
          <span className="font-medium">No bid, reason {response.nbr}</span>
          {response.nbrMeaning ? `: ${response.nbrMeaning}` : ""}
          <span className="text-muted-foreground"> (OpenRTB no-bid reason code)</span>
        </p>
      ) : bids.length === 0 ? (
        <p className="border-t px-3 py-2 text-xs text-muted-foreground">
          The answer carries no bids and gives no reason.
        </p>
      ) : (
        <Table
          label="Bids"
          head={["Seat", "Imp (ad unit)", "Price", "Size", "Advertiser", "Creative", "Deal", "Markup"]}
          rows={bids.map(({ seat, bid }) => [
            seat ?? EMPTY,
            <span key="imp">
              <span className="font-mono">{revealInvisibles(bid.impId)}</span>
              {bid.adUnit ? revealInvisibles(` (${bid.adUnit})`) : ""}
            </span>,
            bid.price.toFixed(2),
            bid.size ?? EMPTY,
            bid.advertiserDomains.join(", ") || EMPTY,
            bid.creativeId ?? EMPTY,
            bid.dealId ?? EMPTY,
            bid.markupBytes === null ? EMPTY : formatBytes(bid.markupBytes),
          ])}
        />
      )}
      {response.serverTimings.length > 0 || response.serverErrors.length > 0 ? (
        <Table
          label="Prebid Server, per bidder"
          head={["Prebid Server bidder", "Server-side time", "Error"]}
          rows={[
            ...new Set([
              ...response.serverTimings.map((timing) => timing.bidder),
              ...response.serverErrors.map((error) => error.bidder),
            ]),
          ].map((bidder) => {
            const timing = response.serverTimings.find((candidate) => candidate.bidder === bidder);
            const error = response.serverErrors.find((candidate) => candidate.bidder === bidder);
            return [
              bidder,
              timing ? formatDuration(timing.ms) : EMPTY,
              error ? `${error.message}${error.code === null ? "" : ` (code ${error.code})`}` : EMPTY,
            ];
          })}
        />
      ) : null}
    </section>
  );
}

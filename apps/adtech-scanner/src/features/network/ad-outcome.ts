import type { NetworkRow } from "@/bindings";
import { verdictShort } from "@/shared/domain/exchange-verdict";
import { formatCpm } from "@/shared/lib/format";

export type OutcomeKind =
  | "bid"
  | "noBid"
  | "late"
  | "atDeadline"
  | "timedOutInPage"
  | "noAnswer"
  | "failed"
  | "hbKeys"
  | "noHbKeys";

export type RowOutcome = { kind: OutcomeKind; text: string };

/**
 * What a request meant for the ad stack, in a few characters: the column no generic network tool has.
 * `null` for everything that is neither a matched bid call nor an ad-server call.
 */
export function rowOutcome(row: Pick<NetworkRow, "category" | "ad" | "noHbTargeting">): RowOutcome | null {
  if (row.category === "adServer") {
    return row.noHbTargeting ? { kind: "noHbKeys", text: "no hb keys" } : { kind: "hbKeys", text: "hb keys" };
  }
  if (row.category !== "bid" || !row.ad) return null;
  const { ad } = row;
  switch (ad.verdict) {
    case "failed":
      return { kind: "failed", text: "failed" };
    case "neverAnswered":
      return { kind: "noAnswer", text: "no answer" };
    case "lateOnNetwork":
      return { kind: "late", text: verdictShort(ad) };
    case "atDeadline":
      return { kind: "atDeadline", text: "at deadline" };
    case "inTimeButTimedOut":
      return { kind: "timedOutInPage", text: "timed out in page" };
    case "notSent":
      return null;
    case "inTime":
      if (ad.cpm !== null) return { kind: "bid", text: formatCpm(ad.cpm) };
      return { kind: "noBid", text: ad.nbr === null ? "no bid" : `no-bid ${ad.nbr}` };
  }
}

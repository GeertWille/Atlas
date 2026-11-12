import type { ExchangeVerdict } from "@/bindings";
import { formatDuration } from "@/shared/lib/format";

/** What the network says about one bid, as the backend records it on the bid and on the request. */
export type ExchangeFacts = {
  verdict: ExchangeVerdict;
  /** ms the answer arrived after the auction closed; negative when early. */
  lateByMs: number | null;
  /** From the request being sent to its answer being complete. */
  networkMs: number | null;
  /** Network error or blocked reason of a failed request. */
  error?: string | null;
};

const ms = (value: number) => formatDuration(Math.abs(value));

/**
 * One sentence on how a bid request's answer relates to its auction. The page's clock and the
 * network's can disagree by a few milliseconds: inside that tolerance the backend reports
 * `atDeadline`, and the sentence then gives no number.
 */
export function verdictSentence({ verdict, lateByMs, networkMs, error }: ExchangeFacts): string {
  switch (verdict) {
    case "inTime": {
      const answered = networkMs === null ? "Answered" : `Answered in ${ms(networkMs)}`;
      return lateByMs === null
        ? `${answered}, before the auction closed`
        : `${answered}, ${ms(lateByMs)} before the auction closed`;
    }
    case "atDeadline":
      return "The response arrived as the auction closed";
    case "lateOnNetwork":
      return lateByMs === null
        ? "Response arrived after the auction closed"
        : `Response arrived after the auction closed (+${ms(lateByMs)})`;
    case "inTimeButTimedOut": {
      const arrived =
        lateByMs === null
          ? "The response arrived before the timeout"
          : `The response arrived ${ms(lateByMs)} before the timeout`;
      return `${arrived}, but Prebid recorded a timeout: the page was busy or the adapter was slow to parse`;
    }
    case "neverAnswered":
      return "No response before the visit ended";
    case "failed":
      return error ? `The request failed: ${error}` : "The request failed";
    case "notSent":
      return "Prebid announced this bid but no request left the page";
  }
}

/** The short form for table cells and bars: "late +84 ms", "in time", "not sent". */
export function verdictShort({ verdict, lateByMs }: Pick<ExchangeFacts, "verdict" | "lateByMs">): string {
  switch (verdict) {
    case "inTime":
      return "in time";
    case "atDeadline":
      return "at deadline";
    case "lateOnNetwork":
      return lateByMs === null ? "late" : `late +${ms(lateByMs)}`;
    case "inTimeButTimedOut":
      return "timed out in page";
    case "neverAnswered":
      return "no answer";
    case "failed":
      return "failed";
    case "notSent":
      return "not sent";
  }
}

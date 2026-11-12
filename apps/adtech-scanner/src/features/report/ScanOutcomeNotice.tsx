import { BanIcon, XCircleIcon } from "lucide-react";
import type { ScanSummary } from "@/bindings";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";

/** Explains a scan that ended without a report. */
export function ScanOutcomeNotice({ summary }: { summary: ScanSummary }) {
  if (summary.status === "failed") {
    return (
      <Alert tone="danger">
        <XCircleIcon />
        <AlertTitle>This scan failed</AlertTitle>
        <AlertDescription>
          {summary.error ?? "The scan stopped with an unknown error."} The audit trail below shows how far it got.
        </AlertDescription>
      </Alert>
    );
  }
  return (
    <Alert>
      <BanIcon />
      <AlertTitle>This scan was cancelled</AlertTitle>
      <AlertDescription>
        No report was produced. Findings and audit trail entries recorded before the cancellation are kept.
      </AlertDescription>
    </Alert>
  );
}

import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import type { ErrorCode, ScanOptions } from "@/bindings";
import { toApiError } from "@/lib/api";
import { type ScanStart, useStartScan } from "@/lib/query/hooks";

/** What a refused start is called, by its code. The description is always the backend's own sentence. */
const FAILURE_TITLE: Partial<Record<ErrorCode, string>> = {
  browserNotFound: "No browser found",
  browserbaseAuth: "Browserbase is not ready",
  browserbaseLimit: "Browserbase is at its limit",
  browserbaseFailed: "Browserbase could not run the browser",
};

/** Failures that are fixed on the settings screen. */
const NEEDS_SETTINGS: ErrorCode[] = ["browserNotFound", "apiKeyMissing", "apiKeyInvalid", "browserbaseAuth"];

/** Starts a scan and opens its live view; failures become toasts with a way forward. */
export function useLaunchScan() {
  const startScan = useStartScan();
  const navigate = useNavigate();

  const start = (scan: ScanStart) =>
    startScan.mutate(scan, {
      onSuccess: (summary) => navigate({ to: "/scans/$scanId", params: { scanId: summary.id } }),
      onError: (reason) => {
        const error = toApiError(reason);
        toast.error(FAILURE_TITLE[error.code] ?? "Could not start the scan", {
          description: error.message,
          action: NEEDS_SETTINGS.includes(error.code)
            ? { label: "Open settings", onClick: () => navigate({ to: "/settings" }) }
            : undefined,
        });
      },
    });

  return {
    launch: (options: ScanOptions) => start({ options }),
    /** Runs a saved configuration as it is stored; the scan is recorded as its last run. */
    launchConfiguration: (configurationId: string) => start({ configurationId }),
    isPending: startScan.isPending,
  };
}

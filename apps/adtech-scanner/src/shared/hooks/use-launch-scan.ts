import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import type { ScanOptions } from "@/bindings";
import { toApiError } from "@/lib/api";
import { type ScanStart, useStartScan } from "@/lib/query/hooks";

/** Starts a scan and opens its live view; failures become toasts with a way forward. */
export function useLaunchScan() {
  const startScan = useStartScan();
  const navigate = useNavigate();

  const start = (scan: ScanStart) =>
    startScan.mutate(scan, {
      onSuccess: (summary) => navigate({ to: "/scans/$scanId", params: { scanId: summary.id } }),
      onError: (reason) => {
        const error = toApiError(reason);
        const needsSettings = ["browserNotFound", "apiKeyMissing", "apiKeyInvalid"].includes(error.code);
        toast.error(error.code === "browserNotFound" ? "No browser found" : "Could not start the scan", {
          description: error.message,
          action: needsSettings ? { label: "Open settings", onClick: () => navigate({ to: "/settings" }) } : undefined,
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

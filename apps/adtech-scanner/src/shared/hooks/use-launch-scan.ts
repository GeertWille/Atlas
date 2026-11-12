import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import type { ScanOptions } from "@/bindings";
import { toApiError } from "@/lib/api";
import { useStartScan } from "@/lib/query/hooks";

/** Starts a scan and opens its live view; failures become toasts with a way forward. */
export function useLaunchScan() {
  const startScan = useStartScan();
  const navigate = useNavigate();

  const launch = (options: ScanOptions) =>
    startScan.mutate(options, {
      onSuccess: (summary) => navigate({ to: "/scans/$scanId", params: { scanId: summary.id } }),
      onError: (reason) => {
        const error = toApiError(reason);
        const needsSettings = error.code === "browserNotFound" || error.code === "apiKeyMissing";
        toast.error(error.code === "browserNotFound" ? "No browser found" : "Could not start the scan", {
          description: error.message,
          action: needsSettings ? { label: "Open settings", onClick: () => navigate({ to: "/settings" }) } : undefined,
        });
      },
    });

  return { launch, isPending: startScan.isPending };
}

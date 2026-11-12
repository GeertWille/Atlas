import { toast } from "sonner";
import type { ScanEvent } from "@/bindings";
import { isActive } from "@/shared/domain/vocabulary";

/** Toasts the outcome of scans the user is not currently looking at. */
export function announceScanOutcome(
  event: ScanEvent,
  isViewing: (scanId: string) => boolean,
  open: (scanId: string) => void,
) {
  if (event.type !== "scanUpdated") return;
  const { summary } = event;
  if (isActive(summary.status) || summary.status === "cancelled" || isViewing(summary.id)) return;
  const action = { label: "Open", onClick: () => open(summary.id) };
  if (summary.status === "failed") {
    toast.error(`Scan of ${summary.hostname} failed`, { description: summary.error ?? undefined, action });
  } else {
    toast.success(`Scan of ${summary.hostname} finished`, {
      description: summary.healthScore === null ? undefined : `Health score ${summary.healthScore}/100`,
      action,
    });
  }
}

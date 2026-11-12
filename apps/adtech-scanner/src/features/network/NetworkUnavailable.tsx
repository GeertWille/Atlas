import { Link } from "@tanstack/react-router";
import { ArchiveXIcon, HistoryIcon, type LucideIcon, WifiOffIcon } from "lucide-react";
import type { NetworkAvailability, ScanDetail } from "@/bindings";
import { rescanOptions } from "@/shared/domain/scan-options";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";

type Missing = Exclude<NetworkAvailability, "available">;

const REASON: Record<Missing, { icon: LucideIcon; title: string; description: string; settings: boolean }> = {
  notRecorded: {
    icon: HistoryIcon,
    title: "No network record for this scan",
    description:
      "It was made before the app recorded what a scan loaded, or it ended before its page was captured. A new scan records every request.",
    settings: false,
  },
  disabled: {
    icon: WifiOffIcon,
    title: "Network recording was off for this scan",
    description:
      "“Keep network data” was switched off in Settings when it ran, so no request of it was stored. The findings and the rest of the report are complete.",
    settings: true,
  },
  evicted: {
    icon: ArchiveXIcon,
    title: "The network record of this scan was removed",
    description:
      "It was deleted to stay under the storage limit, or cleared in Settings. The report itself is kept. A new scan records the page again, as it is now.",
    settings: true,
  },
};

/** Why a scan has no requests to show, and the way to get some: scan again. */
export function NetworkUnavailable({ detail, availability }: { detail: ScanDetail; availability: Missing }) {
  const { launch, isPending } = useLaunchScan();
  const reason = REASON[availability];
  const options = rescanOptions(detail.options);
  return (
    <Empty icon={reason.icon} title={reason.title} description={reason.description}>
      <Button size="sm" disabled={isPending} onClick={() => launch(options)}>
        Scan again
      </Button>
      {options.aiEnabled ? (
        <Button
          variant="outline"
          size="sm"
          disabled={isPending}
          onClick={() => launch({ ...options, aiEnabled: false })}
        >
          Scan again without AI (free)
        </Button>
      ) : null}
      {reason.settings ? (
        <Button asChild variant="ghost" size="sm">
          <Link to="/settings">Storage settings</Link>
        </Button>
      ) : null}
    </Empty>
  );
}

/** What a scan's network record is, for callers that only need to know whether there is a list to show. */
export function networkState(detail: Pick<ScanDetail, "network">): "available" | "listOnly" | Missing {
  const network = detail.network;
  if (!network) return "notRecorded";
  if (network.availability === "evicted") return network.requestCount > 0 ? "listOnly" : "evicted";
  return network.availability;
}

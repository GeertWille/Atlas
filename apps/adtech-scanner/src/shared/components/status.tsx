import {
  AlertOctagonIcon,
  AlertTriangleIcon,
  BanIcon,
  CheckCircle2Icon,
  CircleDashedIcon,
  CircleDotIcon,
  CircleIcon,
  InfoIcon,
  type LucideIcon,
  MinusCircleIcon,
  MonitorIcon,
  SmartphoneIcon,
  XCircleIcon,
} from "lucide-react";
import type { DeviceProfile, ScanStatus, Severity, SeverityCounts, StepStatus } from "@/bindings";
import {
  DEVICE_LABEL,
  SCAN_STATUS_META,
  SEVERITIES,
  SEVERITY_META,
  STEP_STATUS_LABEL,
  totalFindings,
} from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";

export const SEVERITY_ICON: Record<Severity, LucideIcon> = {
  critical: AlertOctagonIcon,
  high: AlertTriangleIcon,
  medium: CircleDotIcon,
  low: CircleIcon,
  info: InfoIcon,
};

export function SeverityBadge({ severity, className }: { severity: Severity; className?: string }) {
  const Icon = SEVERITY_ICON[severity];
  const meta = SEVERITY_META[severity];
  return (
    <Badge tone={meta.tone} className={className}>
      <Icon />
      {meta.label}
    </Badge>
  );
}

/** Compact per-severity tally, most severe first; zero counts are omitted. */
export function SeverityCountPills({
  counts,
  compact = false,
  className,
}: {
  counts: SeverityCounts;
  /** Tighter pills for table cells. */
  compact?: boolean;
  className?: string;
}) {
  if (totalFindings(counts) === 0) {
    return <span className={cn("text-[13px] text-subtle-foreground", className)}>No findings</span>;
  }
  return (
    <span className={cn("inline-flex items-center", compact ? "gap-0.5" : "gap-1", className)}>
      {SEVERITIES.filter((severity) => counts[severity] > 0).map((severity) => {
        const Icon = SEVERITY_ICON[severity];
        const meta = SEVERITY_META[severity];
        return (
          <Badge
            key={severity}
            tone={meta.tone}
            className={cn("tabular-nums", compact && "gap-0.5 px-1.5")}
            title={`${counts[severity]} ${meta.label.toLowerCase()}`}
          >
            <Icon />
            <span className="sr-only">{meta.label}: </span>
            {counts[severity]}
          </Badge>
        );
      })}
    </span>
  );
}

const SCAN_STATUS_ICON: Record<ScanStatus, LucideIcon> = {
  queued: CircleDashedIcon,
  running: CircleDotIcon,
  completed: CheckCircle2Icon,
  failed: XCircleIcon,
  cancelled: BanIcon,
};

export function ScanStatusBadge({ status }: { status: ScanStatus }) {
  const Icon = SCAN_STATUS_ICON[status];
  const meta = SCAN_STATUS_META[status];
  return (
    <Badge tone={meta.tone}>
      <Icon className={cn(status === "running" && "animate-pulse-dot")} />
      {meta.label}
    </Badge>
  );
}

/** Pulsing marker for something that is happening right now. */
export function LiveDot() {
  return (
    <span aria-hidden="true" className="relative flex size-2.5 shrink-0">
      <span className="absolute inline-flex size-full animate-ping rounded-full bg-violet opacity-60" />
      <span className="relative inline-flex size-2.5 rounded-full bg-violet" />
    </span>
  );
}

export function DeviceIcon({ device, className }: { device: DeviceProfile; className?: string }) {
  const Icon = device === "desktop" ? MonitorIcon : SmartphoneIcon;
  return <Icon className={className} aria-hidden="true" />;
}

export function DeviceLabel({ device, className }: { device: DeviceProfile; className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-1.5 text-muted-foreground", className)}>
      <DeviceIcon device={device} className="size-3.5" />
      {DEVICE_LABEL[device]}
    </span>
  );
}

const STEP_STATUS_STYLE: Record<StepStatus, { icon: LucideIcon; className: string }> = {
  pending: { icon: CircleDashedIcon, className: "text-subtle-foreground" },
  running: { icon: CircleDotIcon, className: "text-primary-text animate-pulse-dot" },
  completed: { icon: CheckCircle2Icon, className: "text-green-fg" },
  failed: { icon: XCircleIcon, className: "text-red-fg" },
  skipped: { icon: MinusCircleIcon, className: "text-subtle-foreground" },
  cancelled: { icon: BanIcon, className: "text-muted-foreground" },
};

export function StepStatusIcon({ status, className }: { status: StepStatus; className?: string }) {
  const { icon: Icon, className: tone } = STEP_STATUS_STYLE[status];
  return <Icon role="img" aria-label={STEP_STATUS_LABEL[status]} className={cn("size-4 shrink-0", tone, className)} />;
}

import { Link } from "@tanstack/react-router";
import {
  CopyIcon,
  MoreHorizontalIcon,
  PencilIcon,
  PlayIcon,
  TextCursorInputIcon,
  Trash2Icon,
  TriangleAlertIcon,
} from "lucide-react";
import { toast } from "sonner";
import type { ScanConfiguration, ScanSummary, Settings } from "@/bindings";
import { useLatestConfigurationScans, useSaveScanConfiguration } from "@/lib/query/hooks";
import { HealthRing } from "@/shared/components/health-ring";
import { ScanIdentity } from "@/shared/components/scan-identity";
import { DeviceLabel, ScanStatusBadge } from "@/shared/components/status";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { cn } from "@/shared/lib/cn";
import { formatDateTime, formatRelativeTime } from "@/shared/lib/format";
import { notifyError } from "@/shared/lib/notify-error";
import { hostnameOf, pathOf } from "@/shared/lib/url";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Hint } from "@/shared/ui/tooltip";
import {
  configurationProblem,
  duplicateName,
  healthChange,
  proxySummary,
  visitorSummary,
} from "./configuration-summary";

const CELL = "px-3 py-2.5 align-middle";

/** How the health score moved against the run before: "+4" or "−12", spelled out for screen readers. */
function HealthChange({ scans }: { scans: ScanSummary[] }) {
  const change = healthChange(scans);
  if (change === null || change === 0) return null;
  return (
    <span className={cn("font-medium tabular-nums", change > 0 ? "text-green-fg" : "text-red-fg")}>
      <span aria-hidden="true">{change > 0 ? `+${change}` : `−${-change}`}</span>
      <span className="sr-only">
        , {change > 0 ? "up" : "down"} {Math.abs(change)} from the run before,
      </span>
    </span>
  );
}

/** The newest scan started from the configuration: its health, how that moved, and its age, linked to the report. */
function LastRun({ configuration }: { configuration: ScanConfiguration }) {
  const scans = useLatestConfigurationScans(configuration.id);
  const [scan] = scans;
  if (!scan) {
    return (
      <span className="text-subtle-foreground">
        {configuration.lastRunAt === null
          ? "Never run"
          : // The run happened, but its scan has been deleted from history since.
            `Ran ${formatRelativeTime(configuration.lastRunAt)}`}
      </span>
    );
  }
  return (
    <Link
      to="/scans/$scanId"
      params={{ scanId: scan.id }}
      title={formatDateTime(scan.startedAt)}
      className="inline-flex items-center gap-2 rounded-sm text-muted-foreground hover:text-foreground"
    >
      {scan.status === "completed" ? (
        <HealthRing score={scan.healthScore} size={28} />
      ) : (
        <ScanStatusBadge status={scan.status} />
      )}
      <HealthChange scans={scans} />
      <span className="whitespace-nowrap">{formatRelativeTime(scan.startedAt)}</span>
    </Link>
  );
}

export type RowAction = { kind: "rename" | "delete"; configuration: ScanConfiguration };

/** One configuration: what it scans, how, how its last run went, and one click to open it or run it again. */
export function ConfigurationRow({
  configuration,
  settings,
  names,
  underSite,
  onAction,
}: {
  configuration: ScanConfiguration;
  settings: Settings;
  /** Every configuration's name, so a duplicate gets one that is free. */
  names: string[];
  /** The row sits under its site's heading, so only the path is left to say. */
  underSite: boolean;
  onAction: (action: RowAction) => void;
}) {
  const { launchConfiguration, isPending } = useLaunchScan();
  const save = useSaveScanConfiguration();
  const { options, name } = configuration;
  const problem = configurationProblem(configuration, settings);

  const duplicate = () =>
    save.mutate(
      { name: duplicateName(name, names), options },
      {
        onSuccess: (copy) => toast.success(`Created “${copy.name}”`),
        onError: (reason) => notifyError("Could not duplicate the configuration", reason),
      },
    );

  return (
    <tr className="border-b transition-colors last:border-b-0 hover:bg-accent/50">
      <td className={`${CELL} max-w-0 pl-4`}>
        <div className="flex min-w-0 flex-col gap-0.5">
          <Link
            to="/"
            search={{ configuration: configuration.id }}
            title={`Open “${name}” in the composer`}
            className="truncate rounded-sm font-medium hover:underline"
          >
            {name}
          </Link>
          <span className="flex min-w-0 items-center gap-2 text-xs">
            {underSite ? (
              <span className="block min-w-0 truncate text-subtle-foreground" title={options.url}>
                {pathOf(options.url) || "Home page"}
              </span>
            ) : (
              <ScanIdentity scan={{ url: options.url, hostname: hostnameOf(options.url) }} />
            )}
          </span>
          {problem ? (
            <span className="flex items-start gap-1.5 text-xs text-pretty text-amber-fg">
              <TriangleAlertIcon aria-hidden="true" className="mt-px size-3.5 shrink-0" />
              {problem}
            </span>
          ) : null}
        </div>
      </td>
      <td className={CELL}>
        <div className="flex flex-col gap-0.5 text-xs">
          <span className="flex items-center gap-2 whitespace-nowrap">
            <DeviceLabel device={options.device} />
            <span className="truncate text-muted-foreground">{proxySummary(configuration, settings)}</span>
          </span>
          <span className="truncate text-subtle-foreground">{visitorSummary(configuration)}</span>
        </div>
      </td>
      <td className={`${CELL} text-xs`}>
        <LastRun configuration={configuration} />
      </td>
      <td className={`${CELL} pr-3`}>
        <div className="flex items-center justify-end gap-1">
          <Button
            variant="outline"
            size="sm"
            disabled={problem !== null || isPending}
            aria-label={`Run ${name}`}
            onClick={() => launchConfiguration(configuration.id)}
          >
            <PlayIcon />
            Run
          </Button>
          <DropdownMenu>
            <Hint label="Actions">
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon-sm" aria-label={`Actions for ${name}`}>
                  <MoreHorizontalIcon />
                </Button>
              </DropdownMenuTrigger>
            </Hint>
            <DropdownMenuContent align="end">
              <DropdownMenuGroup>
                <DropdownMenuItem asChild>
                  <Link to="/" search={{ configuration: configuration.id }}>
                    <PencilIcon />
                    Edit
                  </Link>
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={duplicate}>
                  <CopyIcon />
                  Duplicate
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => onAction({ kind: "rename", configuration })}>
                  <TextCursorInputIcon />
                  Rename…
                </DropdownMenuItem>
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                <DropdownMenuItem destructive onSelect={() => onAction({ kind: "delete", configuration })}>
                  <Trash2Icon />
                  Delete…
                </DropdownMenuItem>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </td>
    </tr>
  );
}

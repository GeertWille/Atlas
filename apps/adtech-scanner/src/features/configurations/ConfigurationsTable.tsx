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
import { useState } from "react";
import { toast } from "sonner";
import type { ScanConfiguration, Settings } from "@/bindings";
import { useDeleteScanConfiguration, useLastConfigurationScan, useSaveScanConfiguration } from "@/lib/query/hooks";
import { HealthRing } from "@/shared/components/health-ring";
import { ScanIdentity } from "@/shared/components/scan-identity";
import { DeviceLabel, ScanStatusBadge } from "@/shared/components/status";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { formatDateTime, formatRelativeTime } from "@/shared/lib/format";
import { notifyError } from "@/shared/lib/notify-error";
import { hostnameOf } from "@/shared/lib/url";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
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
import { configurationProblem, duplicateName, proxySummary, visitorSummary } from "./configuration-summary";
import { RenameConfigurationDialog } from "./RenameConfigurationDialog";

const HEAD = "px-3 py-2 text-left text-xs font-medium text-muted-foreground whitespace-nowrap";
const CELL = "px-3 py-2.5 align-middle";

/** The newest scan started from the configuration: its health and age, linked to the report. */
function LastRun({ configuration }: { configuration: ScanConfiguration }) {
  const scan = useLastConfigurationScan(configuration.id);
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
      <span className="whitespace-nowrap">{formatRelativeTime(scan.startedAt)}</span>
    </Link>
  );
}

type RowAction = { kind: "rename" | "delete"; configuration: ScanConfiguration } | null;

function Row({
  configuration,
  settings,
  names,
  onAction,
}: {
  configuration: ScanConfiguration;
  settings: Settings;
  /** Every configuration's name, so a duplicate gets one that is free. */
  names: string[];
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
          <span className="truncate font-medium" title={name}>
            {name}
          </span>
          <span className="flex min-w-0 items-center gap-2 text-xs">
            <ScanIdentity scan={{ url: options.url, hostname: hostnameOf(options.url) }} />
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

/** Saved configurations as rows: what each one scans, how, when it last ran, and one click to run it again. */
export function ConfigurationsTable({
  configurations,
  shown = configurations,
  settings,
}: {
  /** Every configuration; duplicates are named against all of them. */
  configurations: ScanConfiguration[];
  /** The rows to list, when that is not all of them. */
  shown?: ScanConfiguration[];
  settings: Settings;
}) {
  const remove = useDeleteScanConfiguration();
  const [action, setAction] = useState<RowAction>(null);
  const names = configurations.map((configuration) => configuration.name);
  const deleting = action?.kind === "delete" ? action.configuration : null;

  return (
    <>
      <table className="w-full table-fixed text-[13px]">
        <thead>
          <tr className="border-b">
            <th scope="col" className={`${HEAD} pl-4`}>
              Configuration
            </th>
            <th scope="col" className={`${HEAD} w-48 xl:w-64`}>
              Setup
            </th>
            <th scope="col" className={`${HEAD} w-32`}>
              Last run
            </th>
            <th scope="col" className={`${HEAD} w-32`}>
              <span className="sr-only">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {shown.map((configuration) => (
            <Row
              key={configuration.id}
              configuration={configuration}
              settings={settings}
              names={names}
              onAction={setAction}
            />
          ))}
        </tbody>
      </table>

      {action?.kind === "rename" ? (
        <RenameConfigurationDialog configuration={action.configuration} onClose={() => setAction(null)} />
      ) : null}
      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setAction(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete “{deleting?.name}”?</AlertDialogTitle>
            <AlertDialogDescription>
              The saved setup is removed. Scans that were run from it stay in history.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              destructive
              onClick={() => {
                if (!deleting) return;
                remove.mutate(deleting.id, {
                  onSuccess: () => toast.success(`Deleted “${deleting.name}”`),
                  onError: (reason) => notifyError("Could not delete the configuration", reason),
                });
              }}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

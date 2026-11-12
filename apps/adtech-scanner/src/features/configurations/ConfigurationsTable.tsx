import { useState } from "react";
import { toast } from "sonner";
import type { ScanConfiguration, Settings } from "@/bindings";
import { useDeleteScanConfiguration } from "@/lib/query/hooks";
import { pluralize } from "@/shared/lib/format";
import { notifyError } from "@/shared/lib/notify-error";
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
import { ConfigurationRow, type RowAction } from "./ConfigurationRow";
import type { SiteGroup } from "./configuration-list";
import { RenameConfigurationDialog } from "./RenameConfigurationDialog";

const HEAD = "px-3 py-2 text-left text-xs font-medium text-muted-foreground whitespace-nowrap";

function SiteHeading({ group }: { group: SiteGroup }) {
  return (
    <tr className="border-b bg-muted/40">
      <th scope="rowgroup" colSpan={4} className="px-4 py-1.5 text-left text-xs font-medium">
        <span className="selectable">{group.site}</span>{" "}
        <span className="ml-1 font-normal text-subtle-foreground tabular-nums">
          {pluralize(group.configurations.length, "configuration")}
        </span>
      </th>
    </tr>
  );
}

/**
 * Saved configurations as rows: what each one scans, how, when it last ran, and one click to run it again.
 * Either one flat list (`shown`) or one block per site (`groups`).
 */
export function ConfigurationsTable({
  configurations,
  settings,
  ...layout
}: {
  /** Every configuration; duplicates are named against all of them. */
  configurations: ScanConfiguration[];
  settings: Settings;
} & ({ shown: ScanConfiguration[] } | { groups: SiteGroup[] })) {
  const remove = useDeleteScanConfiguration();
  const [action, setAction] = useState<RowAction | null>(null);
  const names = configurations.map((configuration) => configuration.name);
  const deleting = action?.kind === "delete" ? action.configuration : null;

  const rows = (listed: ScanConfiguration[], underSite: boolean) =>
    listed.map((configuration) => (
      <ConfigurationRow
        key={configuration.id}
        configuration={configuration}
        settings={settings}
        names={names}
        underSite={underSite}
        onAction={setAction}
      />
    ));

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
            <th scope="col" className={`${HEAD} w-40`}>
              Last run
            </th>
            <th scope="col" className={`${HEAD} w-32`}>
              <span className="sr-only">Actions</span>
            </th>
          </tr>
        </thead>
        {"groups" in layout ? (
          layout.groups.map((group) => (
            <tbody key={group.site} className="border-b last:border-b-0">
              <SiteHeading group={group} />
              {rows(group.configurations, true)}
            </tbody>
          ))
        ) : (
          <tbody>{rows(layout.shown, false)}</tbody>
        )}
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

import { Link } from "@tanstack/react-router";
import { BookmarkIcon, PlusIcon } from "lucide-react";
import type { ScanConfiguration, Settings } from "@/bindings";
import { Panel } from "@/shared/components/panel";
import { pluralize } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { ConfigurationsTable } from "./ConfigurationsTable";

export function ConfigurationsScreen({
  configurations,
  settings,
}: {
  configurations: ScanConfiguration[];
  settings: Settings;
}) {
  return (
    <>
      <div className="flex items-center justify-between gap-6">
        <p className="text-[13px] text-pretty text-muted-foreground">
          A configuration keeps a page with its whole setup: device, visitor, proxy and options. Run it again with one
          click to see what changed.
        </p>
        <Button asChild variant="outline" size="sm" className="shrink-0">
          <Link to="/">
            <PlusIcon />
            New from a scan
          </Link>
        </Button>
      </div>
      <Panel aria-label="Configurations" className="overflow-hidden">
        {configurations.length === 0 ? (
          <Empty
            icon={BookmarkIcon}
            title="No configurations yet"
            description="Save the options of a scan to repeat it exactly: the same site from Belgium on mobile and from Germany on desktop, each one click away. Fill in a scan on Home, open Options and choose “Save as configuration”."
          >
            <Button asChild variant="outline" size="sm">
              <Link to="/">Set up a scan</Link>
            </Button>
          </Empty>
        ) : (
          <ConfigurationsTable configurations={configurations} settings={settings} />
        )}
      </Panel>
      {configurations.length > 0 ? (
        <p className="text-[13px] text-muted-foreground tabular-nums">
          {pluralize(configurations.length, "configuration")}
        </p>
      ) : null}
    </>
  );
}

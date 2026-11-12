import { Link } from "@tanstack/react-router";
import type { ScanConfiguration, Settings } from "@/bindings";
import { Panel, PanelHeader } from "@/shared/components/panel";
import { Button } from "@/shared/ui/button";
import { ConfigurationsTable } from "./ConfigurationsTable";

const SHOWN = 4;

/** The dashboard's shortcut to saved configurations, most recently run first. Nothing while there are none. */
export function ConfigurationsPanel({
  configurations,
  settings,
}: {
  configurations: ScanConfiguration[];
  settings: Settings;
}) {
  if (configurations.length === 0) return null;
  const recentFirst = [...configurations].sort((a, b) => (b.lastRunAt ?? b.updatedAt) - (a.lastRunAt ?? a.updatedAt));
  return (
    <Panel aria-label="Configurations" className="overflow-hidden">
      <PanelHeader
        title="Configurations"
        description="Saved scans you can run again with one click"
        actions={
          <Button asChild variant="ghost" size="sm">
            <Link to="/configurations">
              {configurations.length > SHOWN ? `View all ${configurations.length}` : "Manage"}
            </Link>
          </Button>
        }
      />
      <ConfigurationsTable configurations={configurations} shown={recentFirst.slice(0, SHOWN)} settings={settings} />
    </Panel>
  );
}

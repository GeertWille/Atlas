import { Link } from "@tanstack/react-router";
import type { ScanConfiguration, Settings } from "@/bindings";
import { Panel, PanelHeader } from "@/shared/components/panel";
import { Button } from "@/shared/ui/button";
import { ConfigurationsTable } from "./ConfigurationsTable";
import { sortConfigurations } from "./configuration-list";

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
  return (
    <Panel aria-label="Configurations" className="overflow-hidden">
      <PanelHeader
        title="Configurations"
        description="Saved scan setups, most recently used first. Open one to change it, or run it as it is."
        actions={
          <Button asChild variant="ghost" size="sm">
            <Link to="/configurations">
              {configurations.length > SHOWN ? `View all ${configurations.length}` : "Manage"}
            </Link>
          </Button>
        }
      />
      <ConfigurationsTable
        configurations={configurations}
        shown={sortConfigurations(configurations, "recent").slice(0, SHOWN)}
        settings={settings}
      />
    </Panel>
  );
}

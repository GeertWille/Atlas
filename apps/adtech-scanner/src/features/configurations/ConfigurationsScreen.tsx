import { BookmarkIcon, PlusIcon, SearchXIcon } from "lucide-react";
import { useState } from "react";
import type { ScanConfiguration, Settings } from "@/bindings";
import { Panel } from "@/shared/components/panel";
import { SearchInput } from "@/shared/components/search-input";
import { countOf } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { ConfigurationsTable } from "./ConfigurationsTable";
import {
  type ConfigurationSort,
  groupBySite,
  SORT_LABEL,
  searchConfigurations,
  sortConfigurations,
} from "./configuration-list";

const SORTS = Object.keys(SORT_LABEL) as ConfigurationSort[];

/** Every saved configuration, grouped by site, with search and sort for when there are many. */
export function ConfigurationsScreen({
  configurations,
  settings,
  onNew,
}: {
  configurations: ScanConfiguration[];
  settings: Settings;
  /** Takes the user to the composer, where a configuration is made from a scan's options. */
  onNew: () => void;
}) {
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<ConfigurationSort>("recent");

  if (configurations.length === 0) {
    return (
      <Panel aria-label="Configurations" className="overflow-hidden">
        <Empty
          icon={BookmarkIcon}
          title="No configurations yet"
          description="A configuration keeps a page with its whole setup, so the same scan can be repeated exactly: the same site from Belgium on mobile and from Germany on desktop, each one click away. Set up a scan on Home and choose “Save as configuration…”."
        >
          <Button variant="outline" size="sm" onClick={onNew}>
            Set up a scan
          </Button>
        </Empty>
      </Panel>
    );
  }

  const found = searchConfigurations(configurations, search, settings);
  return (
    <>
      <div className="flex items-center gap-2">
        <SearchInput
          aria-label="Search configurations"
          placeholder="Search by name, site, device or proxy"
          value={search}
          onValueChange={setSearch}
          className="w-80 max-w-full"
        />
        <Select value={sort} onValueChange={(next) => setSort(next as ConfigurationSort)}>
          <SelectTrigger aria-label="Sort by" className="w-40">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {SORTS.map((option) => (
                <SelectItem key={option} value={option}>
                  {SORT_LABEL[option]}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
        <Button variant="outline" onClick={onNew} className="ml-auto">
          <PlusIcon />
          New configuration
        </Button>
      </div>
      <Panel aria-label="Configurations" className="overflow-hidden">
        {found.length === 0 ? (
          <Empty
            icon={SearchXIcon}
            title="No configurations match"
            description={`Nothing is named, located or set up like “${search.trim()}”.`}
          >
            <Button variant="outline" size="sm" onClick={() => setSearch("")}>
              Show all configurations
            </Button>
          </Empty>
        ) : (
          <ConfigurationsTable
            configurations={configurations}
            groups={groupBySite(sortConfigurations(found, sort))}
            settings={settings}
          />
        )}
      </Panel>
      <p className="text-[13px] text-muted-foreground tabular-nums">
        {countOf(found.length, configurations.length, "configuration")}
      </p>
    </>
  );
}

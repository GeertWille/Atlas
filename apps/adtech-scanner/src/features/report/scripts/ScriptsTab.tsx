import { FileSearchIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { ScriptRecord } from "@/bindings";
import { FilterMenu } from "@/shared/components/filter-menu";
import { Panel } from "@/shared/components/panel";
import { SearchInput } from "@/shared/components/search-input";
import { SCRIPT_CATEGORIES, SCRIPT_CATEGORY_META } from "@/shared/domain/vocabulary";
import { countOf } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { useReportLinks } from "../report-links";
import { ScriptDetails } from "./ScriptDetails";
import { ScriptFunnel } from "./ScriptFunnel";
import { ScriptsTable } from "./ScriptsTable";
import { filterScripts, NO_SCRIPT_FILTERS, type ScriptFilters, type ScriptSort, sortScripts } from "./script-filters";

export function ScriptsTab({
  scripts,
  focusedScriptUrl,
  showPages = false,
}: {
  scripts: ScriptRecord[];
  /** The scan visited several pages: list the ones that loaded each script. */
  showPages?: boolean;
  /** Deep link from a finding's evidence: select that script. */
  focusedScriptUrl?: string;
}) {
  const [filters, setFilters] = useState<ScriptFilters>(NO_SCRIPT_FILTERS);
  const [sort, setSort] = useState<ScriptSort>("loadOrder");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const links = useReportLinks();

  useEffect(() => {
    const target = focusedScriptUrl ? scripts.find((script) => script.url === focusedScriptUrl) : undefined;
    if (!target) return;
    setFilters(NO_SCRIPT_FILTERS);
    setSelectedId(target.id);
  }, [focusedScriptUrl, scripts]);

  if (scripts.length === 0) {
    return (
      <Empty
        icon={FileSearchIcon}
        title="No scripts captured"
        description="The page loaded no JavaScript while it was observed."
      />
    );
  }

  const visible = sortScripts(filterScripts(scripts, filters), sort);
  const selected = scripts.find((script) => script.id === selectedId) ?? null;
  const presentCategories = SCRIPT_CATEGORIES.filter((category) =>
    scripts.some((script) => script.category === category),
  );

  return (
    <div className="flex flex-col gap-3">
      <ScriptFunnel
        scripts={scripts}
        scope={filters.scope}
        onScopeChange={(scope) => setFilters({ ...filters, scope })}
      />
      <div className="flex items-center gap-2">
        <SearchInput
          aria-label="Search scripts"
          placeholder="Search URL, vendor or reason"
          value={filters.search}
          onValueChange={(search) => setFilters({ ...filters, search })}
          className="w-72"
        />
        <FilterMenu
          label="Category"
          selected={filters.categories}
          onChange={(categories) => setFilters({ ...filters, categories })}
          options={presentCategories.map((category) => ({
            value: category,
            label: SCRIPT_CATEGORY_META[category].label,
            count: scripts.filter((script) => script.category === category).length,
          }))}
        />
        <span className="ml-auto text-[13px] text-muted-foreground tabular-nums" aria-live="polite">
          {countOf(visible.length, scripts.length, "script")}
        </span>
      </div>
      <Panel className="flex flex-col overflow-hidden">
        {visible.length === 0 ? (
          <Empty icon={FileSearchIcon} title="No scripts match">
            <Button variant="outline" size="sm" onClick={() => setFilters(NO_SCRIPT_FILTERS)}>
              Clear filters
            </Button>
          </Empty>
        ) : (
          <ScriptsTable
            scripts={visible}
            sort={sort}
            onSortChange={setSort}
            showPages={showPages}
            selectedId={selectedId}
            onSelect={(script) => setSelectedId(script.id === selectedId ? null : script.id)}
            onOpen={links ? (script) => links.openSource({ scriptId: script.id }) : undefined}
          />
        )}
        {selected ? <ScriptDetails script={selected} onClose={() => setSelectedId(null)} /> : null}
      </Panel>
    </div>
  );
}

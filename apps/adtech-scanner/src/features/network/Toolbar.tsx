import { CheckIcon, ChevronDownIcon, Columns3Icon, ListFilterIcon, SparklesIcon, WorkflowIcon } from "lucide-react";
import type { Auction, NetworkCategory, ScanDetail } from "@/bindings";
import { cn } from "@/shared/lib/cn";
import { formatInteger } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Hint } from "@/shared/ui/tooltip";
import { CATEGORIES, CATEGORY_META } from "./labels";
import { activeCategory, hasToken, STATE_TOKENS, type StateChip, toggleToken, withCategory } from "./query/edit";
import type { CategoryCounts } from "./query/facets";
import { SLOW_BID_SHARE, SLOW_MS } from "./query/match";
import { RECIPES } from "./query/recipes";
import type { NetworkGrouping } from "./search";
import { COLUMNS, type ColumnChoices, type ColumnDef, type ColumnKey } from "./table/columns";

const chip =
  "inline-flex h-6 shrink-0 items-center gap-1.5 rounded-full border px-2.5 text-xs whitespace-nowrap transition-colors aria-pressed:border-primary-text aria-pressed:bg-primary-soft aria-pressed:font-medium aria-pressed:text-primary-text";
const idle = "border-border-strong text-muted-foreground hover:bg-accent hover:text-foreground";

function Count({ value }: { value: number }) {
  return <span className="tabular-nums opacity-80">{formatInteger(value)}</span>;
}

const STATE_LABEL: Record<StateChip, { label: string; short: string; hint: string }> = {
  failed: { label: "Failed", short: "Failed", hint: "A status of 400 or more, or a network error" },
  slow: {
    label: "Slow",
    short: "Slow",
    hint: `Longer than ${SLOW_MS / 1000} s, or a bid call that used over ${SLOW_BID_SHARE * 100}% of its auction's timeout`,
  },
  blocked: {
    label: "Blocked or cancelled",
    short: "Blocked",
    hint: "Refused by the browser, or aborted (usually because the page navigated)",
  },
};

/** Chips are shortcuts for words of the filter: a category chip swaps the `cat:` word for its own. */
export function CategoryChips({
  query,
  onQueryChange,
  counts,
  compact,
}: {
  query: string;
  onQueryChange: (query: string) => void;
  counts: CategoryCounts;
  /** Short of room: the categories fold into one menu. */
  compact: boolean;
}) {
  const category = activeCategory(query);
  const present = CATEGORIES.filter((candidate) => (counts.byCategory[candidate] ?? 0) > 0 || candidate === category);
  const select = (next: NetworkCategory | null) => onQueryChange(withCategory(query, next));
  if (compact) {
    return (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="outline"
            size="sm"
            className="h-6 w-fit gap-1.5 rounded-full px-2.5 text-xs"
            aria-label={`Category: ${category ? CATEGORY_META[category].label : "All"}`}
          >
            <ListFilterIcon className="size-3.5!" />
            {category ? CATEGORY_META[category].label : "All categories"}
            <Count value={category ? (counts.byCategory[category] ?? 0) : counts.all} />
            <ChevronDownIcon className="size-3.5! text-subtle-foreground" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuGroup>
            <DropdownMenuItem onSelect={() => select(null)}>
              <span className="flex-1">All</span>
              <Count value={counts.all} />
            </DropdownMenuItem>
            {present.map((candidate) => (
              <DropdownMenuItem key={candidate} onSelect={() => select(candidate)}>
                <span className="flex-1">{CATEGORY_META[candidate].label}</span>
                {candidate === category ? <CheckIcon className="text-primary-text!" /> : null}
                <Count value={counts.byCategory[candidate] ?? 0} />
              </DropdownMenuItem>
            ))}
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    );
  }
  return (
    // biome-ignore lint/a11y/useSemanticElements: a row of toggle buttons, not a form fieldset
    <div role="group" aria-label="Category" className="flex min-w-0 flex-wrap items-center gap-1.5">
      <button
        type="button"
        aria-pressed={category === null}
        onClick={() => select(null)}
        className={cn(chip, category !== null && idle)}
      >
        All <Count value={counts.all} />
      </button>
      {present.map((candidate) => (
        <button
          key={candidate}
          type="button"
          aria-pressed={candidate === category}
          aria-label={`${CATEGORY_META[candidate].label} ${counts.byCategory[candidate] ?? 0}`}
          title={`${CATEGORY_META[candidate].label}: ${CATEGORY_META[candidate].description}`}
          onClick={() => select(candidate === category ? null : candidate)}
          className={cn(chip, candidate !== category && idle)}
        >
          {CATEGORY_META[candidate].short} <Count value={counts.byCategory[candidate] ?? 0} />
        </button>
      ))}
    </div>
  );
}

/** State chips add their own word to the filter; several can be on at once. */
export function StateChips({
  query,
  onQueryChange,
  counts,
}: {
  query: string;
  onQueryChange: (query: string) => void;
  counts: Record<StateChip, number>;
}) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: a row of toggle buttons, not a form fieldset
    <div role="group" aria-label="State" className="flex shrink-0 items-center gap-1.5">
      {(Object.keys(STATE_TOKENS) as StateChip[]).map((state) => {
        const on = hasToken(query, STATE_TOKENS[state]);
        return (
          <button
            key={state}
            type="button"
            aria-pressed={on}
            aria-label={`${STATE_LABEL[state].label} ${counts[state]}`}
            title={`${STATE_LABEL[state].label}: ${STATE_LABEL[state].hint}`}
            onClick={() => onQueryChange(toggleToken(query, STATE_TOKENS[state]))}
            className={cn(chip, !on && idle)}
          >
            {STATE_LABEL[state].short} <Count value={counts[state]} />
          </button>
        );
      })}
    </div>
  );
}

/** Filters that follow the ad lifecycle: what happened before the first auction, and during each one. */
export function LifecycleMenu({
  scan,
  auctions,
  query,
  onQueryChange,
}: {
  scan: ScanDetail;
  auctions: Auction[];
  query: string;
  onQueryChange: (query: string) => void;
}) {
  const number = (auction: Auction) => scan.auctions.findIndex((candidate) => candidate.id === auction.id) + 1;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm">
          <WorkflowIcon />
          Lifecycle
          <ChevronDownIcon className="text-subtle-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuGroup>
          <DropdownMenuCheckboxItem
            checked={hasToken(query, "is:before-auction")}
            onCheckedChange={() => onQueryChange(toggleToken(query, "is:before-auction"))}
          >
            Before the first auction
          </DropdownMenuCheckboxItem>
          {auctions.map((auction) => {
            const token = `auction:${number(auction)}`;
            return (
              <DropdownMenuCheckboxItem
                key={auction.id}
                checked={hasToken(query, token)}
                onCheckedChange={() => onQueryChange(toggleToken(query, token))}
              >
                During auction {number(auction)}
                {scan.pages.length > 1 ? <span className="text-subtle-foreground">page {auction.page + 1}</span> : null}
              </DropdownMenuCheckboxItem>
            );
          })}
        </DropdownMenuGroup>
        {auctions.length === 0 ? (
          <p className="max-w-64 px-2 py-1.5 text-xs text-pretty text-muted-foreground">
            No auction ran here. “Before the first auction” then shows every request: the page never got that far.
          </p>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function RecipesMenu({ recents, onPick }: { recents: string[]; onPick: (query: string) => void }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm">
          <SparklesIcon />
          Recipes
          <ChevronDownIcon className="text-subtle-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-w-96">
        <DropdownMenuGroup>
          {RECIPES.map((recipe) => (
            <DropdownMenuItem
              key={recipe.query}
              onSelect={() => onPick(recipe.query)}
              className="flex-col items-start gap-0"
            >
              {recipe.label}
              <span className="font-mono text-[11px] text-subtle-foreground">{recipe.query}</span>
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
        {recents.length > 0 ? (
          <>
            <DropdownMenuSeparator />
            <p className="px-2 pt-1 pb-0.5 text-[11px] font-medium text-subtle-foreground">Recent filters</p>
            <DropdownMenuGroup>
              {recents.map((recent) => (
                <DropdownMenuItem key={recent} onSelect={() => onPick(recent)}>
                  <span className="truncate font-mono text-xs">{recent}</span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuGroup>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

const GROUP_LABEL: Record<NetworkGrouping, string> = {
  page: "Page view",
  vendor: "Vendor",
  type: "Type",
  none: "None",
};

export function GroupMenu({
  value,
  onChange,
  multiPage,
}: {
  value: NetworkGrouping;
  onChange: (grouping: NetworkGrouping) => void;
  multiPage: boolean;
}) {
  const options: NetworkGrouping[] = multiPage ? ["page", "vendor", "type", "none"] : ["vendor", "type", "none"];
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" aria-label={`Group by: ${GROUP_LABEL[value]}`}>
          <span className="text-muted-foreground">Group</span>
          {GROUP_LABEL[value]}
          <ChevronDownIcon className="text-subtle-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuGroup>
          {options.map((option) => (
            <DropdownMenuItem key={option} onSelect={() => onChange(option)}>
              <span className="flex-1">{GROUP_LABEL[option]}</span>
              {option === value ? <CheckIcon className="text-primary-text!" /> : null}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function ColumnsMenu({
  shown,
  choices,
  onChoicesChange,
  disabled,
  comfortable,
  onComfortableChange,
}: {
  /** The columns on screen now, defaults included. */
  shown: ColumnDef[];
  choices: ColumnChoices;
  onChoicesChange: (choices: ColumnChoices) => void;
  /** Beside a docked detail panel the list shows its essential columns only. */
  disabled: boolean;
  comfortable: boolean;
  onComfortableChange: (comfortable: boolean) => void;
}) {
  const isShown = (key: ColumnKey) => shown.some((column) => column.key === key);
  return (
    <DropdownMenu>
      <Hint label={disabled ? "Columns are fixed while the details are docked beside the list" : "Columns"}>
        <DropdownMenuTrigger asChild>
          <Button variant="outline" size="icon-sm" aria-label="Columns" disabled={disabled}>
            <Columns3Icon />
          </Button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end" className="max-h-96 overflow-y-auto">
        <DropdownMenuGroup>
          {COLUMNS.filter((column) => !column.fixed).map((column) => (
            <DropdownMenuCheckboxItem
              key={column.key}
              checked={isShown(column.key)}
              onCheckedChange={(checked) => onChoicesChange({ ...choices, [column.key]: checked })}
            >
              {column.label}
            </DropdownMenuCheckboxItem>
          ))}
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuCheckboxItem checked={comfortable} onCheckedChange={onComfortableChange}>
            Taller rows
          </DropdownMenuCheckboxItem>
          {Object.keys(choices).length > 0 ? (
            <DropdownMenuItem onSelect={() => onChoicesChange({})}>Back to the default columns</DropdownMenuItem>
          ) : null}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

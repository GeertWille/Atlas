import {
  BookmarkIcon,
  HistoryIcon,
  HomeIcon,
  MonitorIcon,
  MoonIcon,
  PlayIcon,
  PlusIcon,
  ScrollTextIcon,
  SettingsIcon,
  SunIcon,
} from "lucide-react";
import { useState } from "react";
import type { ScanConfiguration, ScanSummary, ThemePreference } from "@/bindings";
import { DeviceIcon } from "@/shared/components/status";
import { useReturnFocus } from "@/shared/hooks/use-return-focus";
import { formatRelativeTime } from "@/shared/lib/format";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from "@/shared/ui/command";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/shared/ui/dialog";
import { MOD_KEY } from "./shortcuts";

export type PaletteActions = {
  newScan: () => void;
  scanDemo: () => void;
  runConfiguration: (configurationId: string) => void;
  goTo: (to: "/" | "/configurations" | "/history" | "/logs" | "/settings") => void;
  openScan: (scanId: string) => void;
  searchLogs: (query: string) => void;
  setTheme: (theme: ThemePreference) => void;
};

/** Plain term matching instead of cmdk's fuzzy scoring, which surfaces unrelated commands for short queries. */
function matchesEveryTerm(value: string, search: string): number {
  const haystack = value.toLowerCase();
  return search
    .toLowerCase()
    .split(/\s+/)
    .every((term) => haystack.includes(term))
    ? 1
    : 0;
}

/** Mounted only while the palette is open, so every opening starts with an empty query. */
function PaletteCommands({
  recentScans,
  configurations,
  actions,
  onClose,
}: {
  recentScans: ScanSummary[];
  configurations: ScanConfiguration[];
  actions: PaletteActions;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const run = (action: () => void) => () => {
    onClose();
    action();
  };
  const trimmed = query.trim();

  return (
    <Command loop filter={matchesEveryTerm}>
      <CommandInput placeholder="Type a command or search…" value={query} onValueChange={setQuery} />
      <CommandList>
        <CommandEmpty>No matching commands.</CommandEmpty>
        <CommandGroup heading="Actions">
          <CommandItem onSelect={run(actions.newScan)}>
            <PlusIcon />
            New scan
            <CommandShortcut>{MOD_KEY}N</CommandShortcut>
          </CommandItem>
          <CommandItem onSelect={run(actions.scanDemo)}>
            <PlayIcon />
            Scan the demo site
          </CommandItem>
        </CommandGroup>
        {configurations.length > 0 ? (
          <CommandGroup heading="Configurations">
            {configurations.map((configuration) => (
              <CommandItem
                key={configuration.id}
                value={`run configuration ${configuration.name} ${configuration.options.url} ${configuration.id}`}
                onSelect={run(() => actions.runConfiguration(configuration.id))}
              >
                <PlayIcon />
                <span className="min-w-0 flex-1 truncate">Run configuration: {configuration.name}</span>
                <DeviceIcon device={configuration.options.device} />
              </CommandItem>
            ))}
          </CommandGroup>
        ) : null}
        <CommandGroup heading="Go to">
          <CommandItem onSelect={run(() => actions.goTo("/"))}>
            <HomeIcon />
            Home
          </CommandItem>
          <CommandItem onSelect={run(() => actions.goTo("/configurations"))}>
            <BookmarkIcon />
            Configurations
          </CommandItem>
          <CommandItem onSelect={run(() => actions.goTo("/history"))}>
            <HistoryIcon />
            History
          </CommandItem>
          <CommandItem onSelect={run(() => actions.goTo("/logs"))}>
            <ScrollTextIcon />
            Log search
          </CommandItem>
          <CommandItem onSelect={run(() => actions.goTo("/settings"))}>
            <SettingsIcon />
            Settings
            <CommandShortcut>{MOD_KEY},</CommandShortcut>
          </CommandItem>
        </CommandGroup>
        {recentScans.length > 0 ? (
          <CommandGroup heading="Recent scans">
            {recentScans.map((scan) => (
              <CommandItem
                key={scan.id}
                value={`${scan.url} ${scan.device} ${scan.id}`}
                onSelect={run(() => actions.openScan(scan.id))}
              >
                <DeviceIcon device={scan.device} />
                <span className="min-w-0 flex-1 truncate">{scan.url.replace(/^https?:\/\//, "")}</span>
                <CommandShortcut>{formatRelativeTime(scan.startedAt)}</CommandShortcut>
              </CommandItem>
            ))}
          </CommandGroup>
        ) : null}
        <CommandGroup heading="Theme">
          <CommandItem onSelect={run(() => actions.setTheme("light"))}>
            <SunIcon />
            Use light theme
          </CommandItem>
          <CommandItem onSelect={run(() => actions.setTheme("dark"))}>
            <MoonIcon />
            Use dark theme
          </CommandItem>
          <CommandItem onSelect={run(() => actions.setTheme("system"))}>
            <MonitorIcon />
            Follow system theme
          </CommandItem>
        </CommandGroup>
        {trimmed.length >= 2 ? (
          // Always offered for a typed query, after the commands that match it.
          <CommandGroup heading="Search" forceMount>
            <CommandItem
              forceMount
              value={`search audit trails ${trimmed}`}
              onSelect={run(() => actions.searchLogs(trimmed))}
            >
              <ScrollTextIcon />
              Search audit trails for “{trimmed}”
            </CommandItem>
          </CommandGroup>
        ) : null}
      </CommandList>
    </Command>
  );
}

export function CommandPalette({
  open,
  onOpenChange,
  recentScans,
  configurations,
  actions,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  recentScans: ScanSummary[];
  /** The ones that can run as saved. */
  configurations: ScanConfiguration[];
  actions: PaletteActions;
}) {
  const returnFocus = useReturnFocus();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showClose={false}
        className="top-[18%] max-w-[560px] translate-y-0 gap-0 overflow-hidden p-0"
        {...returnFocus}
      >
        <DialogTitle className="sr-only">Command palette</DialogTitle>
        <DialogDescription className="sr-only">
          Search scans, run a saved configuration, navigate, or run a command.
        </DialogDescription>
        <PaletteCommands
          recentScans={recentScans}
          configurations={configurations}
          actions={actions}
          onClose={() => onOpenChange(false)}
        />
      </DialogContent>
    </Dialog>
  );
}

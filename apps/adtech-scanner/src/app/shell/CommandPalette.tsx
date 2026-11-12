import { HistoryIcon, HomeIcon, MonitorIcon, MoonIcon, PlayIcon, PlusIcon, ScrollTextIcon, SettingsIcon, SunIcon } from "lucide-react";
import { useState } from "react";
import type { ScanSummary, ThemePreference } from "@/bindings";
import { DeviceIcon } from "@/shared/components/status";
import { formatRelativeTime } from "@/shared/lib/format";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandShortcut } from "@/shared/ui/command";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/shared/ui/dialog";
import { MOD_KEY } from "./shortcuts";

export type PaletteActions = {
  newScan: () => void;
  scanDemo: () => void;
  goTo: (to: "/" | "/history" | "/logs" | "/settings") => void;
  openScan: (scanId: string) => void;
  searchLogs: (query: string) => void;
  setTheme: (theme: ThemePreference) => void;
};

export function CommandPalette({
  open,
  onOpenChange,
  recentScans,
  actions,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  recentScans: ScanSummary[];
  actions: PaletteActions;
}) {
  const [query, setQuery] = useState("");
  const run = (action: () => void) => () => {
    onOpenChange(false);
    setQuery("");
    action();
  };
  const trimmed = query.trim();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent showClose={false} className="top-[18%] max-w-[560px] translate-y-0 gap-0 overflow-hidden p-0">
        <DialogTitle className="sr-only">Command palette</DialogTitle>
        <DialogDescription className="sr-only">Search scans, navigate, or run a command.</DialogDescription>
        <Command loop>
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
              {trimmed.length >= 2 ? (
                <CommandItem value={`search audit trails ${trimmed}`} onSelect={run(() => actions.searchLogs(trimmed))}>
                  <ScrollTextIcon />
                  Search audit trails for “{trimmed}”
                </CommandItem>
              ) : null}
            </CommandGroup>
            <CommandGroup heading="Go to">
              <CommandItem onSelect={run(() => actions.goTo("/"))}>
                <HomeIcon />
                Home
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
                  <CommandItem key={scan.id} value={`${scan.url} ${scan.device} ${scan.id}`} onSelect={run(() => actions.openScan(scan.id))}>
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
          </CommandList>
        </Command>
      </DialogContent>
    </Dialog>
  );
}

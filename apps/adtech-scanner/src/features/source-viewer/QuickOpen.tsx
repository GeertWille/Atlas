import type { Finding, ScriptRecord } from "@/bindings";
import { SEVERITY_ICON } from "@/shared/components/status";
import { useReturnFocus } from "@/shared/hooks/use-return-focus";
import { pluralize } from "@/shared/lib/format";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/shared/ui/command";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/shared/ui/dialog";
import { FILE_GROUP_LABEL, groupFiles } from "./file-groups";

function matchesEveryTerm(value: string, search: string): number {
  const haystack = value.toLowerCase();
  return search
    .toLowerCase()
    .split(/\s+/)
    .every((term) => haystack.includes(term))
    ? 1
    : 0;
}

/** Go to a script by name. The way in when the file list is folded away. */
export function QuickOpen({
  open,
  onOpenChange,
  scripts,
  findings,
  onSelect,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  scripts: ScriptRecord[];
  findings: Finding[];
  onSelect: (scriptId: string) => void;
}) {
  const returnFocus = useReturnFocus();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showClose={false}
        className="top-[18%] max-w-xl translate-y-0 gap-0 overflow-hidden p-0"
        {...returnFocus}
      >
        <DialogTitle className="sr-only">Go to file</DialogTitle>
        <DialogDescription className="sr-only">Type part of a script's name, host or vendor.</DialogDescription>
        {/* Mounted only while open, so every opening starts with an empty query. */}
        <Command filter={matchesEveryTerm}>
          <CommandInput placeholder="Go to file…" />
          <CommandList>
            <CommandEmpty>No script matches.</CommandEmpty>
            {groupFiles(scripts, findings).map((group) => (
              <CommandGroup key={group.key} heading={FILE_GROUP_LABEL[group.key]}>
                {group.files.map((file) => {
                  const Icon = file.worst ? SEVERITY_ICON[file.worst] : null;
                  return (
                    <CommandItem
                      key={file.script.id}
                      value={`${file.name} ${file.script.url} ${file.script.vendor ?? ""} ${file.script.id}`}
                      onSelect={() => {
                        onOpenChange(false);
                        onSelect(file.script.id);
                      }}
                    >
                      <span className="truncate font-medium">{file.name}</span>
                      <span className="truncate text-xs text-subtle-foreground">{file.origin}</span>
                      {Icon ? (
                        <span className="ml-auto flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
                          {pluralize(file.findingCount, "finding")}
                          <Icon />
                        </span>
                      ) : null}
                    </CommandItem>
                  );
                })}
              </CommandGroup>
            ))}
          </CommandList>
        </Command>
      </DialogContent>
    </Dialog>
  );
}

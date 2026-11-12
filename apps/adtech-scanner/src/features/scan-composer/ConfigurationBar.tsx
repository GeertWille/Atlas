import { Link } from "@tanstack/react-router";
import { BookmarkIcon, XIcon } from "lucide-react";
import type { ScanConfiguration } from "@/bindings";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Hint } from "@/shared/ui/tooltip";

function status(edited: boolean, blocked: boolean): string {
  if (blocked) return "This configuration cannot run until another proxy is chosen under Origin.";
  if (edited) return "Scan runs these edited options once; they are not saved yet.";
  return "Scan runs this configuration as saved and records it as its last run.";
}

/**
 * The configuration the composer has open: its name, whether the options differ from what is saved, what
 * Scan does in either case, and how to keep or drop the edits. Two lines in both states, so editing does
 * not push the form down.
 */
export function ConfigurationBar({
  configuration,
  edited,
  blocked,
  saving,
  onRevert,
  onUpdate,
  onSaveAsNew,
}: {
  configuration: ScanConfiguration;
  edited: boolean;
  /** The options name a saved proxy that has been deleted: nothing can be scanned or saved until that changes. */
  blocked: boolean;
  saving: boolean;
  onRevert: () => void;
  onUpdate: () => void;
  onSaveAsNew: () => void;
}) {
  return (
    <div className="flex flex-col gap-0.5 border-b bg-muted/40 py-1.5 pr-2 pl-4">
      <div className="flex min-h-7 items-center gap-3">
        <p className="flex min-w-0 flex-1 items-center gap-2 text-[13px]">
          <BookmarkIcon aria-hidden="true" className="size-4 shrink-0 text-primary-text" />
          <span className="text-muted-foreground">Configuration</span>
          <span className="truncate font-medium" title={configuration.name}>
            {configuration.name}
          </span>
          {edited ? <Badge tone="amber">Edited</Badge> : null}
        </p>
        <Hint label="Close configuration">
          <Button asChild variant="ghost" size="icon-sm" aria-label="Close configuration">
            <Link to="/">
              <XIcon />
            </Link>
          </Button>
        </Hint>
      </div>
      <div className="flex min-h-7 items-center justify-between gap-3 pl-6">
        <p className={cn("min-w-0 text-xs text-pretty", blocked ? "text-amber-fg" : "text-muted-foreground")}>
          {status(edited, blocked)}
        </p>
        {edited ? (
          <div className="flex shrink-0 items-center gap-1">
            <Button variant="ghost" size="sm" disabled={saving} onClick={onRevert}>
              Revert
            </Button>
            <Button variant="ghost" size="sm" disabled={saving} onClick={onSaveAsNew}>
              Save as new…
            </Button>
            <Button variant="outline" size="sm" disabled={saving} onClick={onUpdate}>
              Update configuration
            </Button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

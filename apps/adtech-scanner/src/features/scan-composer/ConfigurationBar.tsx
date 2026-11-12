import { Link } from "@tanstack/react-router";
import { BookmarkIcon, XIcon } from "lucide-react";
import type { ScanConfiguration } from "@/bindings";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";

/** The loaded configuration: its name, whether the options differ from what is saved, and how to save them. */
export function ConfigurationBar({
  configuration,
  edited,
  saving,
  onUpdate,
  onSaveAsNew,
}: {
  configuration: ScanConfiguration;
  edited: boolean;
  saving: boolean;
  onUpdate: () => void;
  onSaveAsNew: () => void;
}) {
  return (
    <div className="flex min-h-11 flex-wrap items-center gap-x-3 gap-y-1.5 border-b px-4 py-2">
      <p className="flex min-w-0 flex-1 items-center gap-2 text-[13px]">
        <BookmarkIcon aria-hidden="true" className="size-4 shrink-0 text-primary-text" />
        <span className="text-muted-foreground">Configuration</span>
        <span className="truncate font-medium">{configuration.name}</span>
        {edited ? <Badge tone="amber">Edited</Badge> : null}
      </p>
      {edited ? (
        <>
          <Button variant="outline" size="sm" disabled={saving} onClick={onUpdate}>
            Update configuration
          </Button>
          <Button variant="ghost" size="sm" disabled={saving} onClick={onSaveAsNew}>
            Save as new…
          </Button>
        </>
      ) : null}
      <Button asChild variant="ghost" size="icon-sm" aria-label="Close configuration">
        <Link to="/">
          <XIcon />
        </Link>
      </Button>
    </div>
  );
}

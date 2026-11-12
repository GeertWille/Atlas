import type { ScanSummary } from "@/bindings";
import { cn } from "@/shared/lib/cn";
import { pathOf } from "@/shared/lib/url";

/** Hostname with the path de-emphasised; truncates as one line. */
export function ScanIdentity({
  scan,
  className,
}: {
  scan: Pick<ScanSummary, "hostname" | "url">;
  className?: string;
}) {
  return (
    <span className={cn("block min-w-0 truncate", className)} title={scan.url}>
      <span className="font-medium text-foreground">{scan.hostname}</span>
      <span className="text-subtle-foreground">{pathOf(scan.url)}</span>
    </span>
  );
}

import { Link } from "@tanstack/react-router";
import { ArchiveXIcon, FileX2Icon, HistoryIcon, type LucideIcon, RulerIcon } from "lucide-react";
import type { Finding, ScanDetail, ScriptRecord, SourceAvailability } from "@/bindings";
import { CodeBlock } from "@/shared/components/code-block";
import { SeverityBadge } from "@/shared/components/status";
import { rescanOptions } from "@/shared/domain/scan-options";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { formatBytes } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { findingsCiting } from "./file-groups";

type Unavailable = Exclude<SourceAvailability, "available">;

const REASON: Record<Unavailable, { icon: LucideIcon; title: string; description: string }> = {
  notRecorded: {
    icon: HistoryIcon,
    title: "This scan has no stored source",
    description: "It was made before the app kept what a scan loaded. Scan the page again to read its scripts here.",
  },
  notRetained: {
    icon: FileX2Icon,
    title: "This script's source was not kept",
    description:
      "“Keep network data” was off when this scan ran, or the scan had reached its storage cap. Nothing of this script was stored.",
  },
  evicted: {
    icon: ArchiveXIcon,
    title: "This script's source was removed",
    description:
      "The network data of this scan was deleted to stay under the storage limit, or cleared in Settings. The report itself is kept.",
  },
  tooLarge: {
    icon: RulerIcon,
    title: "This script is too large to show",
    description: "Text over 8 MB is not loaded into the viewer. What the report knows about the file is above.",
  },
};

function KeptEvidence({ script, findings }: { script: ScriptRecord; findings: Finding[] }) {
  const cited = findingsCiting(script, findings).flatMap((finding) =>
    finding.evidence
      .filter((evidence) => evidence.kind === "code" && (evidence.scriptId ?? script.id) === script.id)
      .map((evidence) => ({ finding, evidence })),
  );
  if (cited.length === 0) return null;
  return (
    <section aria-label="Evidence the report kept" className="flex w-full max-w-2xl flex-col gap-2.5">
      <h2 className="text-xs font-semibold tracking-wide text-subtle-foreground uppercase">
        Evidence the report kept ({cited.length})
      </h2>
      {cited.map(({ finding, evidence }, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: evidence of a stored finding never reorders
        <figure key={`${finding.id}-${index}`} className="overflow-hidden rounded-lg border text-left">
          <figcaption className="flex min-h-9 items-center gap-2 border-b bg-muted/50 px-3 py-1 text-xs">
            <SeverityBadge severity={finding.severity} />
            <span className="min-w-0 flex-1 truncate font-medium">{evidence.label}</span>
            {evidence.line ? (
              <span className="shrink-0 font-mono text-subtle-foreground tabular-nums">
                L{evidence.line}
                {evidence.endLine && evidence.endLine > evidence.line ? `–${evidence.endLine}` : ""}
              </span>
            ) : null}
          </figcaption>
          <CodeBlock code={evidence.body} startLine={evidence.line ?? undefined} />
        </figure>
      ))}
    </section>
  );
}

/** Why there is no code to show, what is still known, and the way to get the code: scan again. */
export function SourceUnavailable({
  detail,
  script,
  availability,
}: {
  detail: ScanDetail;
  script: ScriptRecord;
  availability: Unavailable;
}) {
  const { launch, isPending } = useLaunchScan();
  const reason = REASON[availability];
  const options = rescanOptions(detail.options);
  return (
    <div className="flex h-full flex-col items-center gap-2 overflow-y-auto bg-code px-6 pb-8">
      <Empty
        icon={reason.icon}
        title={reason.title}
        description={
          <>
            {reason.description}
            {availability === "tooLarge" ? ` This file is ${formatBytes(script.sizeBytes)} as transferred.` : ""}
            {availability !== "tooLarge"
              ? " A new scan captures the page as it is now: its code may have changed."
              : ""}
          </>
        }
        className="pb-6"
      >
        {availability === "tooLarge" ? null : (
          <>
            <Button size="sm" disabled={isPending} onClick={() => launch(options)}>
              Scan again
            </Button>
            {options.aiEnabled ? (
              <Button
                variant="outline"
                size="sm"
                disabled={isPending}
                onClick={() => launch({ ...options, aiEnabled: false })}
              >
                Scan again without AI (free)
              </Button>
            ) : null}
            {availability === "notRetained" || availability === "evicted" ? (
              <Button asChild variant="ghost" size="sm">
                <Link to="/settings">Storage settings</Link>
              </Button>
            ) : null}
          </>
        )}
      </Empty>
      <KeptEvidence script={script} findings={detail.findings} />
    </div>
  );
}

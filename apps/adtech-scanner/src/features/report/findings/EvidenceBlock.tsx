import { ActivityIcon, BracesIcon, FileCode2Icon, GaugeIcon, type LucideIcon, ScrollTextIcon } from "lucide-react";
import type { Evidence, EvidenceKind } from "@/bindings";
import { CodeBlock } from "@/shared/components/code-block";
import { CopyButton } from "@/shared/components/copy-button";
import { fileNameOf } from "@/shared/lib/url";
import { Button } from "@/shared/ui/button";
import { Hint } from "@/shared/ui/tooltip";

const KIND_ICON: Record<EvidenceKind, LucideIcon> = {
  code: BracesIcon,
  metric: GaugeIcon,
  log: ScrollTextIcon,
  timeline: ActivityIcon,
};

const KIND_LABEL: Record<EvidenceKind, string> = {
  code: "Code",
  metric: "Metric",
  log: "Log",
  timeline: "Timeline",
};

export function EvidenceBlock({
  evidence,
  onOpenScript,
}: {
  evidence: Evidence;
  onOpenScript: (scriptUrl: string) => void;
}) {
  const Icon = KIND_ICON[evidence.kind];
  const { scriptUrl } = evidence;
  return (
    <figure className="overflow-hidden rounded-lg border">
      <figcaption className="flex min-h-9 items-center gap-2 border-b bg-muted/50 py-1 pr-1.5 pl-3 text-xs">
        <Icon role="img" aria-label={KIND_LABEL[evidence.kind]} className="size-3.5 shrink-0 text-subtle-foreground" />
        <span className="min-w-0 flex-1 truncate font-medium">{evidence.label}</span>
        {scriptUrl ? (
          <Hint label={`Show in Scripts: ${scriptUrl}`}>
            <Button variant="ghost" size="sm" className="h-6 max-w-64 gap-1 px-1.5 font-mono text-[11px]" onClick={() => onOpenScript(scriptUrl)}>
              <FileCode2Icon className="size-3.5!" />
              <span className="truncate">
                {fileNameOf(scriptUrl)}
                {evidence.line ? `:${evidence.line}` : ""}
              </span>
            </Button>
          </Hint>
        ) : null}
        <CopyButton text={evidence.body} label="Copy evidence" className="size-6" />
      </figcaption>
      {evidence.kind === "code" ? (
        <CodeBlock code={evidence.body} startLine={evidence.line ?? undefined} />
      ) : (
        <p className="selectable bg-code px-3 py-2.5 font-mono text-xs leading-relaxed break-words">{evidence.body}</p>
      )}
    </figure>
  );
}

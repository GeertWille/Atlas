import {
  ActivityIcon,
  BracesIcon,
  FileCode2Icon,
  GaugeIcon,
  type LucideIcon,
  NetworkIcon,
  ScrollTextIcon,
} from "lucide-react";
import type { Evidence, EvidenceKind } from "@/bindings";
import { CodeBlock } from "@/shared/components/code-block";
import { CopyButton } from "@/shared/components/copy-button";
import { fileNameOf } from "@/shared/lib/url";
import { Button } from "@/shared/ui/button";
import { Hint } from "@/shared/ui/tooltip";
import { useReportLinks } from "../report-links";

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
  findingId,
  onOpenScript,
}: {
  evidence: Evidence;
  findingId: string;
  /** For evidence that names a script by URL only, as reports stored before script sources were kept do. */
  onOpenScript: (scriptUrl: string) => void;
}) {
  const Icon = KIND_ICON[evidence.kind];
  const links = useReportLinks();
  const { scriptUrl, scriptId, requestId } = evidence;
  const openRequest = links?.openRequest;
  // With the script's id the chip opens its source at the cited lines; without, the Scripts tab row.
  const inViewer = links !== null && scriptId !== null;
  const openScript = () => {
    if (links && scriptId) {
      links.openSource({
        scriptId,
        line: evidence.line ?? undefined,
        to: evidence.endLine ?? undefined,
        finding: findingId,
      });
    } else if (scriptUrl) onOpenScript(scriptUrl);
  };
  return (
    <figure className="overflow-hidden rounded-lg border">
      <figcaption className="flex min-h-9 items-center gap-2 border-b bg-muted/50 py-1 pr-1.5 pl-3 text-xs">
        <Icon role="img" aria-label={KIND_LABEL[evidence.kind]} className="size-3.5 shrink-0 text-subtle-foreground" />
        <span className="min-w-0 flex-1 truncate font-medium">{evidence.label}</span>
        {scriptUrl ? (
          <Hint label={`${inViewer ? "Open the source at this line" : "Show in Scripts"}: ${scriptUrl}`}>
            <Button
              variant="ghost"
              size="sm"
              className="h-6 max-w-64 gap-1 px-1.5 font-mono text-[11px]"
              onClick={openScript}
            >
              <FileCode2Icon className="size-3.5!" />
              <span className="truncate">
                {fileNameOf(scriptUrl)}
                {evidence.line ? `:${evidence.line}` : ""}
              </span>
            </Button>
          </Hint>
        ) : null}
        {requestId && openRequest ? (
          <Hint label="Open this request in the network view">
            <Button
              variant="ghost"
              size="sm"
              className="h-6 gap-1 px-1.5 text-[11px]"
              onClick={() => openRequest(requestId)}
            >
              <NetworkIcon className="size-3.5!" />
              View request
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

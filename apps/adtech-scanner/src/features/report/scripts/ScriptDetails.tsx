import { XIcon } from "lucide-react";
import type { ScriptRecord } from "@/bindings";
import { CopyButton } from "@/shared/components/copy-button";
import { FINDING_SOURCE_LABEL, SCRIPT_CATEGORY_META } from "@/shared/domain/vocabulary";
import { EMPTY, formatBytes, formatOffset, formatPercent } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Hint } from "@/shared/ui/tooltip";

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-xs text-subtle-foreground">{label}</dt>
      <dd className="selectable text-[13px] tabular-nums">{value}</dd>
    </div>
  );
}

export function ScriptDetails({ script, onClose }: { script: ScriptRecord; onClose: () => void }) {
  return (
    <section aria-label="Script details" className="flex animate-enter flex-col gap-3 border-t bg-muted/30 px-4 py-3.5">
      <div className="flex items-start gap-2">
        <p className="selectable min-w-0 flex-1 font-mono text-xs leading-5 break-all">
          {script.url}
          <span className="ml-2 text-subtle-foreground">sha-256 {script.hash}</span>
        </p>
        <CopyButton text={script.url} label="Copy URL" />
        <Hint label="Close details">
          <Button variant="ghost" size="icon-sm" aria-label="Close details" onClick={onClose}>
            <XIcon />
          </Button>
        </Hint>
      </div>
      <dl className="grid grid-cols-3 gap-x-4 gap-y-3 xl:grid-cols-6">
        <Fact label="Category" value={SCRIPT_CATEGORY_META[script.category].label} />
        <Fact label="Vendor" value={script.vendor ?? EMPTY} />
        <Fact label="Size" value={formatBytes(script.sizeBytes)} />
        <Fact label="Loaded at" value={formatOffset(script.loadedAtMs)} />
        <Fact label="Classified by" value={FINDING_SOURCE_LABEL[script.classifiedBy]} />
        <Fact
          label="Confidence"
          value={script.classificationConfidence === null ? EMPTY : formatPercent(script.classificationConfidence)}
        />
      </dl>
      <p className="selectable text-[13px] text-pretty text-muted-foreground">
        <span className="font-medium text-foreground">{script.adRelated ? "Ad-related. " : "Not ad-related. "}</span>
        {script.classificationReason ?? "No classification reason recorded."}
        {script.analyzed ? " Its beautified source was handed to Phase 2." : ""}
      </p>
    </section>
  );
}

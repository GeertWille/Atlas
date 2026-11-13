import { useId } from "react";
import type { Settings } from "@/bindings";
import type { AiAvailability } from "@/shared/domain/ai-availability";
import { BROWSERBASE_LIST_PRICE, BROWSERBASE_PLAN_CAVEAT, type BrowserbaseEstimate } from "@/shared/domain/browserbase";
import { cn } from "@/shared/lib/cn";
import { formatUsd } from "@/shared/lib/format";
import { Input } from "@/shared/ui/input";
import { Label } from "@/shared/ui/label";
import { Switch } from "@/shared/ui/switch";
import { AiSummary } from "./AiSummary";
import { describeBrowserbaseUsage } from "./cost-estimate";
import { OptionCell, OptionHelp, OptionsSection } from "./option-layout";
import type { ScanForm } from "./scan-form";

function budgetHelp(ai: AiAvailability, aiEnabled: boolean): string {
  if (ai.triage !== null && !aiEnabled) return "Not used while AI is off.";
  return "Hard cap for this scan. Empty uses the settings.";
}

/**
 * What the browser itself costs, apart from the models: Browserbase bills its time and proxy traffic, and no
 * cap of this app applies to that. The last line of the options, and only there for a Browserbase scan, so
 * that nothing above it moves when it comes and goes.
 */
function BrowserCost({ usage }: { usage: BrowserbaseEstimate }) {
  return (
    <p role="note" aria-label="Browser cost" className="text-xs text-pretty text-muted-foreground">
      Browserbase bills apart from the LLM budget: {describeBrowserbaseUsage(usage)}. An estimated{" "}
      <span className="font-medium text-foreground tabular-nums">{formatUsd(usage.usd)}</span> at{" "}
      {BROWSERBASE_LIST_PRICE}; {BROWSERBASE_PLAN_CAVEAT}.
    </p>
  );
}

/** What runs after the capture and what it may cost: the AI switch, this scan's own cap and the estimates. */
export function AnalysisOptions({
  form,
  settings,
  ai,
  aiEnabled,
  budgetUsd,
  budgetError,
  usage,
  update,
}: {
  form: ScanForm;
  settings: Settings;
  ai: AiAvailability;
  /** The AI switch as it takes effect: off while no key is stored, whatever the form says. */
  aiEnabled: boolean;
  /** This scan's own cap, when the form sets a valid one. */
  budgetUsd: number | undefined;
  budgetError: string | undefined;
  /** What the scan is likely to use at Browserbase; `null` on this Mac. */
  usage: BrowserbaseEstimate | null;
  update: (patch: Partial<ScanForm>) => void;
}) {
  const ids = { ai: useId(), budget: useId(), budgetHelp: useId() };
  const triageOnly = ai.triage !== null && !ai.logicAnalysis;
  return (
    <OptionsSection title="Analysis" question="What gets analysed, and what may it cost?">
      <div className="grid grid-cols-2 gap-x-8 gap-y-3">
        <OptionCell
          label={triageOnly ? "AI script triage" : "AI logic analysis"}
          htmlFor={ids.ai}
          control={
            <Switch
              id={ids.ai}
              checked={aiEnabled}
              disabled={ai.triage === null}
              onCheckedChange={(aiEnabled) => update({ aiEnabled })}
            />
          }
        >
          <OptionHelp>
            {triageOnly
              ? "Classify scripts with Jev. Logic analysis needs an Anthropic key."
              : "Classify scripts, then hunt for wrapper logic bugs."}
          </OptionHelp>
        </OptionCell>
        <div className="flex min-w-0 items-start justify-between gap-3">
          <div className="flex min-w-0 flex-col gap-1">
            <Label htmlFor={ids.budget} className="flex min-h-7 items-center">
              LLM budget
            </Label>
            <OptionHelp
              id={ids.budgetHelp}
              role={budgetError ? "alert" : undefined}
              className={cn(budgetError && "text-red-fg")}
            >
              {budgetError ?? budgetHelp(ai, aiEnabled)}
            </OptionHelp>
          </div>
          <div className="relative w-24 shrink-0">
            <span className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-sm text-subtle-foreground">
              $
            </span>
            <Input
              id={ids.budget}
              inputMode="decimal"
              autoComplete="off"
              placeholder={settings.budgetUsd.toFixed(2)}
              value={form.budget}
              aria-invalid={budgetError ? true : undefined}
              aria-describedby={ids.budgetHelp}
              onChange={(event) => update({ budget: event.target.value })}
              className="pl-6 tabular-nums"
            />
          </div>
        </div>
      </div>
      <AiSummary settings={settings} ai={ai} form={{ ...form, aiEnabled }} budgetUsd={budgetUsd} />
      {usage ? <BrowserCost usage={usage} /> : null}
    </OptionsSection>
  );
}

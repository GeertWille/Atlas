import { ExternalLinkIcon } from "lucide-react";
import { useId } from "react";
import type { Settings, SettingsPatch } from "@/bindings";
import { aiAvailability } from "@/shared/domain/ai-availability";
import { modelLabel, PRICED_MODELS } from "@/shared/domain/model-pricing";
import { BUDGET_USD } from "@/shared/domain/scan-options";
import { openApiKeysPage } from "@/shared/lib/system";
import { Badge } from "@/shared/ui/badge";
import { Field } from "@/shared/ui/field";
import { Switch } from "@/shared/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { ApiKeyField } from "./ApiKeyField";
import { CommitInput } from "./CommitInput";
import { parseBudgetUsd, parseModelId } from "./settings-schema";

const MODEL_SUGGESTIONS_ID = "model-suggestions";

function GetKeyLink({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      className="inline-flex items-center gap-1 font-medium text-primary-text hover:underline"
      onClick={onClick}
    >
      Get a key
      <ExternalLinkIcon className="size-3" />
    </button>
  );
}

/** Says which classifier a scan started now would use, so the effect of the keys and the Jev switch is never a guess. */
function ScriptTriageStatus({ settings }: { settings: Settings }) {
  const { triage, logicAnalysis } = aiAvailability(settings);
  if (triage === "jev") {
    return (
      <Field
        label="Script triage"
        description={
          logicAnalysis
            ? `Jev classifies the scripts that vendor rules cannot place and builds a reading guide for the logic analysis. ${modelLabel(settings.filterModel)} takes over if Jev fails.`
            : "Jev classifies the scripts that vendor rules cannot place. Logic analysis needs an Anthropic key."
        }
      >
        <Badge tone="violet">Jev System One</Badge>
      </Field>
    );
  }
  if (triage === "claude") {
    return (
      <Field
        label="Script triage"
        description="The script filter model classifies the scripts that vendor rules cannot place."
      >
        <Badge tone="blue">{modelLabel(settings.filterModel)}</Badge>
      </Field>
    );
  }
  return (
    <Field label="Script triage" description="No key stored: scripts are classified by vendor rules only.">
      <Badge tone="outline">None</Badge>
    </Field>
  );
}

/** The keys, the Jev switch, the two models, the effort and the per-scan cap. */
export function AiAnalysisFields({ settings, save }: { settings: Settings; save: (patch: SettingsPatch) => void }) {
  const ids = { filter: useId(), analysis: useId(), budget: useId(), jev: useId(), jevDisclosure: useId() };
  return (
    <>
      <Field
        stacked
        label="Anthropic API key"
        description={
          <>
            Stored in your system keychain, never in the app database or logs.{" "}
            <GetKeyLink onClick={() => openApiKeysPage("anthropic")} />
          </>
        }
      >
        <div className="w-full">
          <ApiKeyField provider="anthropic" stored={settings.hasApiKey} />
        </div>
      </Field>
      <Field
        stacked
        label="TypeSafe API key (optional)"
        description={
          <>
            Lets Jev System One, a low-cost classifier, triage scripts. Stored in your system keychain.{" "}
            <GetKeyLink onClick={() => openApiKeysPage("typesafe")} />
          </>
        }
      >
        <div className="w-full">
          <ApiKeyField provider="typesafe" stored={settings.hasTypesafeKey} />
        </div>
      </Field>
      <Field
        label="Classify scripts with Jev"
        htmlFor={ids.jev}
        description={
          <span id={ids.jevDisclosure}>
            Sends excerpts of the scanned page's scripts to api.typesafe.ai, a second vendor. Jev is sent more of each
            script than Claude Haiku.
            {settings.hasTypesafeKey ? null : (
              <span className="mt-1 block font-medium text-foreground">Add a TypeSafe key first.</span>
            )}
          </span>
        }
      >
        <Switch
          id={ids.jev}
          aria-describedby={ids.jevDisclosure}
          disabled={!settings.hasTypesafeKey}
          checked={settings.jevEnabled && settings.hasTypesafeKey}
          onCheckedChange={(jevEnabled) => save({ jevEnabled })}
        />
      </Field>
      <ScriptTriageStatus settings={settings} />
      <Field
        label="Script filter model"
        htmlFor={ids.filter}
        description="Classifies which scripts are ad-related. Used when Jev is off, and as the fallback if Jev fails."
      >
        <CommitInput
          id={ids.filter}
          list={MODEL_SUGGESTIONS_ID}
          spellCheck={false}
          className="w-56 font-mono text-[13px]"
          value={settings.filterModel}
          parse={parseModelId}
          onCommit={(filterModel) => save({ filterModel })}
        />
        {/* Suggestions for both model fields. */}
        <datalist id={MODEL_SUGGESTIONS_ID}>
          {PRICED_MODELS.map((model) => (
            <option key={model} value={model} />
          ))}
        </datalist>
      </Field>
      <Field
        label="Logic analysis model"
        htmlFor={ids.analysis}
        description="Phase 2 reads the beautified scripts with tools. This is where most of the cost goes."
      >
        <CommitInput
          id={ids.analysis}
          list={MODEL_SUGGESTIONS_ID}
          spellCheck={false}
          className="w-56 font-mono text-[13px]"
          value={settings.analysisModel}
          parse={parseModelId}
          onCommit={(analysisModel) => save({ analysisModel })}
        />
      </Field>
      <Field label="Analysis effort" description="Higher effort reads more code and reasons longer, at higher cost.">
        <ToggleGroup
          aria-label="Analysis effort"
          value={settings.analysisEffort}
          onValueChange={(analysisEffort) => save({ analysisEffort })}
        >
          <ToggleGroupItem value="low">Low</ToggleGroupItem>
          <ToggleGroupItem value="medium">Medium</ToggleGroupItem>
          <ToggleGroupItem value="high">High</ToggleGroupItem>
        </ToggleGroup>
      </Field>
      <Field
        label="Budget per scan"
        htmlFor={ids.budget}
        description={`Hard cap on LLM spend. Analysis stops and reports partial results when it is reached. $${BUDGET_USD.min.toFixed(2)} to $${BUDGET_USD.max.toFixed(2)}.`}
      >
        <CommitInput
          leading="$"
          id={ids.budget}
          inputMode="decimal"
          className="w-24 pl-6 text-right tabular-nums"
          value={settings.budgetUsd}
          parse={parseBudgetUsd}
          format={(budget) => budget.toFixed(2)}
          onCommit={(budgetUsd) => save({ budgetUsd })}
        />
      </Field>
    </>
  );
}

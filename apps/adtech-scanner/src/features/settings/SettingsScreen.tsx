import { ExternalLinkIcon, FolderOpenIcon, MonitorIcon, MoonIcon, SmartphoneIcon, SunIcon } from "lucide-react";
import { type ReactNode, useId } from "react";
import { toast } from "sonner";
import type { BrowserInfo, Settings, SettingsPatch } from "@/bindings";
import { useDeleteAllScans, useUpdateSettings } from "@/lib/query/hooks";
import { Panel } from "@/shared/components/panel";
import { aiAvailability } from "@/shared/domain/ai-availability";
import { modelLabel, PRICED_MODELS } from "@/shared/domain/model-pricing";
import { OBSERVE_SECONDS } from "@/shared/domain/scan-options";
import { notifyError } from "@/shared/lib/notify-error";
import { chooseBrowserExecutable, openApiKeysPage } from "@/shared/lib/system";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/shared/ui/alert-dialog";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Field, FieldGroup } from "@/shared/ui/field";
import { Skeleton } from "@/shared/ui/skeleton";
import { Switch } from "@/shared/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { ApiKeyField } from "./ApiKeyField";
import { CommitInput } from "./CommitInput";
import { BUDGET_USD, parseBudgetUsd, parseModelId, parseObserveSeconds } from "./settings-schema";

const MODEL_SUGGESTIONS_ID = "model-suggestions";

declare const __APP_VERSION__: string;

function Section({ title, description, children }: { title: string; description?: ReactNode; children: ReactNode }) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="grid grid-cols-[220px_minmax(0,1fr)] gap-8">
      <div className="flex flex-col gap-1 pt-1">
        <h2 id={headingId} className="text-sm font-semibold">
          {title}
        </h2>
        {description ? <p className="text-[13px] text-pretty text-muted-foreground">{description}</p> : null}
      </div>
      <Panel className="p-4">
        <FieldGroup>{children}</FieldGroup>
      </Panel>
    </section>
  );
}

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

function BrowserField({
  browser,
  settings,
  save,
}: {
  browser: BrowserInfo | null | undefined;
  settings: Settings;
  save: (patch: SettingsPatch) => void;
}) {
  const choose = async () => {
    const path = await chooseBrowserExecutable();
    if (path) save({ chromePath: path });
  };
  return (
    <Field
      stacked
      label="Browser"
      description={
        browser === undefined ? (
          <Skeleton className="h-4 w-64" />
        ) : browser === null ? (
          <span className="text-amber-fg">
            No Chrome or Chromium installation was found. Choose an executable to run scans.
          </span>
        ) : (
          <>
            <span className="font-medium text-foreground">{browser.version}</span>
            {settings.chromePath ? " (chosen manually)" : " (detected automatically)"}
            <span className="selectable mt-1 block truncate font-mono text-xs text-subtle-foreground">
              {browser.path}
            </span>
          </>
        )
      }
    >
      <Button variant="outline" size="sm" onClick={choose}>
        <FolderOpenIcon />
        Choose executable…
      </Button>
      {settings.chromePath ? (
        <Button variant="ghost" size="sm" onClick={() => save({ chromePath: null })}>
          Use auto-detect
        </Button>
      ) : null}
    </Field>
  );
}

function DeleteAllScansButton() {
  const deleteAll = useDeleteAllScans();
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button variant="destructive-outline" size="sm" disabled={deleteAll.isPending}>
          Delete all scans…
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete all scans?</AlertDialogTitle>
          <AlertDialogDescription>
            Every report, finding and audit trail is removed from this computer. Settings and your API keys are kept.
            This cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            destructive
            onClick={() =>
              deleteAll.mutate(undefined, {
                onSuccess: () => toast.success("All scans deleted"),
                onError: (reason) => notifyError("Could not delete the scans", reason),
              })
            }
          >
            Delete all
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

export function SettingsScreen({ settings, browser }: { settings: Settings; browser: BrowserInfo | null | undefined }) {
  const updateSettings = useUpdateSettings();
  const ids = {
    filter: useId(),
    analysis: useId(),
    budget: useId(),
    headless: useId(),
    observe: useId(),
    jev: useId(),
    jevDisclosure: useId(),
  };

  const save = (patch: SettingsPatch) =>
    updateSettings.mutate(patch, {
      onSuccess: () => toast.success("Settings saved", { id: "settings-saved", duration: 1500 }),
      onError: (reason) => notifyError("Could not save settings", reason),
    });

  return (
    <>
      <datalist id={MODEL_SUGGESTIONS_ID}>
        {PRICED_MODELS.map((model) => (
          <option key={model} value={model} />
        ))}
      </datalist>

      <Section
        title="AI analysis"
        description="Two model phases find logic bugs in wrapper code. Without a key, scans still run every deterministic check."
      >
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
      </Section>

      <Section
        title="Browser"
        description="Scans drive a Chromium-based browser on this computer over the DevTools protocol."
      >
        <BrowserField browser={browser} settings={settings} save={save} />
        <Field
          label="Run headless"
          htmlFor={ids.headless}
          description="Hide the browser window during scans. Turn off to watch the page being captured."
        >
          <Switch id={ids.headless} checked={settings.headless} onCheckedChange={(headless) => save({ headless })} />
        </Field>
      </Section>

      <Section title="Scan defaults" description="Starting values for the new-scan composer.">
        <Field label="Device">
          <ToggleGroup
            aria-label="Default device"
            value={settings.defaultDevice}
            onValueChange={(defaultDevice) => save({ defaultDevice })}
          >
            <ToggleGroupItem value="desktop">
              <MonitorIcon />
              Desktop
            </ToggleGroupItem>
            <ToggleGroupItem value="mobile">
              <SmartphoneIcon />
              Mobile
            </ToggleGroupItem>
          </ToggleGroup>
        </Field>
        <Field
          label="Observe after load"
          htmlFor={ids.observe}
          description={`Seconds to keep watching the page, ${OBSERVE_SECONDS.min} to ${OBSERVE_SECONDS.max}.`}
        >
          <CommitInput
            trailing="s"
            id={ids.observe}
            inputMode="numeric"
            className="w-24 pr-7 text-right tabular-nums"
            value={settings.defaultObserveSeconds}
            parse={parseObserveSeconds}
            onCommit={(defaultObserveSeconds) => save({ defaultObserveSeconds })}
          />
        </Field>
      </Section>

      <Section title="Appearance">
        <Field label="Theme" description="System follows your macOS appearance.">
          <ToggleGroup aria-label="Theme" value={settings.theme} onValueChange={(theme) => save({ theme })}>
            <ToggleGroupItem value="system">
              <MonitorIcon />
              System
            </ToggleGroupItem>
            <ToggleGroupItem value="light">
              <SunIcon />
              Light
            </ToggleGroupItem>
            <ToggleGroupItem value="dark">
              <MoonIcon />
              Dark
            </ToggleGroupItem>
          </ToggleGroup>
        </Field>
      </Section>

      <Section
        title="Data"
        description="Everything is stored locally. Nothing is uploaded except the model requests you enable."
      >
        <Field
          label="What is stored"
          description="Reports keep findings with short code excerpts, derived auctions, slots, metrics, script metadata and the audit trail. Script bodies and raw network payloads are held in memory during a scan and then discarded."
        >
          <DeleteAllScansButton />
        </Field>
      </Section>

      <Section title="About">
        <Field
          label="Ad-Tech Scanner"
          description="Finds hidden revenue leaks caused by logic bugs in ad-tech wrappers. Outbound traffic goes only to the page being scanned, api.anthropic.com and, when Jev is enabled, api.typesafe.ai."
        >
          <span className="selectable font-mono text-xs text-muted-foreground">v{__APP_VERSION__}</span>
        </Field>
      </Section>
    </>
  );
}

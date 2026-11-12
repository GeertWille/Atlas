import { Link } from "@tanstack/react-router";
import { ChevronDownIcon, GlobeIcon, MonitorIcon, ScanSearchIcon, SmartphoneIcon, SparklesIcon } from "lucide-react";
import { type FormEvent, useEffect, useId, useState } from "react";
import type { Settings } from "@/bindings";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { cn } from "@/shared/lib/cn";
import { formatUsd } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/shared/ui/collapsible";
import { Label } from "@/shared/ui/label";
import { Slider } from "@/shared/ui/slider";
import { Switch } from "@/shared/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { SCAN_URL_INPUT_ID, takePendingComposerFocus } from "./composer-focus";
import { estimateScanCost } from "./cost-estimate";
import { OBSERVE_SECONDS, type ScanForm, validateScanForm } from "./scan-form";

function OptionSwitch({
  label,
  description,
  checked,
  disabled,
  onCheckedChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  const id = useId();
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="flex flex-col gap-0.5">
        <Label htmlFor={id}>{label}</Label>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
      <Switch id={id} checked={checked} disabled={disabled} onCheckedChange={onCheckedChange} />
    </div>
  );
}

function AiSummary({ settings, aiEnabled }: { settings: Settings; aiEnabled: boolean }) {
  if (!settings.hasApiKey) {
    return (
      <p className="text-xs text-muted-foreground">
        No API key: this scan runs the deterministic checks only.{" "}
        <Link to="/settings" className="font-medium text-primary-text hover:underline">
          Add a key
        </Link>{" "}
        to analyse wrapper logic.
      </p>
    );
  }
  if (!aiEnabled) {
    return <p className="text-xs text-muted-foreground">Deterministic checks only. No LLM cost.</p>;
  }
  const estimate = estimateScanCost(settings);
  return (
    <p className="text-xs text-muted-foreground">
      <SparklesIcon className="mr-1 inline size-3.5 -translate-y-px text-primary-text" />
      Estimated <span className="font-medium text-foreground tabular-nums">{formatUsd(estimate.expectedUsd)}</span> with{" "}
      {settings.analysisModel}, hard cap {formatUsd(settings.budgetUsd)} per scan
      {estimate.cappedByBudget ? ". Analysis may stop early at the cap." : "."}
    </p>
  );
}

export function ScanComposer({ settings }: { settings: Settings }) {
  const { launch, isPending } = useLaunchScan();
  const errorId = useId();
  const observeId = useId();
  const [form, setForm] = useState<ScanForm>({
    url: "",
    device: settings.defaultDevice,
    observeSeconds: settings.defaultObserveSeconds,
    autoScroll: true,
    acceptConsent: true,
    aiEnabled: true,
  });
  const [urlError, setUrlError] = useState<string | null>(null);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const aiEnabled = form.aiEnabled && settings.hasApiKey;

  useEffect(() => {
    if (takePendingComposerFocus()) document.getElementById(SCAN_URL_INPUT_ID)?.focus();
  }, []);

  const update = (patch: Partial<ScanForm>) => setForm((current) => ({ ...current, ...patch }));

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const result = validateScanForm({ ...form, aiEnabled });
    if (!result.ok) {
      setUrlError(result.urlError);
      return;
    }
    launch(result.options);
  };

  return (
    <form
      onSubmit={submit}
      noValidate
      aria-label="New scan"
      className="rounded-xl border bg-card shadow-xs transition-shadow focus-within:shadow-pop"
    >
      <div className="flex items-center gap-2 p-2">
        <div className="relative min-w-0 flex-1">
          <GlobeIcon className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-subtle-foreground" />
          <input
            id={SCAN_URL_INPUT_ID}
            type="text"
            inputMode="url"
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            aria-label="Page URL"
            aria-invalid={urlError ? true : undefined}
            aria-describedby={urlError ? errorId : undefined}
            placeholder="publisher.com/article"
            value={form.url}
            onChange={(event) => {
              update({ url: event.target.value });
              setUrlError(null);
            }}
            className="selectable h-10 w-full rounded-lg bg-transparent pr-3 pl-9 text-[15px] outline-none placeholder:text-subtle-foreground"
          />
        </div>
        <ToggleGroup value={form.device} onValueChange={(device) => update({ device })} aria-label="Device">
          <ToggleGroupItem value="desktop">
            <MonitorIcon />
            Desktop
          </ToggleGroupItem>
          <ToggleGroupItem value="mobile">
            <SmartphoneIcon />
            Mobile
          </ToggleGroupItem>
        </ToggleGroup>
        <Button type="submit" size="lg" disabled={isPending} className="h-9">
          <ScanSearchIcon />
          Scan
        </Button>
      </div>

      {urlError ? (
        <p id={errorId} role="alert" className="px-5 pb-2 text-[13px] text-red-fg">
          {urlError}
        </p>
      ) : null}

      <Collapsible open={optionsOpen} onOpenChange={setOptionsOpen} className="border-t">
        <div className="flex min-h-10 items-center justify-between gap-4 px-4 py-2">
          <AiSummary settings={settings} aiEnabled={aiEnabled} />
          <CollapsibleTrigger asChild>
            <Button variant="ghost" size="sm" className="-mr-2 shrink-0">
              Options
              <ChevronDownIcon className={cn("transition-transform", optionsOpen && "rotate-180")} />
            </Button>
          </CollapsibleTrigger>
        </div>
        <CollapsibleContent>
          <div className="grid grid-cols-2 gap-x-10 gap-y-4 border-t px-4 py-4">
            <div className="flex flex-col gap-2">
              <div className="flex items-baseline justify-between">
                <Label htmlFor={observeId}>Observe after load</Label>
                <span className="text-[13px] text-muted-foreground tabular-nums">{form.observeSeconds} s</span>
              </div>
              <Slider
                id={observeId}
                aria-label="Observe after load, seconds"
                min={OBSERVE_SECONDS.min}
                max={OBSERVE_SECONDS.max}
                step={OBSERVE_SECONDS.step}
                value={[form.observeSeconds]}
                onValueChange={([observeSeconds]) => {
                  if (observeSeconds !== undefined) update({ observeSeconds });
                }}
              />
              <p className="text-xs text-muted-foreground">
                Longer windows catch refreshes and late lazy-loaded auctions.
              </p>
            </div>
            <OptionSwitch
              label="AI logic analysis"
              description="Classify scripts, then hunt for wrapper logic bugs."
              checked={aiEnabled}
              disabled={!settings.hasApiKey}
              onCheckedChange={(checked) => update({ aiEnabled: checked })}
            />
            <OptionSwitch
              label="Auto-scroll"
              description="Scroll the page to trigger lazy-loaded slots."
              checked={form.autoScroll}
              onCheckedChange={(autoScroll) => update({ autoScroll })}
            />
            <OptionSwitch
              label="Accept consent banner"
              description="Click through the CMP so bidding is not blocked."
              checked={form.acceptConsent}
              onCheckedChange={(acceptConsent) => update({ acceptConsent })}
            />
          </div>
        </CollapsibleContent>
      </Collapsible>
    </form>
  );
}

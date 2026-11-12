import { Link, useNavigate } from "@tanstack/react-router";
import {
  BookmarkIcon,
  ChevronDownIcon,
  GlobeIcon,
  MonitorIcon,
  ScanSearchIcon,
  SmartphoneIcon,
  SparklesIcon,
  TriangleAlertIcon,
  XIcon,
} from "lucide-react";
import { type FormEvent, type ReactNode, useEffect, useId, useState } from "react";
import { toast } from "sonner";
import type { ScanConfiguration, Settings } from "@/bindings";
import { useSaveScanConfiguration } from "@/lib/query/hooks";
import { type AiAvailability, aiAvailability } from "@/shared/domain/ai-availability";
import { type ProxyChoiceKey, proxyChoiceKey, proxyChoiceLabel } from "@/shared/domain/proxy";
import {
  isScripted,
  OBSERVE_SECONDS,
  VISIT_PAGES,
  VISITOR_BEHAVIORS,
  VISITOR_META,
} from "@/shared/domain/scan-options";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { cn } from "@/shared/lib/cn";
import { formatUsd, pluralize } from "@/shared/lib/format";
import { notifyError } from "@/shared/lib/notify-error";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/shared/ui/collapsible";
import { Input } from "@/shared/ui/input";
import { Label } from "@/shared/ui/label";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { Slider } from "@/shared/ui/slider";
import { Stepper } from "@/shared/ui/stepper";
import { Switch } from "@/shared/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { SCAN_URL_INPUT_ID, takePendingComposerFocus } from "./composer-focus";
import { estimateScanCost } from "./cost-estimate";
import { SaveConfigurationDialog } from "./SaveConfigurationDialog";
import {
  formFromOptions,
  newScanForm,
  type ProfileField,
  type ProxyField,
  type ScanForm,
  type ScanFormErrors,
  validateScanForm,
  withVisitor,
} from "./scan-form";

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
        <p className="text-xs text-pretty text-muted-foreground">{description}</p>
      </div>
      <Switch id={id} checked={checked} disabled={disabled} onCheckedChange={onCheckedChange} />
    </div>
  );
}

function SettingsLink({ children }: { children: ReactNode }) {
  return (
    <Link to="/settings" className="font-medium text-primary-text hover:underline">
      {children}
    </Link>
  );
}

function AiSummary({
  settings,
  ai,
  form,
  budgetUsd,
}: {
  settings: Settings;
  ai: AiAvailability;
  form: ScanForm;
  /** This scan's own cap, when the form sets a valid one. */
  budgetUsd: number | undefined;
}) {
  if (ai.triage === null) {
    return (
      <p className="text-xs text-muted-foreground">
        No API key: this scan runs the deterministic checks only. <SettingsLink>Add a key</SettingsLink> to analyse
        wrapper logic.
      </p>
    );
  }
  if (!form.aiEnabled) {
    return <p className="text-xs text-muted-foreground">Deterministic checks only. No LLM cost.</p>;
  }
  const extraPages = form.visitor === "browse" ? form.pages - 1 : 0;
  const estimate = estimateScanCost(settings, { extraPages, budgetUsd });
  const amount = <span className="font-medium text-foreground tabular-nums">{formatUsd(estimate.expectedUsd)}</span>;
  if (!ai.logicAnalysis) {
    return (
      <p className="text-xs text-muted-foreground">
        <SparklesIcon className="mr-1 inline size-3.5 -translate-y-px text-primary-text" />
        Estimated {amount} for Jev script triage. <SettingsLink>Add an Anthropic key</SettingsLink> to analyse wrapper
        logic.
      </p>
    );
  }
  return (
    <p className="text-xs text-muted-foreground">
      <SparklesIcon className="mr-1 inline size-3.5 -translate-y-px text-primary-text" />
      Estimated {amount} with {settings.analysisModel}
      {ai.triage === "jev" ? " and Jev script triage" : ""}
      {extraPages > 0 ? ` for ${pluralize(extraPages + 1, "page")}` : ""}, hard cap{" "}
      {formatUsd(budgetUsd ?? settings.budgetUsd)} per scan
      {estimate.cappedByBudget ? ". Analysis may stop early at the cap." : "."}
      {estimate.fallbackPricing ? " No list price is known for this model, so a conservative rate applies." : ""}
    </p>
  );
}

/** What the page can tell about the visit. Scripted visits never pretend to be a person. */
function AutomationNote({ form }: { form: ScanForm }) {
  if (!isScripted(form.visitor)) {
    return (
      <p className="text-xs text-pretty text-muted-foreground">
        Chrome opens; browse the site, then press <span className="font-medium text-foreground">Finish capture</span>{" "}
        (up to 15 minutes). You drive the browser and handle the consent banner yourself. The page can still notice the
        debugger that records it.
      </p>
    );
  }
  return (
    <p className="text-xs text-pretty text-muted-foreground">
      This visit is automated and says so to the page (navigator.webdriver is true). Ad partners can treat it
      differently from a person's visit.
    </p>
  );
}

const PROFILE_LABEL: Record<Exclude<ProfileField, "settings">, string> = {
  fresh: "Fresh profile",
  kept: "Kept scan profile",
};

function AdvancedOptions({
  form,
  settings,
  budgetError,
  proxyMissing,
  update,
}: {
  form: ScanForm;
  settings: Settings;
  budgetError: string | undefined;
  /** The form names a proxy profile that has been deleted. */
  proxyMissing: boolean;
  update: (patch: Partial<ScanForm>) => void;
}) {
  const ids = { budget: useId(), budgetHelp: useId(), proxyError: useId() };
  const settingsProfile = settings.persistentProfile ? "kept" : "fresh";
  return (
    <div className="grid grid-cols-3 gap-x-6 gap-y-2 pt-3">
      <div className="flex min-w-0 flex-col gap-1.5">
        <Label>Proxy</Label>
        <Select value={form.proxy} onValueChange={(proxy) => update({ proxy: proxy as ProxyField })}>
          <SelectTrigger
            aria-label="Proxy"
            aria-invalid={proxyMissing ? true : undefined}
            aria-describedby={proxyMissing ? ids.proxyError : undefined}
            className={cn("w-full", proxyMissing && "border-destructive")}
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem value="settings">
                Settings default ({proxyChoiceLabel(settings.defaultProxy, settings.proxies)})
              </SelectItem>
              <SelectItem value="none">No proxy</SelectItem>
              <SelectItem value="system">System proxy</SelectItem>
              {settings.proxies.map((proxy) => (
                <SelectItem key={proxy.id} value={`profile:${proxy.id}` satisfies ProxyChoiceKey}>
                  {proxy.name}
                </SelectItem>
              ))}
              {proxyMissing ? (
                <SelectItem value={form.proxy} disabled>
                  Deleted proxy
                </SelectItem>
              ) : null}
            </SelectGroup>
          </SelectContent>
        </Select>
        {proxyMissing ? (
          <p id={ids.proxyError} role="alert" className="text-xs text-red-fg">
            This proxy was deleted. Choose another one.
          </p>
        ) : (
          <p className="text-xs text-muted-foreground">
            <SettingsLink>Manage proxies</SettingsLink>
          </p>
        )}
      </div>
      <div className="flex min-w-0 flex-col gap-1.5">
        <Label>Browser profile</Label>
        <Select value={form.profile} onValueChange={(profile) => update({ profile: profile as ProfileField })}>
          <SelectTrigger aria-label="Browser profile" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem value="settings">
                Settings default ({PROFILE_LABEL[settingsProfile].toLowerCase()})
              </SelectItem>
              <SelectItem value="fresh">{PROFILE_LABEL.fresh}</SelectItem>
              <SelectItem value="kept">{PROFILE_LABEL.kept}</SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
        <p className="text-xs text-pretty text-muted-foreground">
          {(form.profile === "settings" ? settingsProfile : form.profile) === "kept"
            ? "Cookies and consent carry over from earlier scans."
            : "Starts without cookies, like a first-time visitor."}
        </p>
      </div>
      <div className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={ids.budget}>LLM budget</Label>
        <div className="relative">
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
        <p
          id={ids.budgetHelp}
          role={budgetError ? "alert" : undefined}
          className={cn("text-xs text-pretty", budgetError ? "text-red-fg" : "text-muted-foreground")}
        >
          {budgetError ?? "Hard cap for this scan. Empty uses the settings."}
        </p>
      </div>
    </div>
  );
}

/** The loaded configuration: its name, whether the options differ from what is saved, and how to save them. */
function ConfigurationBar({
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

export function ScanComposer({
  settings,
  configuration,
}: {
  settings: Settings;
  /** A saved configuration opened for editing; the composer starts from its options. */
  configuration?: ScanConfiguration;
}) {
  const navigate = useNavigate();
  const { launch, launchConfiguration, isPending } = useLaunchScan();
  const saveConfiguration = useSaveScanConfiguration();
  const ids = { error: useId(), observe: useId(), pages: useId(), visitor: useId() };
  const saved = configuration ? formFromOptions(configuration.options, settings) : null;
  const [form, setForm] = useState<ScanForm>(() => saved ?? newScanForm(settings));
  const [errors, setErrors] = useState<ScanFormErrors>({});
  const [optionsOpen, setOptionsOpen] = useState(configuration !== undefined);
  const [advancedOpen, setAdvancedOpen] = useState(
    () => form.proxy !== "settings" || form.profile !== "settings" || form.budget !== "",
  );
  const [savingAsNew, setSavingAsNew] = useState(false);

  const ai = aiAvailability(settings);
  const effective = { ...form, aiEnabled: form.aiEnabled && ai.triage !== null };
  const triageOnly = ai.triage !== null && !ai.logicAnalysis;
  const manual = form.visitor === "manual";
  const browse = form.visitor === "browse";
  const validation = validateScanForm(effective);
  const proxyMissing =
    form.proxy.startsWith("profile:") &&
    !settings.proxies.some((proxy) => proxyChoiceKey({ kind: "profile", id: proxy.id }) === form.proxy);
  const edited = saved !== null && JSON.stringify(form) !== JSON.stringify(saved);

  useEffect(() => {
    if (takePendingComposerFocus()) document.getElementById(SCAN_URL_INPUT_ID)?.focus();
  }, []);

  const update = (patch: Partial<ScanForm>) => {
    setForm((current) => ({ ...current, ...patch }));
    if (patch.url !== undefined) setErrors((current) => ({ ...current, url: undefined }));
    if (patch.budget !== undefined) setErrors((current) => ({ ...current, budget: undefined }));
  };

  /** The options to scan or save, or `null` after pointing at what has to be fixed first. */
  const validOptions = () => {
    if (validation.ok && !proxyMissing) return validation.options;
    setErrors(validation.ok ? {} : validation.errors);
    if (proxyMissing || (!validation.ok && validation.errors.budget)) {
      setOptionsOpen(true);
      setAdvancedOpen(true);
    }
    return null;
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const options = validOptions();
    if (!options) return;
    // Unchanged, the scan is a run of the configuration; with edits it is a scan of its own.
    if (configuration && !edited) launchConfiguration(configuration.id);
    else launch(options);
  };

  const updateConfiguration = () => {
    const options = validOptions();
    if (!configuration || !options) return;
    saveConfiguration.mutate(
      { id: configuration.id, name: configuration.name, options },
      {
        onSuccess: (updated) => {
          // The saved URL is the normalised one; show it, so the form matches what is stored.
          setForm(formFromOptions(updated.options, settings));
          toast.success(`Updated “${updated.name}”`);
        },
        onError: (reason) => notifyError("Could not update the configuration", reason),
      },
    );
  };

  return (
    <form
      onSubmit={submit}
      noValidate
      aria-label="New scan"
      className="rounded-xl border bg-card shadow-xs transition-shadow focus-within:shadow-pop"
    >
      {configuration ? (
        <ConfigurationBar
          configuration={configuration}
          edited={edited}
          saving={saveConfiguration.isPending}
          onUpdate={updateConfiguration}
          onSaveAsNew={() => validOptions() && setSavingAsNew(true)}
        />
      ) : null}
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
            aria-invalid={errors.url ? true : undefined}
            aria-describedby={errors.url ? ids.error : undefined}
            placeholder="publisher.com/article"
            value={form.url}
            onChange={(event) => update({ url: event.target.value })}
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

      {errors.url ? (
        <p id={ids.error} role="alert" className="px-5 pb-2 text-[13px] text-red-fg">
          {errors.url}
        </p>
      ) : null}
      {configuration && edited ? (
        <p className="px-5 pb-2 text-xs text-muted-foreground">
          Scan runs these edited options once. Update the configuration to keep them.
        </p>
      ) : null}

      <Collapsible open={optionsOpen} onOpenChange={setOptionsOpen} className="border-t">
        <div className="flex min-h-10 items-center justify-between gap-4 px-4 py-2">
          <AiSummary
            settings={settings}
            ai={ai}
            form={effective}
            budgetUsd={validation.ok ? validation.options.budgetUsd : undefined}
          />
          <div className="-mr-2 flex shrink-0 items-center gap-1">
            <span className="text-xs whitespace-nowrap text-muted-foreground">
              {VISITOR_META[form.visitor].label}
              {browse ? `, ${pluralize(form.pages, "page")}` : ""}
            </span>
            <CollapsibleTrigger asChild>
              <Button variant="ghost" size="sm">
                Options
                <ChevronDownIcon className={cn("transition-transform", optionsOpen && "rotate-180")} />
              </Button>
            </CollapsibleTrigger>
          </div>
        </div>
        <CollapsibleContent>
          <div className="grid grid-cols-2 gap-x-10 gap-y-4 border-t px-4 py-4">
            <div className="col-span-2 flex flex-col gap-2">
              <Label id={ids.visitor}>Visitor</Label>
              <ToggleGroup
                aria-labelledby={ids.visitor}
                value={form.visitor}
                onValueChange={(visitor) => setForm((current) => withVisitor(current, visitor, settings))}
                className="self-start"
              >
                {VISITOR_BEHAVIORS.map((visitor) => (
                  <ToggleGroupItem key={visitor} value={visitor}>
                    {VISITOR_META[visitor].label}
                  </ToggleGroupItem>
                ))}
              </ToggleGroup>
              <p className="text-xs text-pretty text-foreground">{VISITOR_META[form.visitor].description}</p>
              <AutomationNote form={form} />
            </div>

            {manual ? null : (
              <div className="flex flex-col gap-2">
                <div className="flex items-baseline justify-between">
                  <Label htmlFor={ids.observe}>{browse ? "Time on each page" : "Observe after load"}</Label>
                  <span className="text-[13px] text-muted-foreground tabular-nums">{form.observeSeconds} s</span>
                </div>
                <Slider
                  id={ids.observe}
                  aria-label={browse ? "Time on each page, seconds" : "Observe after load, seconds"}
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
            )}
            {browse ? (
              <div className="flex items-start justify-between gap-4">
                <div className="flex flex-col gap-0.5">
                  <Label htmlFor={ids.pages}>Pages to visit</Label>
                  <p className="text-xs text-pretty text-muted-foreground">
                    {form.pages === 1
                      ? "The entry page only; no link is followed."
                      : `The entry page, then up to ${pluralize(form.pages - 1, "link")} on the same site. Ads, sign-in and checkout links are never clicked.`}
                  </p>
                </div>
                <Stepper
                  id={ids.pages}
                  value={form.pages}
                  min={VISIT_PAGES.min}
                  max={VISIT_PAGES.max}
                  onValueChange={(pages) => update({ pages })}
                  decreaseLabel="Fewer pages"
                  increaseLabel="More pages"
                >
                  {pluralize(form.pages, "page")}
                </Stepper>
              </div>
            ) : manual ? null : (
              <div />
            )}

            {manual ? null : (
              <>
                <OptionSwitch
                  label="Show the browser"
                  description="Watch the visit in a Chrome window. Headless Chrome is easier for ad partners to recognise as automated."
                  checked={form.showBrowser}
                  onCheckedChange={(showBrowser) => update({ showBrowser })}
                />
                <OptionSwitch
                  label="Accept consent banner"
                  description="Click through the CMP so bidding is not blocked."
                  checked={form.acceptConsent}
                  onCheckedChange={(acceptConsent) => update({ acceptConsent })}
                />
              </>
            )}
            <OptionSwitch
              label={triageOnly ? "AI script triage" : "AI logic analysis"}
              description={
                triageOnly
                  ? "Classify scripts with Jev. Logic analysis needs an Anthropic key."
                  : "Classify scripts, then hunt for wrapper logic bugs."
              }
              checked={effective.aiEnabled}
              disabled={ai.triage === null}
              onCheckedChange={(aiEnabled) => update({ aiEnabled })}
            />
            <div className="flex flex-col gap-0.5">
              <span className="text-[13px] leading-none font-medium">Scan profile</span>
              <p className="text-xs text-pretty text-muted-foreground">
                {(form.profile === "settings" ? settings.persistentProfile : form.profile === "kept")
                  ? "Kept between scans: cookies and consent persist, like a returning visitor's."
                  : "A fresh profile for this scan: no cookies, like a first-time visitor."}{" "}
                {form.profile === "settings" ? <SettingsLink>Change in Settings</SettingsLink> : null}
              </p>
            </div>

            <Collapsible open={advancedOpen} onOpenChange={setAdvancedOpen} className="col-span-2 border-t pt-2">
              <div className="flex items-center justify-between gap-4">
                <CollapsibleTrigger asChild>
                  <Button variant="ghost" size="sm" className="-ml-2">
                    <ChevronDownIcon className={cn("transition-transform", !advancedOpen && "-rotate-90")} />
                    Proxy, profile and budget
                    {proxyMissing ? <TriangleAlertIcon aria-label="Needs attention" className="text-amber-fg" /> : null}
                  </Button>
                </CollapsibleTrigger>
                {configuration ? null : (
                  <Button variant="outline" size="sm" onClick={() => validOptions() && setSavingAsNew(true)}>
                    <BookmarkIcon />
                    Save as configuration…
                  </Button>
                )}
              </div>
              <CollapsibleContent>
                <AdvancedOptions
                  form={form}
                  settings={settings}
                  budgetError={errors.budget}
                  proxyMissing={proxyMissing}
                  update={update}
                />
              </CollapsibleContent>
            </Collapsible>
          </div>
        </CollapsibleContent>
      </Collapsible>

      {savingAsNew && validation.ok ? (
        <SaveConfigurationDialog
          options={validation.options}
          onClose={() => setSavingAsNew(false)}
          onSaved={(created) => navigate({ to: "/", search: { configuration: created.id } })}
        />
      ) : null}
    </form>
  );
}

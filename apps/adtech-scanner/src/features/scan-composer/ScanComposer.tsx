import { useBlocker, useNavigate } from "@tanstack/react-router";
import { BookmarkIcon, ChevronDownIcon, GlobeIcon, MonitorIcon, ScanSearchIcon, SmartphoneIcon } from "lucide-react";
import { type FormEvent, useEffect, useId, useRef, useState } from "react";
import { toast } from "sonner";
import type { ScanConfiguration, Settings } from "@/bindings";
import { useSaveScanConfiguration } from "@/lib/query/hooks";
import { aiAvailability } from "@/shared/domain/ai-availability";
import { estimateBrowserbaseUsage, reachableFromCloud } from "@/shared/domain/browserbase";
import { proxyChoiceKey } from "@/shared/domain/proxy";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { cn } from "@/shared/lib/cn";
import { notifyError } from "@/shared/lib/notify-error";
import { hasModifier } from "@/shared/lib/platform";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/shared/ui/collapsible";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { AnalysisOptions } from "./AnalysisOptions";
import { ComposerSummary } from "./ComposerSummary";
import { ConfigurationBar } from "./ConfigurationBar";
import { SCAN_URL_INPUT_ID, takePendingComposerFocus } from "./composer-focus";
import { OriginOptions } from "./OriginOptions";
import { SaveConfigurationDialog } from "./SaveConfigurationDialog";
import {
  formFromOptions,
  newScanForm,
  proxyFieldFor,
  type ScanForm,
  type ScanFormErrors,
  sameScan,
  validateScanForm,
  withBrowser,
  withVisitor,
} from "./scan-form";
import { scanSummary } from "./scan-summary";
import { normalizeScanUrl } from "./scan-url";
import { VisitOptions } from "./VisitOptions";

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
  const errorId = useId();
  const summaryId = useId();
  // What the loaded configuration holds, as a form: the composer's edits are measured against it.
  const [saved, setSaved] = useState(() => (configuration ? formFromOptions(configuration.options, settings) : null));
  const [form, setForm] = useState<ScanForm>(() => saved ?? newScanForm(settings));
  const [errors, setErrors] = useState<ScanFormErrors>({});
  const [optionsOpen, setOptionsOpen] = useState(configuration !== undefined);
  const [savingAsNew, setSavingAsNew] = useState(false);

  const ai = aiAvailability(settings);
  const effective = { ...form, aiEnabled: form.aiEnabled && ai.triage !== null };
  const validation = validateScanForm(effective);
  // What a configuration keeps is the AI choice as made, also while no key is stored to act on it.
  const savable = validateScanForm(form);
  const proxyMissing =
    form.proxy.startsWith("profile:") &&
    !settings.proxies.some((proxy) => proxyChoiceKey({ kind: "profile", id: proxy.id }) === form.proxy);
  const typedUrl = normalizeScanUrl(form.url);
  const unreachable = typedUrl.ok && !reachableFromCloud(typedUrl.url);
  const wantsBrowserbase = form.browser === "browserbase" && !unreachable;
  // A configuration can name Browserbase after its credentials were removed: nothing runs until that changes.
  const browserMissing = wantsBrowserbase && !settings.browserbase.connected;
  const remote = wantsBrowserbase && !browserMissing;
  const blocked = proxyMissing || browserMissing;
  const cloudProxy = proxyFieldFor(form, true);
  const usage = remote
    ? estimateBrowserbaseUsage({
        visitor: form.visitor,
        pages: form.visitor === "browse" ? form.pages : 1,
        observeSeconds: form.observeSeconds,
        browserbaseProxy:
          cloudProxy === "browserbaseLocation" ||
          (cloudProxy === "settings" && settings.browserbase.defaultProxy.kind === "browserbaseLocation"),
      })
    : null;
  const edited = saved !== null && !sameScan(form, saved);
  // Set when the composer itself moves on (a scan started, a new configuration opened): no question then.
  const leaving = useRef(false);
  const unsaved = useBlocker({
    shouldBlockFn: () => edited && !leaving.current,
    enableBeforeUnload: false,
    withResolver: true,
  });

  useEffect(() => {
    if (takePendingComposerFocus()) document.getElementById(SCAN_URL_INPUT_ID)?.focus();
  }, []);

  const update = (patch: Partial<ScanForm>) => {
    leaving.current = false;
    setForm((current) => ({ ...current, ...patch }));
    if (patch.url !== undefined) setErrors((current) => ({ ...current, url: undefined }));
    if (patch.budget !== undefined) setErrors((current) => ({ ...current, budget: undefined }));
    if (Object.keys(patch).some((field) => field.startsWith("location") || field === "proxy")) {
      setErrors((current) => ({ ...current, location: undefined }));
    }
  };

  /** The options to scan or save, or `null` after pointing at what has to be fixed first. */
  const validOptions = () => {
    if (validation.ok && !blocked) return validation.options;
    setErrors(validation.ok ? {} : validation.errors);
    // These are fixed inside the options.
    if (blocked || (!validation.ok && (validation.errors.budget || validation.errors.location))) {
      setOptionsOpen(true);
    }
    return null;
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const options = validOptions();
    if (!options) return;
    // Unchanged, the scan is a run of the configuration; with edits it is a scan of its own.
    if (configuration && !edited) launchConfiguration(configuration.id);
    else {
      leaving.current = true;
      launch(options);
    }
  };

  const updateConfiguration = () => {
    if (!configuration || !validOptions() || !savable.ok) return;
    saveConfiguration.mutate(
      { id: configuration.id, name: configuration.name, options: savable.options },
      {
        onSuccess: (updated) => {
          // The saved URL is the normalised one; show it, so the form matches what is stored.
          const stored = formFromOptions(updated.options, settings);
          setSaved(stored);
          setForm(stored);
          toast.success(`Updated “${updated.name}”`);
        },
        onError: (reason) => notifyError("Could not update the configuration", reason),
      },
    );
  };

  const saveAsNew = () => {
    if (validOptions()) setSavingAsNew(true);
  };

  // Mod+Enter scans and Mod+S keeps the options (into the open configuration, or as a new one) from anywhere
  // on the screen the composer is on. A dialog, a menu and a field that is not the composer's keep their keys.
  const formRef = useRef<HTMLFormElement>(null);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const form = formRef.current;
      if (!form || event.defaultPrevented || !hasModifier(event) || event.altKey || event.shiftKey) return;
      const key = event.key.toLowerCase();
      if (key !== "enter" && key !== "s") return;
      const target = event.target as HTMLElement;
      const ownField = form.contains(target);
      const otherField = target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName);
      const overlayOpen =
        document.querySelector("[role='dialog'], [role='alertdialog'], [role='menu'], [role='listbox']") !== null;
      if (overlayOpen || (otherField && !ownField)) return;
      event.preventDefault();
      if (key === "enter") form.requestSubmit();
      else if (!configuration) saveAsNew();
      else if (edited) updateConfiguration();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  const budgetUsd = validation.ok ? validation.options.budgetUsd : undefined;

  return (
    <form
      ref={formRef}
      onSubmit={submit}
      noValidate
      aria-label="New scan"
      className="overflow-hidden rounded-xl border bg-card shadow-xs transition-shadow focus-within:shadow-pop"
    >
      {configuration ? (
        <ConfigurationBar
          configuration={configuration}
          edited={edited}
          blocked={proxyMissing ? "proxy" : browserMissing ? "browserbase" : null}
          saving={saveConfiguration.isPending}
          onRevert={() => saved && setForm(saved)}
          onUpdate={updateConfiguration}
          onSaveAsNew={saveAsNew}
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
            aria-describedby={errors.url ? errorId : undefined}
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
        <Button type="submit" size="lg" disabled={isPending} aria-describedby={summaryId} className="h-9">
          <ScanSearchIcon />
          Scan
        </Button>
      </div>

      {errors.url ? (
        <p id={errorId} role="alert" className="px-5 pb-2 text-[13px] text-red-fg">
          {errors.url}
        </p>
      ) : null}
      <Collapsible open={optionsOpen} onOpenChange={setOptionsOpen} className="border-t">
        <div className="flex min-h-11 items-center justify-between gap-4 py-2 pr-2 pl-4">
          <ComposerSummary
            id={summaryId}
            parts={scanSummary({ form: effective, settings, ai, budgetUsd, proxyMissing, remote, browserMissing })}
            needsKey={ai.triage === null}
          />
          <div className="flex shrink-0 items-center gap-1">
            {configuration ? null : (
              <Button variant="ghost" size="sm" onClick={saveAsNew}>
                <BookmarkIcon />
                Save as configuration…
              </Button>
            )}
            <CollapsibleTrigger asChild>
              <Button variant="ghost" size="sm">
                Options
                <ChevronDownIcon className={cn("transition-transform", optionsOpen && "rotate-180")} />
              </Button>
            </CollapsibleTrigger>
          </div>
        </div>
        <CollapsibleContent>
          <VisitOptions
            form={form}
            // A configuration that names Browserbase shows what it holds, also while it cannot run there.
            remote={remote || browserMissing}
            update={update}
            onVisitorChange={(visitor) => {
              leaving.current = false;
              setForm((current) => withVisitor(current, visitor, settings));
            }}
          />
          <OriginOptions
            form={form}
            settings={settings}
            remote={remote}
            unreachable={unreachable}
            browserMissing={browserMissing}
            proxyMissing={proxyMissing}
            locationError={errors.location}
            usage={usage}
            update={update}
            onBrowserChange={(browser) => {
              leaving.current = false;
              setForm((current) => withBrowser(current, browser));
            }}
          />
          <AnalysisOptions
            form={form}
            settings={settings}
            ai={ai}
            aiEnabled={effective.aiEnabled}
            budgetUsd={budgetUsd}
            budgetError={errors.budget}
            usage={settings.browserbase.connected ? usage : undefined}
            update={update}
          />
        </CollapsibleContent>
      </Collapsible>

      {savingAsNew && savable.ok ? (
        <SaveConfigurationDialog
          options={savable.options}
          onClose={() => setSavingAsNew(false)}
          onSaved={(created) => {
            leaving.current = true;
            void navigate({ to: "/", search: { configuration: created.id } });
          }}
        />
      ) : null}
      <AlertDialog open={unsaved.status === "blocked"} onOpenChange={(open) => !open && unsaved.reset?.()}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard the changes to “{configuration?.name}”?</AlertDialogTitle>
            <AlertDialogDescription>
              The options you changed are not saved in the configuration yet. Update the configuration to keep them.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction destructive onClick={() => unsaved.proceed?.()}>
              Discard changes
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </form>
  );
}

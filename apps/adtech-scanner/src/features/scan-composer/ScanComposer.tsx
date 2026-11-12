import { useBlocker, useNavigate } from "@tanstack/react-router";
import {
  BookmarkIcon,
  ChevronDownIcon,
  GlobeIcon,
  MonitorIcon,
  ScanSearchIcon,
  SmartphoneIcon,
  TriangleAlertIcon,
} from "lucide-react";
import { type FormEvent, useEffect, useId, useRef, useState } from "react";
import { toast } from "sonner";
import type { ScanConfiguration, Settings } from "@/bindings";
import { useSaveScanConfiguration } from "@/lib/query/hooks";
import { aiAvailability } from "@/shared/domain/ai-availability";
import { proxyChoiceKey } from "@/shared/domain/proxy";
import { VISITOR_META } from "@/shared/domain/scan-options";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { cn } from "@/shared/lib/cn";
import { pluralize } from "@/shared/lib/format";
import { notifyError } from "@/shared/lib/notify-error";
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
import { AdvancedOptions } from "./AdvancedOptions";
import { AiSummary } from "./AiSummary";
import { ConfigurationBar } from "./ConfigurationBar";
import { SCAN_URL_INPUT_ID, takePendingComposerFocus } from "./composer-focus";
import { SaveConfigurationDialog } from "./SaveConfigurationDialog";
import {
  formFromOptions,
  newScanForm,
  type ScanForm,
  type ScanFormErrors,
  sameScan,
  validateScanForm,
  withVisitor,
} from "./scan-form";
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
  // What the loaded configuration holds, as a form: the composer's edits are measured against it.
  const [saved, setSaved] = useState(() => (configuration ? formFromOptions(configuration.options, settings) : null));
  const [form, setForm] = useState<ScanForm>(() => saved ?? newScanForm(settings));
  const [errors, setErrors] = useState<ScanFormErrors>({});
  const [optionsOpen, setOptionsOpen] = useState(configuration !== undefined);
  const [advancedOpen, setAdvancedOpen] = useState(
    () => form.proxy !== "settings" || form.profile !== "settings" || form.budget !== "",
  );
  const [savingAsNew, setSavingAsNew] = useState(false);

  const ai = aiAvailability(settings);
  const effective = { ...form, aiEnabled: form.aiEnabled && ai.triage !== null };
  const browse = form.visitor === "browse";
  const validation = validateScanForm(effective);
  // What a configuration keeps is the AI choice as made, also while no key is stored to act on it.
  const savable = validateScanForm(form);
  const proxyMissing =
    form.proxy.startsWith("profile:") &&
    !settings.proxies.some((proxy) => proxyChoiceKey({ kind: "profile", id: proxy.id }) === form.proxy);
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
        <Button type="submit" size="lg" disabled={isPending} className="h-9">
          <ScanSearchIcon />
          Scan
        </Button>
      </div>

      {errors.url ? (
        <p id={errorId} role="alert" className="px-5 pb-2 text-[13px] text-red-fg">
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
          <VisitOptions
            form={form}
            settings={settings}
            ai={ai}
            aiEnabled={effective.aiEnabled}
            update={update}
            onVisitorChange={(visitor) => {
              leaving.current = false;
              setForm((current) => withVisitor(current, visitor, settings));
            }}
          >
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
          </VisitOptions>
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

import { CheckCircle2Icon } from "lucide-react";
import { type FormEvent, type ReactNode, useId, useState } from "react";
import { toast } from "sonner";
import type {
  BrowserbaseProbe,
  BrowserbaseProject,
  BrowserbaseRegion,
  BrowserbaseStatus,
  Settings,
  SettingsPatch,
} from "@/bindings";
import { toApiError } from "@/lib/api";
import {
  useBrowserbaseProfile,
  useClearBrowserbaseCredentials,
  useClearBrowserbaseProfile,
  useProbeBrowserbase,
  useSetBrowserbaseCredentials,
  useTestBrowserbase,
} from "@/lib/query/hooks";
import { KeyForm, type KeySaveResult } from "@/shared/components/api-key-form";
import {
  BROWSERBASE_DISCLOSURE,
  BROWSERBASE_RATES,
  BROWSERBASE_REGION_META,
  BROWSERBASE_REGIONS,
  cityName,
  formatTraffic,
  locationLabel,
  parseLocation,
  regionOptionLabel,
  regionPlace,
} from "@/shared/domain/browserbase";
import { proxyChoiceFromKey, proxyChoiceKey, type SavedProxyKey } from "@/shared/domain/proxy";
import { formatInteger, formatRelativeTime, pluralize } from "@/shared/lib/format";
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
  AlertDialogTrigger,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Field } from "@/shared/ui/field";
import { Input } from "@/shared/ui/input";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { Switch } from "@/shared/ui/switch";

type Save = (patch: SettingsPatch) => void;

/** The key field, and the choice between projects for a key that has more than one. */
function Connect() {
  const setCredentials = useSetBrowserbaseCredentials();
  const projectLabelId = useId();
  const [projects, setProjects] = useState<BrowserbaseProject[]>([]);
  const [projectId, setProjectId] = useState<string | undefined>(undefined);

  const save = async (apiKey: string): Promise<KeySaveResult> => {
    try {
      const status = await setCredentials.mutateAsync({ apiKey, projectId });
      if (status.valid) {
        toast.success("Browserbase key validated and saved to your keychain");
        return { saved: true };
      }
      setProjects(status.projects);
      return { saved: false, reason: status.message ?? "Browserbase did not accept the API key." };
    } catch (reason) {
      notifyError("Could not save the Browserbase key", reason);
      return { saved: false };
    }
  };

  return (
    <KeyForm
      name="Browserbase"
      placeholder="bb_live_…"
      keyNoun="Browserbase key"
      submitLabel="Save Browserbase key"
      pending={setCredentials.isPending}
      onSave={save}
    >
      {projects.length > 1 ? (
        <div className="flex items-center gap-3">
          <span id={projectLabelId} className="text-[13px] font-medium">
            Project
          </span>
          <Select value={projectId ?? ""} onValueChange={setProjectId}>
            <SelectTrigger aria-labelledby={projectLabelId} className="w-56">
              <SelectValue placeholder="Choose a project" />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                {projects.map((project) => (
                  <SelectItem key={project.id} value={project.id}>
                    {project.name}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        </div>
      ) : null}
    </KeyForm>
  );
}

function Connected({ projectName }: { projectName: string | null }) {
  const clear = useClearBrowserbaseCredentials();
  return (
    <div className="flex items-center justify-between gap-4 rounded-lg border border-green-line bg-green-soft/60 px-3.5 py-2.5">
      <p className="flex items-center gap-2 text-[13px] text-green-fg">
        <CheckCircle2Icon className="size-4 shrink-0" />
        <span>
          Connected to the project {projectName ? `“${projectName}”` : "of this key"}. The key is in your system
          keychain.
        </span>
      </p>
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button variant="outline" size="sm" disabled={clear.isPending}>
            Remove Browserbase key
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove the Browserbase key?</AlertDialogTitle>
            <AlertDialogDescription>
              The key and its project are deleted from your keychain. Scans run on this Mac again, and saved
              configurations that use Browserbase cannot run until a key is added. A browser profile kept at Browserbase
              is deleted there too, if Browserbase can be reached.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              destructive
              onClick={() =>
                clear.mutate(undefined, {
                  onSuccess: () => toast.success("Browserbase key removed"),
                  onError: (reason) => notifyError("Could not remove the Browserbase key", reason),
                })
              }
            >
              Remove Browserbase key
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function ProfileState() {
  const profile = useBrowserbaseProfile();
  const clear = useClearBrowserbaseProfile();
  if (!profile.data) return null;
  const { exists, lastUsedAt, inUse } = profile.data;
  if (!exists) {
    return <p className="text-[13px] text-muted-foreground">No browser profile is kept at Browserbase.</p>;
  }
  return (
    <div className="flex w-full items-center justify-between gap-4">
      <p className="text-[13px] text-muted-foreground" aria-live="polite">
        A browser profile is kept at Browserbase
        {lastUsedAt === null ? "" : `, last used ${formatRelativeTime(lastUsedAt)}`}
        {inUse ? ". A running scan is using it." : "."}
      </p>
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button variant="outline" size="sm" disabled={inUse || clear.isPending}>
            Clear Browserbase profile…
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Clear the browser profile at Browserbase?</AlertDialogTitle>
            <AlertDialogDescription>
              Its cookies, consent choices and site storage are deleted at Browserbase. The next Browserbase scan starts
              like a first-time visitor. The profile on this Mac is not touched.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              destructive
              onClick={() =>
                clear.mutate(undefined, {
                  onSuccess: () => toast.success("Browserbase profile cleared"),
                  onError: (reason) => notifyError("Could not clear the Browserbase profile", reason),
                })
              }
            >
              Clear profile
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/**
 * Browserbase as a second place for a scan's browser to run: what it means for the scanned pages, the
 * credentials, two checks of the account, and what a Browserbase scan starts from.
 */
export function BrowserbaseSection({ settings, save }: { settings: Settings; save: Save }) {
  const ids = { disclosure: useId(), keep: useId(), keepNote: useId() };
  const { browserbase } = settings;
  return (
    <>
      <Field stacked label="Browserbase account">
        <div className="flex w-full flex-col gap-3">
          <p
            id={ids.disclosure}
            role="note"
            aria-label="What using Browserbase means"
            className="selectable rounded-lg border bg-muted/40 px-3.5 py-3 text-[13px] text-pretty text-muted-foreground"
          >
            {BROWSERBASE_DISCLOSURE}
          </p>
          {browserbase.connected ? <Connected projectName={browserbase.projectName} /> : <Connect />}
        </div>
      </Field>
      {browserbase.connected ? (
        <>
          <TestConnection />
          <PageProbe region={browserbase.region} />
          <Field
            label="Default region"
            description="Where Browserbase starts the browser. The region decides both speed (one near you answers faster) and, without a proxy, where the visit comes from."
          >
            <Select
              value={browserbase.region}
              onValueChange={(region) => save({ browserbase: { region: region as BrowserbaseRegion } })}
            >
              <SelectTrigger aria-label="Default region" className="w-64">
                <SelectValue />
              </SelectTrigger>
              <SelectContent align="end">
                <SelectGroup>
                  {BROWSERBASE_REGIONS.map((region) => (
                    <SelectItem key={region} value={region}>
                      {regionOptionLabel(region)}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </Field>
          <DefaultLocation settings={settings} save={save} />
          <Field
            label="Keep a browser profile on Browserbase"
            htmlFor={ids.keep}
            description={
              <span id={ids.keepNote}>
                On: Browserbase scans share one profile per device type that Browserbase stores: cookies, consent
                choices and site storage, until you clear it here. Off: every Browserbase scan starts fresh, whatever
                the setting for this Mac's browser says.
              </span>
            }
          >
            <Switch
              id={ids.keep}
              aria-describedby={ids.keepNote}
              checked={browserbase.persistentProfile}
              onCheckedChange={(persistentProfile) => save({ browserbase: { persistentProfile } })}
            />
          </Field>
          <Field stacked label="Browser profile at Browserbase">
            <ProfileState />
          </Field>
        </>
      ) : null}
    </>
  );
}

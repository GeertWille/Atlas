import { FolderOpenIcon, MonitorIcon, MoonIcon, SunIcon } from "lucide-react";
import { type ReactNode, useId } from "react";
import { toast } from "sonner";
import type { BrowserInfo, Settings, SettingsPatch } from "@/bindings";
import { useDeleteAllScans, useUpdateSettings } from "@/lib/query/hooks";
import { Panel } from "@/shared/components/panel";
import { notifyError } from "@/shared/lib/notify-error";
import { chooseBrowserExecutable } from "@/shared/lib/system";
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
import { Field, FieldGroup } from "@/shared/ui/field";
import { Skeleton } from "@/shared/ui/skeleton";
import { Switch } from "@/shared/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { AiAnalysisFields } from "./AiAnalysisFields";
import { ProxySection } from "./ProxySection";
import { ScanDefaultsFields } from "./ScanDefaultsFields";
import { ScanProfileField } from "./ScanProfileField";
import { StoredDataFields } from "./StoredDataFields";

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
            Every report, finding and audit trail is removed from this computer, together with the stored scan data.
            Settings and your API keys are kept. This cannot be undone.
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
  const ids = { headless: useId(), sellersJson: useId(), sellersJsonDisclosure: useId() };

  const save = (patch: SettingsPatch) =>
    updateSettings.mutate(patch, {
      onSuccess: () => toast.success("Settings saved", { id: "settings-saved", duration: 1500 }),
      onError: (reason) => notifyError("Could not save settings", reason),
    });

  return (
    <>
      <Section
        title="AI analysis"
        description="Two model phases find logic bugs in wrapper code. Without a key, scans still run every deterministic check."
      >
        <AiAnalysisFields settings={settings} save={save} />
      </Section>

      <Section
        title="Bidder checks"
        description="Every scan verifies each bidder's adapter params, its endpoint's answers, the consent it was sent and the scanned site's own ads.txt."
      >
        <Field
          label="Check sellers.json at ad exchanges"
          htmlFor={ids.sellersJson}
          description={
            <span id={ids.sellersJsonDisclosure}>
              Requests sellers.json from the ad exchanges the scanned site's ads.txt names (appnexus.com,
              rubiconproject.com and the like) to verify its seller ids. Off: only the scanned site is contacted.
            </span>
          }
        >
          <Switch
            id={ids.sellersJson}
            aria-describedby={ids.sellersJsonDisclosure}
            checked={settings.checkSellersJson}
            onCheckedChange={(checkSellersJson) => save({ checkSellersJson })}
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
          description="Hide the browser window while a scan observes or scrolls. Browse visits and manual sessions show it unless a scan says otherwise. Headless Chrome is easier for ad partners to recognise as automated."
        >
          <Switch id={ids.headless} checked={settings.headless} onCheckedChange={(headless) => save({ headless })} />
        </Field>
        <ScanProfileField settings={settings} save={save} />
      </Section>

      <Section
        title="Proxies"
        description="Scan a site as it is served somewhere else: save a proxy per country and choose it per scan."
      >
        <ProxySection settings={settings} save={save} />
      </Section>

      <Section
        title="Scan defaults"
        description="What a new scan starts from. A scan's Options and a saved configuration can set each of these differently."
      >
        <ScanDefaultsFields settings={settings} save={save} />
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
        title="Stored scan data"
        description="What a scan loaded is kept on this Mac so that you can read its scripts and its requests afterwards."
      >
        <StoredDataFields settings={settings} save={save} />
      </Section>

      <Section
        title="Data"
        description="Everything is stored locally. Nothing is uploaded except the model requests you enable."
      >
        <Field
          label="What is stored"
          description="Reports keep findings with short excerpts, derived auctions, slots, metrics, bidder check results, script metadata and the audit trail. With “Keep network data” on, each scan also keeps the requests it made and the text it received, compressed, in a file of its own that is deleted with the scan; passwords, tokens and other values with a credential name in addresses, request bodies and JSON responses are replaced before it is written, and so are cookie and credential headers unless “Store full headers” is on. ads.txt and sellers.json files are read in memory and not kept. API keys and proxy passwords are in the Keychain, never in these files."
        >
          <DeleteAllScansButton />
        </Field>
      </Section>

      <Section title="About">
        <Field
          label="Ad-Tech Scanner"
          description="Finds hidden revenue leaks caused by logic bugs in ad-tech wrappers. Outbound traffic goes only to the site being scanned (the page you enter, the pages a browse visit follows on that site and its ads.txt), api.anthropic.com, api.typesafe.ai when Jev is enabled and, when the sellers.json check is on, the ad exchanges that ads.txt names. A scan's traffic to the site and to those exchanges goes through the proxy you chose for it, if any. Scan data never leaves this Mac unless you export a report or copy something out of it yourself."
        >
          <span className="selectable font-mono text-xs text-muted-foreground">v{__APP_VERSION__}</span>
        </Field>
      </Section>
    </>
  );
}

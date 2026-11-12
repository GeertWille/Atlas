import { useId } from "react";
import { toast } from "sonner";
import type { Settings, SettingsPatch } from "@/bindings";
import { useArchiveStorage, useClearArchive } from "@/lib/query/hooks";
import { formatBytes, formatDateTime, pluralize } from "@/shared/lib/format";
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
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { Switch } from "@/shared/ui/switch";

const GIB = 1024 ** 3;
const LIMITS_GIB = [1, 2, 5, 10, 20, 50];

/** The limit as it stands in the menu. A limit set elsewhere to another value still gets its own entry. */
function limitOptions(limitBytes: number): number[] {
  return LIMITS_GIB.some((limit) => limit * GIB === limitBytes)
    ? LIMITS_GIB.map((limit) => limit * GIB)
    : [...LIMITS_GIB.map((limit) => limit * GIB), limitBytes].sort((a, b) => a - b);
}

const limitLabel = (bytes: number) => (bytes % GIB === 0 ? `${bytes / GIB} GB` : formatBytes(bytes));

function Usage({ limitBytes }: { limitBytes: number }) {
  const storage = useArchiveStorage();
  const clear = useClearArchive();
  if (!storage.data) return null;
  const { scans, evictedScans, storedBytes, oldestAt } = storage.data;
  const share = limitBytes > 0 ? Math.min(1, storedBytes / limitBytes) : 0;
  return (
    <div className="flex w-full flex-col gap-2.5">
      {/* biome-ignore lint/a11y/useSemanticElements: a native meter cannot be styled alike across WebKit and Chromium */}
      <div
        role="meter"
        aria-label="Storage used by scan data"
        aria-valuemin={0}
        aria-valuemax={limitBytes}
        aria-valuenow={storedBytes}
        aria-valuetext={`${formatBytes(storedBytes)} of ${limitLabel(limitBytes)}`}
        className="h-1.5 overflow-hidden rounded-full bg-muted"
      >
        <div
          className="h-full rounded-full bg-primary"
          style={{ width: `${Math.max(share * 100, storedBytes > 0 ? 1 : 0)}%` }}
        />
      </div>
      <div className="flex items-center justify-between gap-4">
        <p className="text-[13px] text-muted-foreground" aria-live="polite">
          <span className="font-medium text-foreground tabular-nums">{formatBytes(storedBytes)}</span> of{" "}
          {limitLabel(limitBytes)} in {pluralize(scans, "scan")}
          {oldestAt === null ? "" : `, the oldest from ${formatDateTime(oldestAt)}`}.
          {evictedScans > 0
            ? ` ${pluralize(evictedScans, "scan")} no longer ${evictedScans === 1 ? "has its" : "have their"} network data: it was cleared here, or removed to stay under the limit.`
            : ""}
        </p>
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button variant="destructive-outline" size="sm" disabled={scans === 0 || clear.isPending}>
              Clear stored scan data…
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Clear stored scan data?</AlertDialogTitle>
              <AlertDialogDescription>
                The requests, headers, scripts and bid payloads of every finished scan are deleted from this Mac. The
                reports stay, with their findings and audit trails; their script viewer and network view will have
                nothing to show. This cannot be undone.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction
                destructive
                onClick={() =>
                  clear.mutate(undefined, {
                    onSuccess: () => toast.success("Stored scan data cleared"),
                    onError: (reason) => notifyError("Could not clear the stored scan data", reason),
                  })
                }
              >
                Clear scan data
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </div>
  );
}

/** What a scan keeps of the pages it loaded, how much room that may take, and the way to remove it. */
export function StoredDataFields({ settings, save }: { settings: Settings; save: (patch: SettingsPatch) => void }) {
  const ids = { keep: useId(), keepNote: useId(), headers: useId(), headersNote: useId() };
  return (
    <>
      <Field
        label="Keep network data with reports"
        htmlFor={ids.keep}
        description={
          <span id={ids.keepNote}>
            Stores what each scan loaded on this Mac, next to its report: every request with its address and headers,
            the scripts, pages, JSON and other text it received, and what was posted, bid requests and responses
            included. The script viewer and the network view show this data. Images, video, audio and fonts are listed
            but their content is not stored. If you scan a page while signed in, or browse it yourself, the stored pages
            and requests can contain details of that account. Off: new scans keep their findings only.
          </span>
        }
      >
        <Switch
          id={ids.keep}
          aria-describedby={ids.keepNote}
          checked={settings.keepNetworkData}
          onCheckedChange={(keepNetworkData) => save({ keepNetworkData })}
        />
      </Field>
      <Field
        label="Store full headers"
        htmlFor={ids.headers}
        description={
          <span id={ids.headersNote}>
            Off, the values of the Cookie, Set-Cookie and Authorization headers, and of every other header with a
            credential name (X-Api-Key, X-Auth-Token, X-CSRF-Token and the like), are replaced by their length before
            anything is written; cookie names are kept. On, new scans keep those headers as sent. Replaced whatever this
            switch says: the Proxy-Authorization header, parameters with a credential name in every stored address
            (access_token, password and the like), and fields with such a name in request bodies and JSON responses.
            Everything else is kept exactly as captured and can hold ids and tokens too: the rest of every address,
            consent strings such as gdpr_consent, all other headers, bid requests and responses, and the pages
            themselves.
          </span>
        }
      >
        <Switch
          id={ids.headers}
          aria-describedby={ids.headersNote}
          checked={settings.storeFullHeaders}
          disabled={!settings.keepNetworkData}
          onCheckedChange={(storeFullHeaders) => save({ storeFullHeaders })}
        />
      </Field>
      <Field
        label="Storage limit"
        description="When the stored scan data reaches the limit, the oldest scans lose their network data; their reports stay. Lowering the limit removes data at once."
      >
        <Select
          value={String(settings.archiveLimitBytes)}
          onValueChange={(value) => save({ archiveLimitBytes: Number(value) })}
        >
          <SelectTrigger aria-label="Storage limit" className="w-28">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {limitOptions(settings.archiveLimitBytes).map((bytes) => (
              <SelectItem key={bytes} value={String(bytes)}>
                {limitLabel(bytes)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>
      <Field
        stacked
        label="Stored scan data"
        description="Deleting a scan deletes its data with it. Nothing here is encrypted by the app: it is as protected as the rest of your disk."
      >
        <Usage limitBytes={settings.archiveLimitBytes} />
      </Field>
    </>
  );
}

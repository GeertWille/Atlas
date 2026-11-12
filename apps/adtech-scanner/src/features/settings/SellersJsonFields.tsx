import { useId } from "react";
import { toast } from "sonner";
import type { Settings, SettingsPatch } from "@/bindings";
import { useClearSellersJsonCache, useSellersJsonCache } from "@/lib/query/hooks";
import { formatBytes, pluralize } from "@/shared/lib/format";
import { notifyError } from "@/shared/lib/notify-error";
import { Button } from "@/shared/ui/button";
import { Field } from "@/shared/ui/field";
import { Switch } from "@/shared/ui/switch";

function KeptFiles() {
  const cache = useSellersJsonCache();
  const clear = useClearSellersJsonCache();
  if (!cache.data) return null;
  const { entries, storedBytes } = cache.data;
  return (
    <div className="flex items-center gap-3">
      <p className="text-[13px] whitespace-nowrap text-muted-foreground" aria-live="polite">
        {entries === 0 ? (
          "None kept"
        ) : (
          <>
            <span className="font-medium text-foreground tabular-nums">{formatBytes(storedBytes)}</span> from{" "}
            {pluralize(entries, "exchange")}
          </>
        )}
      </p>
      <Button
        variant="outline"
        size="sm"
        disabled={entries === 0 || clear.isPending}
        onClick={() =>
          clear.mutate(undefined, {
            onSuccess: () => toast.success("Kept sellers.json files cleared"),
            onError: (reason) => notifyError("Could not clear the kept sellers.json files", reason),
          })
        }
      >
        Clear kept files
      </Button>
    </div>
  );
}

/** The one bidder check that contacts hosts other than the scanned site, and the files it keeps between scans. */
export function SellersJsonFields({ settings, save }: { settings: Settings; save: (patch: SettingsPatch) => void }) {
  const ids = { check: useId(), disclosure: useId() };
  return (
    <>
      <Field
        label="Check sellers.json at ad exchanges"
        htmlFor={ids.check}
        description={
          <span id={ids.disclosure}>
            On: a scan asks the ad exchanges that the scanned site's ads.txt names (appnexus.com, rubiconproject.com and
            the like, at most 12 per scan) for their sellers.json, to verify the site's seller ids. These requests go
            straight to the exchanges, through the system's proxy if there is one and never through the proxy chosen for
            a scan. A page on this Mac, such as the demo site, is never asked about. Turn it off on a metered or slow
            connection (a file can be several megabytes), or to contact only the scanned site.
          </span>
        }
      >
        <Switch
          id={ids.check}
          aria-describedby={ids.disclosure}
          checked={settings.checkSellersJson}
          onCheckedChange={(checkSellersJson) => save({ checkSellersJson })}
        />
      </Field>
      <Field
        label="Kept sellers.json files"
        description="An exchange's file is the same for every site, so it is kept on this Mac and reused by all scans: for 24 hours without asking, after that the exchange is asked whether it changed. If an exchange cannot be reached, a copy of up to 7 days is used and the check says so. At most 500 MB are kept; clearing them only makes the next scan download them again."
      >
        <KeptFiles />
      </Field>
    </>
  );
}

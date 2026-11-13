import { KeyRoundIcon, PencilIcon, PlusIcon, Trash2Icon, UserIcon } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import type { ProxyProfile, Settings, SettingsPatch } from "@/bindings";
import { useDeleteProxyProfile } from "@/lib/query/hooks";
import { proxyAddress, proxyChoiceFromKey, proxyChoiceKey, type SavedProxyKey } from "@/shared/domain/proxy";
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
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Field } from "@/shared/ui/field";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { Hint } from "@/shared/ui/tooltip";
import { ProxyDialog } from "./ProxyDialog";

type Editing = { kind: "add" } | { kind: "edit"; proxy: ProxyProfile } | null;

function ProxyRow({ proxy, onEdit, onDelete }: { proxy: ProxyProfile; onEdit: () => void; onDelete: () => void }) {
  return (
    <li className="flex items-center gap-3 border-b px-3.5 py-2.5 last:border-b-0">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-[13px] font-medium">{proxy.name}</span>
        <span className="selectable truncate font-mono text-xs text-muted-foreground">{proxyAddress(proxy)}</span>
      </div>
      {proxy.username ? (
        <Badge tone="outline" className="hidden xl:inline-flex" title={`Signs in as ${proxy.username}`}>
          <UserIcon />
          {proxy.username}
        </Badge>
      ) : null}
      {proxy.hasPassword ? (
        <Badge tone="green">
          <KeyRoundIcon />
          Password stored
        </Badge>
      ) : null}
      <Hint label="Edit">
        <Button variant="ghost" size="icon-sm" aria-label={`Edit ${proxy.name}`} onClick={onEdit}>
          <PencilIcon />
        </Button>
      </Hint>
      <Hint label="Delete">
        <Button variant="ghost" size="icon-sm" aria-label={`Delete ${proxy.name}`} onClick={onDelete}>
          <Trash2Icon />
        </Button>
      </Hint>
    </li>
  );
}

/** The saved proxies and which of them, if any, a scan uses unless its own options say otherwise. */
export function ProxySection({ settings, save }: { settings: Settings; save: (patch: SettingsPatch) => void }) {
  const deleteProxy = useDeleteProxyProfile();
  const [editing, setEditing] = useState<Editing>(null);
  const [deleting, setDeleting] = useState<ProxyProfile | null>(null);

  return (
    <>
      <Field
        label="Scans use"
        description="For scans that run on this Mac. A scan can choose differently under Options. Pages on this computer, like the demo site, always connect directly."
      >
        <Select
          value={proxyChoiceKey(settings.defaultProxy)}
          onValueChange={(key) => save({ defaultProxy: proxyChoiceFromKey(key as SavedProxyKey) })}
        >
          <SelectTrigger aria-label="Default proxy" className="w-44">
            <SelectValue />
          </SelectTrigger>
          <SelectContent align="end">
            <SelectGroup>
              <SelectItem value="none">No proxy</SelectItem>
              <SelectItem value="system">System proxy</SelectItem>
              {settings.proxies.map((proxy) => (
                <SelectItem key={proxy.id} value={proxyChoiceKey({ kind: "profile", id: proxy.id })}>
                  {proxy.name}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
      </Field>
      <Field
        stacked
        label="Saved proxies"
        description="Name a proxy after where it is, then pick it for a scan or a saved configuration. Each scan goes through one proxy; there is no rotation."
      >
        <div className="flex w-full flex-col gap-2.5">
          {settings.proxies.length > 0 ? (
            <ul aria-label="Saved proxies" className="rounded-lg border">
              {settings.proxies.map((proxy) => (
                <ProxyRow
                  key={proxy.id}
                  proxy={proxy}
                  onEdit={() => setEditing({ kind: "edit", proxy })}
                  onDelete={() => setDeleting(proxy)}
                />
              ))}
            </ul>
          ) : null}
          <div>
            <Button variant="outline" size="sm" onClick={() => setEditing({ kind: "add" })}>
              <PlusIcon />
              Add proxy…
            </Button>
          </div>
          <p className="text-[13px] text-pretty text-muted-foreground">
            The scanned pages, and the app's own request for the site's ads.txt, go through the proxy of the scan.
            Behind a proxy the app cannot fully verify where a request lands: the proxy looks up addresses itself.
            Requests to Anthropic and TypeSafe, and those for the sellers.json of ad exchanges, never use it.
          </p>
        </div>
      </Field>

      {editing ? (
        <ProxyDialog
          proxy={editing.kind === "edit" ? editing.proxy : undefined}
          settings={settings}
          onClose={() => setEditing(null)}
        />
      ) : null}
      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete the proxy “{deleting?.name}”?</AlertDialogTitle>
            <AlertDialogDescription>
              Its password is removed from your keychain. Scans already run through it keep its name in their report.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              destructive
              onClick={() => {
                if (!deleting) return;
                deleteProxy.mutate(deleting.id, {
                  onSuccess: () => toast.success(`Deleted “${deleting.name}”`),
                  onError: (reason) => notifyError("Could not delete the proxy", reason),
                });
              }}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

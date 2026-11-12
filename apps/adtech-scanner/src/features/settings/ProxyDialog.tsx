import { type ComponentProps, type FormEvent, type ReactNode, useId, useState } from "react";
import { toast } from "sonner";
import type { ProxyProfile, ProxyScheme, Settings } from "@/bindings";
import { toApiError } from "@/lib/api";
import { useClearProxyPassword, useSaveProxyProfile, useSetProxyPassword } from "@/lib/query/hooks";
import { cn } from "@/shared/lib/cn";
import { notifyError } from "@/shared/lib/notify-error";
import { Button } from "@/shared/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Label } from "@/shared/ui/label";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import {
  newProxyForm,
  PROXY_NAME_MAX,
  PROXY_SCHEME_LABEL,
  PROXY_SCHEMES,
  type ProxyForm,
  type ProxyFormErrors,
  proxyFormOf,
  validateProxyForm,
  withScheme,
} from "./proxy-form";

function TextField({
  label,
  error,
  hint,
  ...input
}: Omit<ComponentProps<typeof Input>, "id"> & { label: string; error?: string; hint?: ReactNode }) {
  const id = useId();
  const helpId = useId();
  const help = error ?? hint;
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        autoComplete="off"
        spellCheck={false}
        aria-invalid={error ? true : undefined}
        aria-describedby={help ? helpId : undefined}
        {...input}
      />
      {help ? (
        <p
          id={helpId}
          role={error ? "alert" : undefined}
          className={cn("text-xs text-pretty", error ? "text-red-fg" : "text-muted-foreground")}
        >
          {help}
        </p>
      ) : null}
    </div>
  );
}

/** Adds a proxy, or edits `proxy`. Mounted while it is open. */
export function ProxyDialog({
  proxy,
  settings,
  onClose,
}: {
  proxy?: ProxyProfile;
  settings: Settings;
  onClose: () => void;
}) {
  const saveProfile = useSaveProxyProfile();
  const setPassword = useSetProxyPassword();
  const clearPassword = useClearProxyPassword();
  const schemeId = useId();
  const [form, setForm] = useState<ProxyForm>(() => (proxy ? proxyFormOf(proxy) : newProxyForm()));
  const [errors, setErrors] = useState<ProxyFormErrors>({});
  const [failure, setFailure] = useState<string | null>(null);
  // Follows the settings, so removing the stored password shows at once.
  const hasPassword = settings.proxies.find((candidate) => candidate.id === proxy?.id)?.hasPassword ?? false;
  const busy = saveProfile.isPending || setPassword.isPending;

  const update = (patch: Partial<ProxyForm>) => {
    setForm((current) => ({ ...current, ...patch }));
    setErrors((current) => Object.fromEntries(Object.entries(current).filter(([field]) => !(field in patch))));
    setFailure(null);
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const others = settings.proxies.filter((other) => other.id !== proxy?.id).map((other) => other.name);
    const result = validateProxyForm(form, others, proxy?.id);
    if (!result.ok) {
      setErrors(result.errors);
      return;
    }
    try {
      const saved = await saveProfile.mutateAsync(result.draft);
      if (result.password !== null) {
        // A new profile gets its id from the backend; its name is unique, so it finds it.
        const id =
          proxy?.id ??
          saved.proxies.find((candidate) => candidate.name.toLowerCase() === result.draft.name.toLowerCase())?.id;
        if (id) await setPassword.mutateAsync({ id, password: result.password });
      }
      toast.success(proxy ? `Saved “${result.draft.name}”` : `Added “${result.draft.name}”`);
      onClose();
    } catch (reason) {
      setFailure(toApiError(reason).message);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-lg">
        <DialogTitle>{proxy ? "Edit proxy" : "Add a proxy"}</DialogTitle>
        <DialogDescription>
          A scan through this proxy reaches the site from where the proxy is, for example from another country.
        </DialogDescription>
        <form onSubmit={submit} noValidate className="flex flex-col gap-4">
          <TextField
            label="Name"
            placeholder="Belgium"
            maxLength={PROXY_NAME_MAX}
            value={form.name}
            error={errors.name}
            onChange={(event) => update({ name: event.target.value })}
          />
          <div className="grid grid-cols-[112px_minmax(0,1fr)_96px] items-start gap-3">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={schemeId}>Type</Label>
              <Select
                value={form.scheme}
                onValueChange={(scheme) => {
                  setForm((current) => withScheme(current, scheme as ProxyScheme));
                  setErrors((current) => ({ ...current, username: undefined }));
                }}
              >
                <SelectTrigger id={schemeId} className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {PROXY_SCHEMES.map((scheme) => (
                      <SelectItem key={scheme} value={scheme}>
                        {PROXY_SCHEME_LABEL[scheme]}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
            </div>
            <TextField
              label="Host"
              placeholder="be.proxy.example"
              className="font-mono"
              value={form.host}
              error={errors.host}
              onChange={(event) => update({ host: event.target.value })}
            />
            <TextField
              label="Port"
              inputMode="numeric"
              className="tabular-nums"
              value={form.port}
              error={errors.port}
              onChange={(event) => update({ port: event.target.value })}
            />
          </div>
          <div className="grid grid-cols-2 items-start gap-3">
            <TextField
              label="User name (optional)"
              value={form.username}
              error={errors.username}
              hint={form.scheme === "socks5" ? "Not available for SOCKS5: Chrome cannot sign in there." : undefined}
              onChange={(event) => update({ username: event.target.value })}
            />
            <TextField
              label="Password (optional)"
              type="password"
              autoComplete="new-password"
              placeholder={hasPassword ? "Stored. Type to replace it" : undefined}
              value={form.password}
              error={errors.password}
              hint="Kept in your system keychain, never in the app database or logs."
              onChange={(event) => update({ password: event.target.value })}
            />
          </div>
          {proxy && hasPassword ? (
            <div className="flex items-center justify-between gap-4 rounded-lg border border-green-line bg-green-soft/60 px-3.5 py-2">
              <p className="text-[13px] text-green-fg">A password for this proxy is stored in your keychain.</p>
              <Button
                variant="outline"
                size="sm"
                disabled={clearPassword.isPending}
                onClick={() =>
                  clearPassword.mutate(proxy.id, {
                    onSuccess: () => toast.success("Proxy password removed"),
                    onError: (reason) => notifyError("Could not remove the password", reason),
                  })
                }
              >
                Remove password
              </Button>
            </div>
          ) : null}
          {failure ? (
            <p role="alert" className="text-[13px] text-red-fg">
              {failure}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {proxy ? "Save proxy" : "Add proxy"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

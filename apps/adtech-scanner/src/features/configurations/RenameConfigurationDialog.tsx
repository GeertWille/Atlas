import { type FormEvent, useId, useState } from "react";
import { toast } from "sonner";
import type { ScanConfiguration } from "@/bindings";
import { toApiError } from "@/lib/api";
import { useSaveScanConfiguration } from "@/lib/query/hooks";
import { CONFIGURATION_NAME_MAX } from "@/shared/domain/scan-options";
import { Button } from "@/shared/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Label } from "@/shared/ui/label";

/** Mounted while it is open, so every opening starts from the current name. */
export function RenameConfigurationDialog({
  configuration,
  onClose,
}: {
  configuration: ScanConfiguration;
  onClose: () => void;
}) {
  const save = useSaveScanConfiguration();
  const nameId = useId();
  const errorId = useId();
  const [name, setName] = useState(configuration.name);
  const [error, setError] = useState<string | null>(null);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const trimmed = name.trim();
    if (trimmed === "") {
      setError("Give the configuration a name.");
      return;
    }
    if (trimmed === configuration.name) {
      onClose();
      return;
    }
    save.mutate(
      { id: configuration.id, name: trimmed, options: configuration.options },
      {
        onSuccess: (renamed) => {
          toast.success(`Renamed to “${renamed.name}”`);
          onClose();
        },
        onError: (reason) => setError(toApiError(reason).message),
      },
    );
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogTitle>Rename configuration</DialogTitle>
        <DialogDescription>Scans already run keep the name they were started with.</DialogDescription>
        <form onSubmit={submit} noValidate className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={nameId}>Name</Label>
            <Input
              id={nameId}
              autoComplete="off"
              maxLength={CONFIGURATION_NAME_MAX}
              value={name}
              aria-invalid={error ? true : undefined}
              aria-describedby={error ? errorId : undefined}
              onChange={(event) => {
                setName(event.target.value);
                setError(null);
              }}
            />
            {error ? (
              <p id={errorId} role="alert" className="text-[13px] text-red-fg">
                {error}
              </p>
            ) : null}
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={save.isPending}>
              Rename
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

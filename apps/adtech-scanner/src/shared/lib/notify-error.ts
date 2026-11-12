import { toast } from "sonner";
import { toApiError } from "@/lib/api";

/** Surfaces a failed command as a toast with the backend's own message. */
export function notifyError(title: string, reason: unknown) {
  toast.error(title, { description: toApiError(reason).message });
}

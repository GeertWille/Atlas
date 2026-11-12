import { api } from "@/lib/api";
import { notifyError } from "./notify-error";

const API_KEYS_URL = "https://console.anthropic.com/settings/keys";

/** Opens the Anthropic Console page for creating API keys in the default browser. */
export function openApiKeysPage() {
  api.openExternal(API_KEYS_URL).catch((reason) => notifyError("Could not open the link", reason));
}

/** Native file picker for a Chrome/Chromium executable. `null` when cancelled or when the picker failed (reported). */
export async function chooseBrowserExecutable(): Promise<string | null> {
  try {
    return await api.pickBrowserExecutable();
  } catch (reason) {
    notifyError("Could not open the file picker", reason);
    return null;
  }
}

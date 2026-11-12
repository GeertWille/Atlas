import type { ApiProvider } from "@/bindings";
import { api } from "@/lib/api";
import { notifyError } from "./notify-error";

const API_KEYS_URL: Record<ApiProvider, string> = {
  anthropic: "https://console.anthropic.com/settings/keys",
  typesafe: "https://console.typesafe.ai/keys",
};

/** Opens the provider's console page for creating API keys in the default browser. */
export function openApiKeysPage(provider: ApiProvider) {
  api.openExternal(API_KEYS_URL[provider]).catch((reason) => notifyError("Could not open the link", reason));
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

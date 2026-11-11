import type { ApiPort } from "./port";

declare const __FAKE_BACKEND__: boolean;

async function loadAdapter(): Promise<ApiPort> {
  if ("__TAURI_INTERNALS__" in window) {
    const { createTauriAdapter } = await import("./tauri-adapter");
    return createTauriAdapter();
  }
  // `__FAKE_BACKEND__` is a build-time constant (false under `tauri build`), so
  // the fake backend chunk is not even emitted for the packaged app.
  if (__FAKE_BACKEND__) {
    const { createFakeBackend } = await import("@/lib/fake-backend");
    return createFakeBackend();
  }
  throw new Error("Ad-Tech Scanner must run inside its desktop shell.");
}

export const api: ApiPort = await loadAdapter();

export { ApiError, toApiError } from "./error";
export type { ApiPort } from "./port";

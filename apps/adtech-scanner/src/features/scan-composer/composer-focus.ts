export const SCAN_URL_INPUT_ID = "scan-url";

let focusPending = false;

/** Focuses the composer's URL field now, or as soon as the composer mounts. */
export function requestComposerFocus() {
  const input = document.getElementById(SCAN_URL_INPUT_ID);
  if (input) input.focus();
  else focusPending = true;
}

export function takePendingComposerFocus(): boolean {
  const pending = focusPending;
  focusPending = false;
  return pending;
}

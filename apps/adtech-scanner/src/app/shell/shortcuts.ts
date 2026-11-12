import { useEffect } from "react";

export const IS_MAC = /Mac|iPhone|iPad/.test(navigator.platform);
export const MOD_KEY = IS_MAC ? "⌘" : "Ctrl+";

export type ShortcutHandlers = {
  onCommandPalette: () => void;
  onNewScan: () => void;
  onSettings: () => void;
};

/** ⌘K, ⌘N and ⌘, (Ctrl on other platforms). Escape is handled by the focused overlay. */
export function useGlobalShortcuts({ onCommandPalette, onNewScan, onSettings }: ShortcutHandlers) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // Ctrl+N and Ctrl+K are text-editing keys on macOS; only the platform's own modifier counts.
      const modifier = IS_MAC ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
      if (!modifier || event.altKey || event.shiftKey) return;
      const handler = { k: onCommandPalette, n: onNewScan, ",": onSettings }[event.key.toLowerCase()];
      if (!handler) return;
      event.preventDefault();
      handler();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onCommandPalette, onNewScan, onSettings]);
}

import { useEffect } from "react";
import { hasModifier } from "@/shared/lib/platform";

export type ShortcutHandlers = {
  onCommandPalette: () => void;
  onNewScan: () => void;
  onSettings: () => void;
};

/** ⌘K, ⌘N and ⌘, (Ctrl on other platforms). Escape is handled by the focused overlay. */
export function useGlobalShortcuts({ onCommandPalette, onNewScan, onSettings }: ShortcutHandlers) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!hasModifier(event) || event.altKey || event.shiftKey) return;
      const handler = { k: onCommandPalette, n: onNewScan, ",": onSettings }[event.key.toLowerCase()];
      if (!handler) return;
      event.preventDefault();
      handler();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onCommandPalette, onNewScan, onSettings]);
}

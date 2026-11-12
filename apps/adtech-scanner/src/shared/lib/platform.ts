export const IS_MAC = /Mac|iPhone|iPad/.test(navigator.platform);

/** How the platform's command modifier is written before a key: "⌘K", "Ctrl+K". */
export const MOD_KEY = IS_MAC ? "⌘" : "Ctrl+";

/**
 * The platform's own command modifier and nothing else: ⌘ on macOS, Ctrl elsewhere. Ctrl+N and Ctrl+K
 * are text-editing keys on macOS, so the other modifier never counts.
 */
export function hasModifier(event: Pick<KeyboardEvent, "metaKey" | "ctrlKey">): boolean {
  return IS_MAC ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}

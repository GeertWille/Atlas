/**
 * Characters that change how text reads without being seen: bidirectional controls, which can make a
 * line display in another order than it is written, and zero-width and other format characters.
 */
const INVISIBLE = /[\u00ad\u061c\u180e\u200b-\u200f\u2028-\u202e\u2060-\u2064\u2066-\u2069\ufeff]/g;

/**
 * Text from a scanned page or a model, safe to show outside the code editor: every invisible character
 * is written out as its code point (`⟨U+202E⟩`), so what is displayed is what is there.
 */
export function revealInvisibles(text: string): string {
  return text.replace(INVISIBLE, (char) => `⟨U+${char.charCodeAt(0).toString(16).toUpperCase().padStart(4, "0")}⟩`);
}

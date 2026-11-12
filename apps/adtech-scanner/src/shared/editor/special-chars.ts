import { highlightSpecialChars } from "@codemirror/view";

/**
 * Characters that change how a line reads without being visible: bidirectional controls that can make
 * code display in a different order than it runs, and zero-width characters. CodeMirror already
 * replaces most control characters; these complete the set.
 */
const HIDDEN_CHARS = /\u200c|\u200d|\u2060|\u2068|[\u202a-\u202c]/g;

const NAMES: Record<number, string> = {
  8203: "zero width space",
  8204: "zero width non-joiner",
  8205: "zero width joiner",
  8206: "left-to-right mark",
  8207: "right-to-left mark",
  8288: "word joiner",
  8234: "left-to-right embedding",
  8235: "right-to-left embedding",
  8236: "pop directional formatting",
  8237: "left-to-right override",
  8238: "right-to-left override",
  8294: "left-to-right isolate",
  8295: "right-to-left isolate",
  8296: "first strong isolate",
  8297: "pop directional isolate",
  65279: "zero width no-break space",
};

function codePoint(code: number): string {
  return `U+${code.toString(16).toUpperCase().padStart(4, "0")}`;
}

/** Every invisible or control character becomes a visible, labelled placeholder: the text can never mislead. */
export const specialChars = highlightSpecialChars({
  addSpecialChars: HIDDEN_CHARS,
  render(code, description) {
    const name = NAMES[code] ?? description ?? "control character";
    const span = document.createElement("span");
    span.className = "cm-hiddenChar";
    span.textContent = codePoint(code);
    span.title = `${codePoint(code)} ${name}`;
    span.setAttribute("aria-label", name);
    return span;
  },
});

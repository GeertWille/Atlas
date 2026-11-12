import { EditorView } from "@codemirror/view";

const FLASH_KEYFRAMES = {
  from: { backgroundColor: "color-mix(in oklab, var(--primary) 34%, transparent)" },
  to: { backgroundColor: "transparent" },
};

/**
 * Every colour is a CSS variable of the design system, so the editor follows the light and dark
 * themes without being reconfigured. Token colours are the `.tok-*` rules in `styles/editor.css`.
 */
export const editorTheme = EditorView.theme({
  "&": {
    height: "100%",
    color: "var(--foreground)",
    backgroundColor: "var(--code)",
    fontSize: "12.5px",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": {
    fontFamily: "var(--font-mono)",
    lineHeight: "1.55",
    fontVariantLigatures: "none",
    overflow: "auto",
  },
  ".cm-content": {
    padding: "6px 0",
    caretColor: "var(--foreground)",
    // Untrusted text never decides the reading direction of the view.
    direction: "ltr",
    userSelect: "text",
    WebkitUserSelect: "text",
    cursor: "text",
  },
  ".cm-line": { padding: "0 12px 0 6px" },
  ".cm-gutters": {
    backgroundColor: "var(--code)",
    color: "var(--subtle-foreground)",
    border: "none",
    borderRight: "1px solid var(--border)",
    userSelect: "none",
    WebkitUserSelect: "none",
  },
  ".cm-lineNumbers .cm-gutterElement": {
    minWidth: "44px",
    padding: "0 8px 0 10px",
    fontVariantNumeric: "tabular-nums",
  },
  ".cm-foldGutter .cm-gutterElement": { width: "14px", padding: "0", textAlign: "center", cursor: "default" },
  ".cm-foldGutter .cm-gutterElement:hover": { color: "var(--foreground)" },
  ".cm-activeLine": { backgroundColor: "color-mix(in oklab, var(--foreground) 4%, transparent)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--foreground)" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--foreground)", borderLeftWidth: "1.5px" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground": {
    backgroundColor: "color-mix(in oklab, var(--primary) 22%, transparent)",
  },
  ".cm-selectionMatch": { backgroundColor: "color-mix(in oklab, var(--primary) 12%, transparent)" },
  "&.cm-focused .cm-matchingBracket": {
    backgroundColor: "transparent",
    outline: "1px solid var(--border-strong)",
    borderRadius: "2px",
  },
  "&.cm-focused .cm-nonmatchingBracket": { backgroundColor: "transparent" },
  ".cm-searchMatch": {
    backgroundColor: "color-mix(in oklab, var(--tone-amber) 32%, transparent)",
    borderRadius: "2px",
  },
  ".cm-searchMatch.cm-searchMatch-selected": {
    backgroundColor: "color-mix(in oklab, var(--tone-amber) 62%, transparent)",
    outline: "1px solid var(--tone-amber-fg)",
  },
  ".cm-foldPlaceholder": {
    margin: "0 4px",
    padding: "0 6px",
    border: "1px solid var(--border-strong)",
    borderRadius: "4px",
    backgroundColor: "var(--muted)",
    color: "var(--muted-foreground)",
  },
  ".cm-hiddenChar": {
    margin: "0 1px",
    padding: "0 3px",
    border: "1px solid var(--tone-red-line)",
    borderRadius: "3px",
    backgroundColor: "var(--tone-red-soft)",
    color: "var(--tone-red-fg)",
    fontSize: "10px",
    lineHeight: "1",
    verticalAlign: "1px",
  },
  ".cm-panels": {
    backgroundColor: "var(--background)",
    color: "var(--foreground)",
    fontFamily: "var(--font-sans)",
  },
  ".cm-panels.cm-panels-top": { borderBottom: "1px solid var(--border)" },
  ".cm-panels.cm-panels-bottom": { borderTop: "1px solid var(--border)" },
  ".cm-flash-a": { animation: "cm-flash-a 600ms ease-out" },
  ".cm-flash-b": { animation: "cm-flash-b 600ms ease-out" },
  "@keyframes cm-flash-a": FLASH_KEYFRAMES,
  "@keyframes cm-flash-b": FLASH_KEYFRAMES,
});

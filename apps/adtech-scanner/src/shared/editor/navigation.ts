import { EditorSelection, type Range, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView } from "@codemirror/view";

const FLASH_MS = 600;

type Flash = { from: number; to: number; beat: 0 | 1 };

const setFlash = StateEffect.define<Flash | null>();
/**
 * Two classes with the same animation under two names: a second jump to lines that are still
 * flashing swaps the name, which is what makes the browser start the animation again.
 */
const FLASH_LINE = [Decoration.line({ class: "cm-flash-a" }), Decoration.line({ class: "cm-flash-b" })] as const;
let jumps = 0;

/** The lines that are flashing after a jump. */
export const flashField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, transaction) {
    let next = transaction.docChanged ? Decoration.none : value;
    for (const effect of transaction.effects) {
      if (!effect.is(setFlash)) continue;
      const range = effect.value;
      if (!range) {
        next = Decoration.none;
        continue;
      }
      const marks: Range<Decoration>[] = [];
      for (let line = range.from; line <= range.to; line += 1) {
        marks.push(FLASH_LINE[range.beat].range(transaction.state.doc.line(line).from));
      }
      next = Decoration.set(marks);
    }
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

export function clampLine(view: EditorView, line: number): number {
  return Math.min(Math.max(1, Math.round(line)), view.state.doc.lines);
}

/**
 * Puts the caret on `line`, centres it and flashes the range up to `endLine`. The flash is a CSS
 * animation, which the app's reduced-motion rule turns off.
 */
export function jumpToLine(view: EditorView, line: number, endLine: number = line, column = 1) {
  const from = clampLine(view, line);
  const to = Math.max(from, clampLine(view, endLine));
  const target = view.state.doc.line(from);
  const position = Math.min(target.from + Math.max(0, column - 1), target.to);
  jumps += 1;
  const jump = jumps;
  const beat = jump % 2 === 0 ? 0 : 1;
  view.dispatch({
    selection: EditorSelection.cursor(position),
    effects: [EditorView.scrollIntoView(position, { y: "center" }), setFlash.of({ from, to, beat })],
  });
  window.setTimeout(() => {
    // The view may be gone by the time the animation ends, and a later jump owns the decoration then.
    if (view.dom.isConnected && jumps === jump) view.dispatch({ effects: setFlash.of(null) });
  }, FLASH_MS + 100);
}

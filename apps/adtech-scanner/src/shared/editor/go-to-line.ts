import { type Extension, StateEffect, StateField } from "@codemirror/state";
import { type EditorView, type Panel, showPanel } from "@codemirror/view";
import { jumpToLine } from "./navigation";

const toggleGoToLine = StateEffect.define<boolean>();

function createGoToLinePanel(view: EditorView): Panel {
  const dom = document.createElement("form");
  dom.className = "flex items-center gap-2 px-2 py-1.5";
  dom.setAttribute("aria-label", "Go to line");

  const label = document.createElement("label");
  label.className = "flex items-center gap-2 text-xs text-muted-foreground";
  label.textContent = "Go to line";

  const input = document.createElement("input");
  input.type = "text";
  input.inputMode = "numeric";
  input.className =
    "h-6 w-24 rounded-md border border-input bg-background px-2 font-mono text-xs text-foreground outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40";
  input.setAttribute("main-field", "true");
  label.append(input);

  const hint = document.createElement("span");
  hint.className = "text-xs text-subtle-foreground tabular-nums";
  hint.textContent = `1 to ${view.state.doc.lines.toLocaleString("en-US")}`;

  const close = () => {
    view.dispatch({ effects: toggleGoToLine.of(false) });
    view.focus();
  };

  dom.addEventListener("submit", (event) => {
    event.preventDefault();
    // "118" or "118:5" (line and column), as editors accept.
    const match = /^\s*(\d+)(?::(\d+))?\s*$/.exec(input.value);
    if (!match) return;
    close();
    jumpToLine(view, Number(match[1]), Number(match[1]), match[2] ? Number(match[2]) : 1);
  });
  dom.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    close();
  });

  dom.append(label, hint);
  return { dom, top: true, mount: () => input.focus() };
}

const goToLineField = StateField.define<boolean>({
  create: () => false,
  update(open, transaction) {
    for (const effect of transaction.effects) if (effect.is(toggleGoToLine)) return effect.value;
    return open;
  },
  provide: (field) => showPanel.from(field, (open) => (open ? createGoToLinePanel : null)),
});

export function openGoToLine(view: EditorView): boolean {
  view.dispatch({ effects: toggleGoToLine.of(true) });
  return true;
}

export const goToLine: Extension = goToLineField;

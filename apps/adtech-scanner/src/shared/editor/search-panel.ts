import {
  closeSearchPanel,
  findNext,
  findPrevious,
  getSearchQuery,
  SearchQuery,
  setSearchQuery,
} from "@codemirror/search";
import { EditorSelection } from "@codemirror/state";
import type { EditorView, Panel, ViewUpdate } from "@codemirror/view";
import { describeMatches, searchSummaryField } from "./search-summary";

const ICON_PATHS = {
  previous: "m18 15-6-6-6 6",
  next: "m6 9 6 6 6-6",
  close: "M18 6 6 18M6 6l12 12",
} as const;

function icon(name: keyof typeof ICON_PATHS): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("class", "size-3.5 fill-none stroke-current stroke-2");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", ICON_PATHS[name]);
  path.setAttribute("stroke-linecap", "round");
  path.setAttribute("stroke-linejoin", "round");
  svg.append(path);
  return svg;
}

const BUTTON_CLASS =
  "flex h-6 min-w-6 shrink-0 items-center justify-center rounded-md px-1 font-mono text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground aria-pressed:bg-primary-soft aria-pressed:text-primary-text";

function button(label: string, content: string | SVGSVGElement, onClick: () => void): HTMLButtonElement {
  const element = document.createElement("button");
  element.type = "button";
  element.className = BUTTON_CLASS;
  element.title = label;
  element.setAttribute("aria-label", label);
  element.append(content);
  element.addEventListener("click", onClick);
  return element;
}

/**
 * The find bar. Built from DOM nodes because CodeMirror owns the panel; it reads the same design
 * tokens as the rest of the app. Enter and Shift+Enter step through matches, Escape closes.
 */
export function createSearchPanel(view: EditorView): Panel {
  const dom = document.createElement("div");
  dom.className = "flex items-center gap-1 px-2 py-1.5";
  dom.setAttribute("role", "search");
  dom.setAttribute("aria-label", "Find in this file");

  const input = document.createElement("input");
  input.type = "text";
  input.className =
    "h-6 w-56 min-w-0 rounded-md border border-input bg-background px-2 font-mono text-xs outline-none placeholder:text-subtle-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40";
  input.placeholder = "Find";
  input.spellcheck = false;
  input.setAttribute("aria-label", "Find");
  // The attribute CodeMirror looks for when it moves focus into a freshly opened panel.
  input.setAttribute("main-field", "true");

  const count = document.createElement("span");
  count.className = "min-w-20 px-1.5 text-xs text-muted-foreground tabular-nums";
  count.setAttribute("role", "status");
  count.setAttribute("aria-live", "polite");

  const flags = { caseSensitive: false, regexp: false, wholeWord: false };

  const commit = () => {
    const query = new SearchQuery({ search: input.value, ...flags });
    const { main } = view.state.selection;
    view.dispatch({
      effects: setSearchQuery.of(query),
      // Collapsing to the start keeps the match under the caret selected while the query still fits it.
      selection: EditorSelection.cursor(main.from),
    });
    if (query.valid) findNext(view);
  };

  const toggle = (flag: keyof typeof flags, label: string, text: string) => {
    const element = button(label, text, () => {
      flags[flag] = !flags[flag];
      element.setAttribute("aria-pressed", String(flags[flag]));
      commit();
      input.focus();
    });
    element.setAttribute("aria-pressed", "false");
    return element;
  };

  input.addEventListener("input", commit);
  dom.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && event.target === input) {
      event.preventDefault();
      (event.shiftKey ? findPrevious : findNext)(view);
    } else if (event.key === "Escape") {
      event.preventDefault();
      closeSearchPanel(view);
      view.focus();
    }
  });

  dom.append(
    input,
    toggle("caseSensitive", "Match case", "Aa"),
    toggle("wholeWord", "Whole word", "ab"),
    toggle("regexp", "Regular expression", ".*"),
    count,
    button("Previous match (Shift+Enter)", icon("previous"), () => findPrevious(view)),
    button("Next match (Enter)", icon("next"), () => findNext(view)),
    button("Close (Escape)", icon("close"), () => {
      closeSearchPanel(view);
      view.focus();
    }),
  );

  const render = () => {
    const query = getSearchQuery(view.state);
    if (input.value === "" || !query.valid) {
      count.textContent = input.value === "" ? "" : "Invalid pattern";
      return;
    }
    count.textContent = describeMatches(view.state.field(searchSummaryField), view.state.selection.main.from);
  };

  return {
    dom,
    top: true,
    mount() {
      // Opening with a selection searches for it, as editors do.
      const { main } = view.state.selection;
      const selected = main.empty ? "" : view.state.sliceDoc(main.from, Math.min(main.to, main.from + 200));
      if (selected && !selected.includes("\n")) {
        input.value = selected;
        view.dispatch({ effects: setSearchQuery.of(new SearchQuery({ search: selected, ...flags })) });
      } else {
        input.value = getSearchQuery(view.state).search;
      }
      input.select();
      render();
    },
    update(update: ViewUpdate) {
      if (update.selectionSet || update.docChanged || update.transactions.some((tr) => tr.effects.length > 0)) render();
    },
  };
}

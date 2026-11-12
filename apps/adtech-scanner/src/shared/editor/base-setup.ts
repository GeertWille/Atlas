import { standardKeymap } from "@codemirror/commands";
import { bracketMatching, codeFolding, foldGutter, foldKeymap, syntaxHighlighting } from "@codemirror/language";
import {
  closeSearchPanel,
  findNext,
  findPrevious,
  highlightSelectionMatches,
  openSearchPanel,
  search,
} from "@codemirror/search";
import { EditorState, type Extension } from "@codemirror/state";
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { classHighlighter } from "@lezer/highlight";
import { goToLine, openGoToLine } from "./go-to-line";
import { flashField } from "./navigation";
import { createSearchPanel } from "./search-panel";
import { searchSummaryField } from "./search-summary";
import { specialChars } from "./special-chars";
import { editorTheme } from "./theme";

/**
 * A read-only code view that still takes focus: there is a caret, keyboard selection and find, and
 * nothing can be typed. Tab is deliberately unbound, so focus is never trapped in the code.
 */
export function baseSetup(label: string): Extension {
  return [
    EditorState.readOnly.of(true),
    EditorView.contentAttributes.of({
      // Editable content takes focus without this; saying so lets tools that only read the attribute see it.
      tabindex: "0",
      "aria-label": label,
      "aria-readonly": "true",
      spellcheck: "false",
      autocorrect: "off",
      autocapitalize: "off",
    }),
    editorTheme,
    lineNumbers(),
    codeFolding(),
    foldGutter(),
    specialChars,
    drawSelection(),
    highlightActiveLine(),
    highlightActiveLineGutter(),
    bracketMatching(),
    syntaxHighlighting(classHighlighter),
    highlightSelectionMatches({ minSelectionLength: 2 }),
    search({ top: true, createPanel: createSearchPanel }),
    searchSummaryField,
    goToLine,
    flashField,
    keymap.of([
      { key: "Mod-f", run: openSearchPanel, scope: "editor search-panel" },
      {
        key: "F3",
        mac: "Cmd-g",
        run: findNext,
        shift: findPrevious,
        scope: "editor search-panel",
        preventDefault: true,
      },
      { key: "Escape", run: closeSearchPanel, scope: "editor search-panel" },
      { key: "Ctrl-g", run: openGoToLine },
      ...foldKeymap,
      ...standardKeymap,
    ]),
  ];
}

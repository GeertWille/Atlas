import { EditorSelection, type StateEffect } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

type Place = { scroll: StateEffect<unknown>; anchor: number; head: number };

/** Where each file was left, for as long as the window lives: per scan, script and view. */
const places = new Map<string, Place>();

export function rememberPlace(key: string, view: EditorView) {
  const { anchor, head } = view.state.selection.main;
  places.set(key, { scroll: view.scrollSnapshot(), anchor, head });
}

/** Puts a freshly opened file back where it was left. Does nothing for a file seen for the first time. */
export function restorePlace(key: string, view: EditorView) {
  const place = places.get(key);
  if (!place) return;
  const { length } = view.state.doc;
  view.dispatch({
    selection: EditorSelection.single(Math.min(place.anchor, length), Math.min(place.head, length)),
    effects: place.scroll,
  });
}

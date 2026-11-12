import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView, type ViewUpdate } from "@codemirror/view";
import { useEffect, useEffectEvent, useState } from "react";
import { baseSetup } from "./base-setup";
import { languageExtension } from "./languages";
import { hasLongLine } from "./long-lines";
import type { EditorLanguage } from "./mime";

export type CodeMirrorOptions = {
  /** The text shown. A new value replaces the document and resets the view. */
  doc: string;
  language: EditorLanguage;
  wrap: boolean;
  /** Accessible name of the code region. */
  label: string;
  /** What the screen adds on top of the shared setup: decorations, gutters, keys. Keep the reference stable. */
  extensions?: Extension;
  onUpdate?: (update: ViewUpdate) => void;
};

const NO_EXTENSIONS: Extension = [];

/**
 * A read-only CodeMirror view bound to a host element. Language, wrapping and the caller's extensions
 * live in compartments, so changing one of them never rebuilds the view or loses the scroll position.
 */
export function useCodeMirror({ doc, language, wrap, label, extensions = NO_EXTENSIONS, onUpdate }: CodeMirrorOptions) {
  const [host, setHost] = useState<HTMLElement | null>(null);
  const [view, setView] = useState<EditorView | null>(null);
  const [compartments] = useState(() => ({
    language: new Compartment(),
    wrap: new Compartment(),
    extensions: new Compartment(),
  }));
  // One very long line is costlier to tokenise and to measure than it is worth.
  const guarded = hasLongLine(doc);
  const effectiveLanguage = guarded ? "text" : language;
  const effectiveWrap = guarded || wrap;
  const notify = useEffectEvent((update: ViewUpdate) => onUpdate?.(update));

  // biome-ignore lint/correctness/useExhaustiveDependencies: the three compartments are reconfigured by the effects below, not by rebuilding the view
  useEffect(() => {
    if (!host) return;
    const created = new EditorView({
      parent: host,
      state: EditorState.create({
        doc,
        extensions: [
          baseSetup(label),
          compartments.language.of(languageExtension(effectiveLanguage)),
          compartments.wrap.of(effectiveWrap ? EditorView.lineWrapping : []),
          compartments.extensions.of(extensions),
          EditorView.updateListener.of((update) => notify(update)),
        ],
      }),
    });
    setView(created);
    return () => {
      created.destroy();
      setView(null);
    };
  }, [host, doc, label, compartments]);

  useEffect(() => {
    view?.dispatch({ effects: compartments.language.reconfigure(languageExtension(effectiveLanguage)) });
  }, [view, compartments, effectiveLanguage]);

  useEffect(() => {
    view?.dispatch({ effects: compartments.wrap.reconfigure(effectiveWrap ? EditorView.lineWrapping : []) });
  }, [view, compartments, effectiveWrap]);

  useEffect(() => {
    view?.dispatch({ effects: compartments.extensions.reconfigure(extensions) });
  }, [view, compartments, extensions]);

  return {
    /** Ref callback for the element the editor fills. */
    mount: setHost,
    view,
    /** Highlighting is off and wrapping forced, because a line is too long. */
    guarded,
  };
}

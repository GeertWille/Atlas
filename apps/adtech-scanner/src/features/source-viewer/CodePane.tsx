import type { Extension } from "@codemirror/state";
import type { EditorView, ViewUpdate } from "@codemirror/view";
import { useEffect } from "react";
import type { SourceRegion } from "@/bindings";
import { useCodeMirror } from "@/shared/editor";
import type { FindingMark } from "./anchors";
import { OverviewRuler, type RulerViewport } from "./OverviewRuler";

/**
 * The code and its overview ruler. The parent owns everything about what is shown and where the
 * caret goes; this is the mounting point of the editor.
 */
export function CodePane({
  text,
  label,
  wrap,
  extensions,
  marks,
  regions,
  hitLines,
  lineCount,
  viewport,
  onView,
  onUpdate,
  onGuarded,
  onJump,
}: {
  text: string;
  label: string;
  wrap: boolean;
  extensions: Extension;
  marks: FindingMark[];
  regions: SourceRegion[];
  hitLines: number[];
  lineCount: number;
  viewport: RulerViewport | null;
  /** The editor of this text, once it exists; `null` again when it is gone. */
  onView: (view: EditorView | null) => void;
  onUpdate: (update: ViewUpdate) => void;
  /** Highlighting was turned off for this text because a line is too long. */
  onGuarded: (guarded: boolean) => void;
  onJump: (line: number) => void;
}) {
  const { mount, view, guarded } = useCodeMirror({
    doc: text,
    language: "javascript",
    wrap,
    label,
    extensions,
    onUpdate,
  });

  useEffect(() => {
    onView(view);
    return () => onView(null);
  }, [view, onView]);

  useEffect(() => {
    onGuarded(guarded);
  }, [guarded, onGuarded]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      <div ref={mount} data-testid="code" className="min-h-0 min-w-0 flex-1 overflow-hidden" />
      <OverviewRuler
        marks={marks}
        regions={regions}
        hitLines={hitLines}
        lineCount={lineCount}
        viewport={viewport}
        onJump={onJump}
      />
    </div>
  );
}

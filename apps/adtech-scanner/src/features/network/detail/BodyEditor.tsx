import type { ViewUpdate } from "@codemirror/view";
import { useState } from "react";
import { CopyButton } from "@/shared/components/copy-button";
import { type EditorLanguage, useCodeMirror } from "@/shared/editor";
import { jsonPathAt } from "./json-path";

/**
 * A body as read-only text. Whatever a server or a page sent is shown as characters and nothing
 * else: never rendered, never followed, never run. For JSON the path of the value under the caret
 * is shown and can be copied.
 */
export default function BodyEditor({
  text,
  language,
  wrap,
  label,
}: {
  text: string;
  language: EditorLanguage;
  wrap: boolean;
  label: string;
}) {
  const [path, setPath] = useState<string | null>(null);
  const onUpdate = (update: ViewUpdate) => {
    if (language !== "json" || !(update.selectionSet || update.docChanged)) return;
    setPath(jsonPathAt(update.state, update.state.selection.main.head));
  };
  const { mount, guarded } = useCodeMirror({ doc: text, language, wrap, label, onUpdate });
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div ref={mount} data-testid="body-editor" className="min-h-0 flex-1 overflow-hidden" />
      {language === "json" || guarded ? (
        <div className="flex h-6 shrink-0 items-center gap-2 border-t bg-sidebar px-2 text-[11px] text-muted-foreground">
          {guarded ? <span>Highlighting is off: this body has very long lines.</span> : null}
          {language === "json" && !guarded ? (
            path ? (
              <>
                <span className="shrink-0">Path</span>
                <code data-testid="json-path" className="selectable min-w-0 truncate font-mono text-foreground">
                  {path}
                </code>
                <CopyButton text={path} label="Copy path" className="size-5 [&_svg]:size-3!" />
              </>
            ) : (
              <span>Put the caret on a value to see its path.</span>
            )
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

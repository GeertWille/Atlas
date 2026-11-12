import { javascriptLanguage } from "@codemirror/lang-javascript";
import { jsonLanguage } from "@codemirror/lang-json";
import { xmlLanguage } from "@codemirror/lang-xml";
import type { Extension } from "@codemirror/state";

export type EditorLanguage = "javascript" | "json" | "xml" | "text";

/** Grammar only: none of the language packages' completion, linting or tag-closing is loaded. */
export function languageExtension(language: EditorLanguage): Extension {
  switch (language) {
    case "javascript":
      return javascriptLanguage;
    case "json":
      return jsonLanguage;
    case "xml":
      return xmlLanguage;
    case "text":
      return [];
  }
}

/** The grammar for a MIME type; anything unknown is shown as plain text. */
export function languageForMime(mime: string | null | undefined): EditorLanguage {
  const type = (mime ?? "").split(";")[0]?.trim().toLowerCase() ?? "";
  if (type.endsWith("json")) return "json";
  if (type.includes("javascript") || type.includes("ecmascript")) return "javascript";
  if (type.endsWith("xml") && !type.includes("html")) return "xml";
  return "text";
}

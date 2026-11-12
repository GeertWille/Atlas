import { javascriptLanguage } from "@codemirror/lang-javascript";
import { jsonLanguage } from "@codemirror/lang-json";
import { xmlLanguage } from "@codemirror/lang-xml";
import type { Extension } from "@codemirror/state";

import type { EditorLanguage } from "./mime";

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

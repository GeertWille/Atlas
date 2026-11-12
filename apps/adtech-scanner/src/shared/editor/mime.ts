/** The grammars the editor can highlight. Kept apart from the editor itself, so choosing one loads no editor code. */
export type EditorLanguage = "javascript" | "json" | "xml" | "text";

/** The grammar for a MIME type; anything unknown is shown as plain text. */
export function languageForMime(mime: string | null | undefined): EditorLanguage {
  const type = (mime ?? "").split(";")[0]?.trim().toLowerCase() ?? "";
  if (type.endsWith("json")) return "json";
  if (type.includes("javascript") || type.includes("ecmascript")) return "javascript";
  if (type.endsWith("xml") && !type.includes("html")) return "xml";
  return "text";
}

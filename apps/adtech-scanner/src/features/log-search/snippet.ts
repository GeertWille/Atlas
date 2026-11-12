export type SnippetPart = { text: string; marked: boolean };

const ENTITIES: Record<string, string> = {
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
  "&#39;": "'",
  "&#x27;": "'",
};

function decodeEntities(text: string): string {
  return text.replace(/&(amp|lt|gt|quot|#39|#x27);/g, (entity) => ENTITIES[entity] ?? entity);
}

/**
 * Parses a backend search snippet into plain-text parts. The snippet is
 * HTML-escaped except for `<mark>` / `</mark>`; nothing else is ever
 * interpreted as markup, so the result is safe to render as text nodes.
 */
export function parseSnippet(snippet: string): SnippetPart[] {
  const parts: SnippetPart[] = [];
  let marked = false;
  for (const token of snippet.split(/(<\/?mark>)/)) {
    if (token === "<mark>") {
      marked = true;
    } else if (token === "</mark>") {
      marked = false;
    } else if (token !== "") {
      parts.push({ text: decodeEntities(token), marked });
    }
  }
  return parts;
}

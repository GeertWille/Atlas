import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import type { EditorState } from "@codemirror/state";
import type { SyntaxNode } from "@lezer/common";

const IDENTIFIER = /^[A-Za-z_$][\w$]*$/;
const PARSE_BUDGET_MS = 50;

function keyOf(state: EditorState, property: SyntaxNode): string {
  const name = property.getChild("PropertyName");
  const raw = name ? state.sliceDoc(name.from, name.to) : "";
  try {
    return JSON.parse(raw) as string;
  } catch {
    return raw.replace(/^"|"$/g, "");
  }
}

/** Position of a value among the values of its array. */
function indexIn(array: SyntaxNode, value: SyntaxNode): number {
  let index = 0;
  for (let child = array.firstChild; child; child = child.nextSibling) {
    if (child.from === value.from && child.to === value.to) return index;
    // Brackets and commas are nodes too; only values count.
    if (child.name !== "[" && child.name !== "]" && child.name !== ",") index += 1;
  }
  return index;
}

/**
 * The path of the JSON value under a position: `seatbid[0].bid[2].price`. `null` outside any value,
 * and at the root. Read from the syntax tree the editor already has, so nothing is parsed twice.
 */
export function jsonPathAt(state: EditorState, position: number): string | null {
  const tree = ensureSyntaxTree(state, position, PARSE_BUDGET_MS) ?? syntaxTree(state);
  const parts: string[] = [];
  let node: SyntaxNode | null = tree.resolveInner(position, -1);
  // On the punctuation between values, the value to the right is the one meant.
  if (node.name === "JsonText" || node.name === "Object" || node.name === "Array")
    node = tree.resolveInner(position, 1);
  for (; node; node = node.parent) {
    const { parent } = node;
    if (!parent) break;
    if (parent.name === "Array" && node.name !== "[" && node.name !== "]" && node.name !== ",") {
      parts.unshift(`[${indexIn(parent, node)}]`);
    } else if (node.name === "Property") {
      const key = keyOf(state, node);
      parts.unshift(IDENTIFIER.test(key) ? `.${key}` : `[${JSON.stringify(key)}]`);
    }
  }
  if (parts.length === 0) return null;
  return parts.join("").replace(/^\./, "");
}

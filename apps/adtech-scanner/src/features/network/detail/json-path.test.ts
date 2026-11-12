import { jsonLanguage } from "@codemirror/lang-json";
import { EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import { jsonPathAt } from "./json-path";

const DOC = `{
  "id": "7f3a",
  "seatbid": [
    {
      "seat": "pubmatic",
      "bid": [
        { "impid": "2a", "price": 1.42, "adomain": ["acme.example"] },
        { "impid": "2c", "price": 0.61 }
      ]
    }
  ],
  "ext": { "response time": { "pubmatic": 140 }, "ok": true, "none": null }
}`;

const state = EditorState.create({ doc: DOC, extensions: [jsonLanguage] });
const at = (needle: string, occurrence = 0) => {
  let index = -1;
  for (let count = 0; count <= occurrence; count += 1) index = DOC.indexOf(needle, index + 1);
  return jsonPathAt(state, index + 1);
};

describe("jsonPathAt", () => {
  it("names a top-level key from its name or its value", () => {
    expect(at('"id"')).toBe("id");
    expect(at('"7f3a"')).toBe("id");
  });

  it("walks through arrays by index and objects by key", () => {
    expect(at("1.42")).toBe("seatbid[0].bid[0].price");
    expect(at("0.61")).toBe("seatbid[0].bid[1].price");
    expect(at('"2c"')).toBe("seatbid[0].bid[1].impid");
    expect(at('"acme.example"')).toBe("seatbid[0].bid[0].adomain[0]");
  });

  it("quotes a key that is not an identifier", () => {
    expect(at("140")).toBe('ext["response time"].pubmatic');
  });

  it("works on literals", () => {
    expect(at("true")).toBe("ext.ok");
    expect(at("null")).toBe("ext.none");
  });

  it("names the container when the caret is on its bracket", () => {
    expect(jsonPathAt(state, DOC.indexOf("[") + 1)).toBe("seatbid");
  });

  it("has no path at the root or in text that is not JSON", () => {
    expect(jsonPathAt(state, 0)).toBeNull();
    const plain = EditorState.create({ doc: "not json at all", extensions: [jsonLanguage] });
    expect(jsonPathAt(plain, 4)).toBeNull();
  });
});

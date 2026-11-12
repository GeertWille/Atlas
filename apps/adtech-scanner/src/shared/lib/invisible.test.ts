import { expect, test } from "vitest";
import { revealInvisibles } from "./invisible";

test("bidirectional controls and zero-width characters are written out as code points", () => {
  expect(revealInvisibles("user\u202E // admins only\u2069")).toBe("user⟨U+202E⟩ // admins only⟨U+2069⟩");
  expect(revealInvisibles("pay\u200Bpal.com")).toBe("pay⟨U+200B⟩pal.com");
  expect(revealInvisibles("a\u2066b\u2067c\u2068d\u202Ae\u202Bf\u202Cg\u202Dh")).toBe(
    "a⟨U+2066⟩b⟨U+2067⟩c⟨U+2068⟩d⟨U+202A⟩e⟨U+202B⟩f⟨U+202C⟩g⟨U+202D⟩h",
  );
  expect(revealInvisibles("\uFEFFbom, soft\u00ADhyphen, \u200Emark\u200F, \u061Calm, \u2060joiner")).toBe(
    "⟨U+FEFF⟩bom, soft⟨U+00AD⟩hyphen, ⟨U+200E⟩mark⟨U+200F⟩, ⟨U+061C⟩alm, ⟨U+2060⟩joiner",
  );
});

test("ordinary text, right-to-left letters and emoji are left alone", () => {
  for (const text of ["example-advertiser.test", "שלום עולם", "مرحبا", "crid-42 👍🏽", "line\nbreak\ttab", ""]) {
    expect(revealInvisibles(text)).toBe(text);
  }
});

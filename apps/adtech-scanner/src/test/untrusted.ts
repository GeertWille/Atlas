import { expect } from "vitest";

/** Text as a scanned page, an exchange or a model could send it: markup, script URLs, styles and invisible characters. */
export const HOSTILE = {
  image: `<img src=x onerror="document.title='owned'">`,
  link: `<a href="javascript:document.title='owned'" target="_blank">click</a>`,
  style: "<style>*{display:none!important}</style>",
  script: `"></span><script>document.title='owned'</script>`,
  scriptUrl: "javascript:document.title='owned'",
  remoteUrl: "https://evil.example/pixel.gif",
  bidi: "user\u202E // admins only\u2069",
  zeroWidth: "pay\u200Bpal.com",
};

/**
 * Whatever was rendered from untrusted text is text: no element that loads, runs or restyles anything,
 * no link that leaves the app or runs script, no event handler, and no invisible character left in place.
 */
export function expectInert(root: HTMLElement) {
  expect(
    root.querySelectorAll("img, picture, video, audio, script, style, iframe, object, embed, link, form, base"),
  ).toHaveLength(0);
  for (const anchor of root.querySelectorAll("a")) {
    // The app's own links are routes; none opens a window or carries a scheme.
    expect(anchor.getAttribute("href") ?? "/").toMatch(/^\/(?!\/)/);
    expect(anchor.hasAttribute("target")).toBe(false);
  }
  for (const element of root.querySelectorAll("*")) {
    for (const attribute of element.attributes) {
      expect(attribute.name.startsWith("on"), `${element.tagName} ${attribute.name}`).toBe(false);
      if (attribute.name === "style") expect(attribute.value).not.toMatch(/display:\s*none|url\(|expression/i);
      if (attribute.name === "href" || attribute.name === "src")
        expect(attribute.value).not.toMatch(/^\s*javascript:/i);
    }
  }
  expect(root.textContent).not.toMatch(/[\u200b-\u200f\u202a-\u202e\u2066-\u2069]/);
  expect(document.title).not.toContain("owned");
}

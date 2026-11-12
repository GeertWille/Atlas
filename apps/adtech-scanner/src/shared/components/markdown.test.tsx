import { render } from "@testing-library/react";
import { expect, test } from "vitest";
import { expectInert, HOSTILE } from "@/test/untrusted";
import { Markdown } from "./markdown";

test("a model's markdown is shown as text: no HTML, no link, no image, no hidden characters", () => {
  const text = [
    `Raw markup ${HOSTILE.image} ${HOSTILE.link} ${HOSTILE.style} ${HOSTILE.script}`,
    `A [link](${HOSTILE.scriptUrl}) and [another](${HOSTILE.remoteUrl}), an image ![tracking pixel](${HOSTILE.remoteUrl}).`,
    `<${HOSTILE.remoteUrl}> and ${HOSTILE.bidi} and ${HOSTILE.zeroWidth}`,
    "```html\n<script>document.title='owned'</script>\n```",
  ].join("\n\n");
  const { container } = render(<Markdown>{text}</Markdown>);
  expectInert(container);
  expect(container.querySelectorAll("a")).toHaveLength(0);
  // The words are all still there to read, the markup as characters.
  expect(container.textContent).toContain(HOSTILE.image);
  expect(container.textContent).toContain("A link and another, an image tracking pixel.");
  expect(container.textContent).toContain("user⟨U+202E⟩ // admins only⟨U+2069⟩");
  expect(container.textContent).toContain("pay⟨U+200B⟩pal.com");
  expect(container.textContent).toContain("<script>document.title='owned'</script>");
});

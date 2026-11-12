import { Highlight, type PrismTheme } from "prism-react-renderer";
import { cn } from "@/shared/lib/cn";

const THEME: PrismTheme = {
  plain: { color: "var(--foreground)" },
  styles: [
    { types: ["comment", "prolog"], style: { color: "var(--syn-comment)", fontStyle: "italic" } },
    { types: ["keyword", "boolean", "operator", "null"], style: { color: "var(--syn-keyword)" } },
    { types: ["string", "attr-value", "template-string", "regex"], style: { color: "var(--syn-string)" } },
    { types: ["number"], style: { color: "var(--syn-number)" } },
    { types: ["function", "property", "class-name"], style: { color: "var(--syn-function)" } },
    { types: ["punctuation"], style: { color: "var(--syn-punctuation)" } },
  ],
};

export function CodeBlock({
  code,
  language = "javascript",
  startLine,
  className,
}: {
  code: string;
  language?: "javascript" | "json";
  /** First line number of the excerpt; omit to hide the gutter. */
  startLine?: number;
  className?: string;
}) {
  return (
    <Highlight code={code.replace(/\n$/, "")} language={language} theme={THEME}>
      {({ tokens, getLineProps, getTokenProps }) => (
        <pre
          // Scrollable regions must be keyboard reachable.
          // biome-ignore lint/a11y/noNoninteractiveTabindex: scroll container
          tabIndex={0}
          className={cn(
            "selectable overflow-x-auto bg-code py-2.5 font-mono text-xs leading-[1.6]",
            startLine === undefined && "px-3",
            className,
          )}
        >
          <code className="block w-max min-w-full">
            {tokens.map((line, index) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: lines of a static excerpt never reorder
              <span key={index} {...getLineProps({ line })} className="flex">
                {startLine !== undefined ? (
                  <span
                    aria-hidden="true"
                    className="w-11 shrink-0 pr-3 text-right text-subtle-foreground/70 tabular-nums select-none"
                  >
                    {startLine + index}
                  </span>
                ) : null}
                <span className="pr-3 whitespace-pre">
                  {line.map((token, tokenIndex) => (
                    // biome-ignore lint/suspicious/noArrayIndexKey: tokens of a static line never reorder
                    <span key={tokenIndex} {...getTokenProps({ token })} />
                  ))}
                </span>
              </span>
            ))}
          </code>
        </pre>
      )}
    </Highlight>
  );
}

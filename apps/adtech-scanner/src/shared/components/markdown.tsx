import ReactMarkdown, { type Components } from "react-markdown";
import { cn } from "@/shared/lib/cn";

const COMPONENTS: Components = {
  p: ({ children }) => <p className="text-pretty">{children}</p>,
  strong: ({ children }) => <strong className="font-semibold text-foreground">{children}</strong>,
  ul: ({ children }) => <ul className="flex list-disc flex-col gap-1 pl-5">{children}</ul>,
  ol: ({ children }) => <ol className="flex list-decimal flex-col gap-1 pl-5">{children}</ol>,
  code: ({ children }) => (
    <code className="rounded-[5px] border bg-muted/70 px-1 py-px font-mono text-[0.85em] text-foreground">
      {children}
    </code>
  ),
  pre: ({ children }) => (
    <pre className="overflow-x-auto rounded-md border bg-code p-3 font-mono text-xs [&_code]:border-0 [&_code]:bg-transparent [&_code]:p-0">
      {children}
    </pre>
  ),
  // Links in model output are shown as text: nothing in a finding should navigate the app shell.
  a: ({ children }) => <span className="underline decoration-border-strong underline-offset-2">{children}</span>,
};

/** Renders trusted-shape markdown from findings. Raw HTML is not parsed (react-markdown default). */
export function Markdown({ children, className }: { children: string; className?: string }) {
  return (
    <div className={cn("selectable flex flex-col gap-2.5 text-[13px] leading-relaxed text-muted-foreground", className)}>
      <ReactMarkdown components={COMPONENTS}>{children}</ReactMarkdown>
    </div>
  );
}

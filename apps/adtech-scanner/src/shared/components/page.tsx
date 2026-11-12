import type { ReactNode } from "react";
import { useDocumentTitle } from "@/shared/hooks/use-document-title";
import { cn } from "@/shared/lib/cn";

/**
 * A routed screen: a draggable title bar (the window has no native one) and a
 * scrolling body. Interactive children of the bar opt out of dragging.
 */
export function Page({
  title,
  documentTitle,
  breadcrumb,
  actions,
  children,
  className,
  scroll = true,
}: {
  title: string;
  documentTitle?: string;
  breadcrumb?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  /** Set false when the page manages its own scroll regions. */
  scroll?: boolean;
}) {
  useDocumentTitle(documentTitle ?? title);
  return (
    <div className="flex h-full min-w-0 flex-col">
      <header
        data-tauri-drag-region
        className="drag-region flex h-12 shrink-0 items-center justify-between gap-4 border-b px-5"
      >
        <div data-tauri-drag-region className="flex min-w-0 items-center gap-1.5 text-sm">
          {breadcrumb}
          <h1 className="truncate font-semibold">{title}</h1>
        </div>
        {actions ? <div className="no-drag flex shrink-0 items-center gap-2">{actions}</div> : null}
      </header>
      <main className={cn("min-h-0 flex-1", scroll && "overflow-y-auto", className)}>{children}</main>
    </div>
  );
}

export function PageContent({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("mx-auto flex w-full max-w-[1180px] flex-col gap-6 px-6 py-6", className)}>{children}</div>;
}

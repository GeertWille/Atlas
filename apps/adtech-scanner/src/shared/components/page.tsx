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
  children,
  scroll = true,
}: {
  title: string;
  documentTitle?: string;
  breadcrumb?: ReactNode;
  children: ReactNode;
  /** Set false when the page manages its own scroll regions. */
  scroll?: boolean;
}) {
  useDocumentTitle(documentTitle ?? title);
  return (
    <div className="flex h-full min-w-0 flex-col">
      <header data-tauri-drag-region="deep" className="drag-region flex h-12 shrink-0 items-center border-b px-5">
        <div className="flex min-w-0 items-center gap-1.5 text-sm">
          {breadcrumb}
          <h1 className="truncate font-semibold">{title}</h1>
        </div>
      </header>
      {/*
        Positioned, so that it contains the absolutely placed helpers inside it (screen-reader text,
        the hidden inputs of switches and selects). Left to the document, those stretch the page
        below the window, and anything scrolled into view can then shift the whole app upwards.
      */}
      <main className={cn("relative min-h-0 flex-1", scroll && "overflow-y-auto")}>{children}</main>
    </div>
  );
}

export function PageContent({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("mx-auto flex w-full max-w-[1180px] flex-col gap-6 px-6 py-6", className)}>{children}</div>;
}

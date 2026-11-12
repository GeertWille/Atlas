import type { ComponentProps, ReactNode } from "react";
import { cn } from "@/shared/lib/cn";

export function Panel({ className, ...props }: ComponentProps<"section">) {
  return <section className={cn("rounded-xl border bg-card shadow-xs", className)} {...props} />;
}

export function PanelHeader({
  title,
  description,
  actions,
  className,
}: {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex items-center justify-between gap-4 border-b px-4 py-3", className)}>
      <div className="flex min-w-0 flex-col gap-0.5">
        <h2 className="truncate text-sm font-semibold">{title}</h2>
        {description ? <p className="text-[13px] text-muted-foreground">{description}</p> : null}
      </div>
      {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
    </div>
  );
}

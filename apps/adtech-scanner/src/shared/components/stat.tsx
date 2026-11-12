import type { ReactNode } from "react";
import { cn } from "@/shared/lib/cn";

/** Stat tile: sentence-case label, value in proportional figures, optional context line. */
export function Stat({
  label,
  value,
  hint,
  className,
}: {
  label: string;
  value: ReactNode;
  hint?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex min-w-0 flex-col gap-1 px-4 py-3.5", className)}>
      <dt className="truncate text-[13px] text-muted-foreground">{label}</dt>
      <dd className="selectable truncate text-xl leading-7 font-semibold tracking-tight">{value}</dd>
      {hint ? <dd className="truncate text-xs text-subtle-foreground">{hint}</dd> : null}
    </div>
  );
}

export function StatGrid({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <dl className={cn("grid overflow-hidden rounded-xl border bg-border shadow-xs [&>*]:bg-card gap-px", className)}>
      {children}
    </dl>
  );
}

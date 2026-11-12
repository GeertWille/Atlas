import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/shared/lib/cn";

function Empty({
  icon: Icon,
  title,
  description,
  tone = "neutral",
  children,
  className,
}: {
  icon: LucideIcon;
  title: string;
  description?: ReactNode;
  tone?: "neutral" | "danger";
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div
      data-slot="empty"
      className={cn("flex flex-col items-center justify-center gap-3 px-6 py-14 text-center", className)}
    >
      <div
        className={cn(
          "flex size-10 items-center justify-center rounded-lg border shadow-xs",
          tone === "danger" ? "border-red-line bg-red-soft text-red-fg" : "bg-background text-muted-foreground",
        )}
      >
        <Icon className="size-5" />
      </div>
      <div className="flex max-w-sm flex-col gap-1">
        <p className="text-sm font-semibold">{title}</p>
        {description ? <p className="text-[13px] text-pretty text-muted-foreground">{description}</p> : null}
      </div>
      {children ? <div className="mt-1 flex items-center gap-2">{children}</div> : null}
    </div>
  );
}

export { Empty };

import type { ComponentProps } from "react";
import { cn } from "@/shared/lib/cn";

function Input({ className, type = "text", ...props }: ComponentProps<"input">) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        "selectable h-8 w-full min-w-0 rounded-md border border-input bg-background px-2.5 text-sm shadow-xs transition-[border-color,box-shadow] outline-none placeholder:text-subtle-foreground focus-visible:border-primary focus-visible:ring-3 focus-visible:ring-ring/30 disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-destructive/20",
        className,
      )}
      {...props}
    />
  );
}

export { Input };

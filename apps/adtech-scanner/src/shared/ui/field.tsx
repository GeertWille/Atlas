import type { ComponentProps, ReactNode } from "react";
import { cn } from "@/shared/lib/cn";
import { Label } from "./label";

function FieldGroup({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="field-group" className={cn("flex flex-col", className)} {...props} />;
}

/** Settings-style row: label and description on the left, control on the right. */
function Field({
  label,
  htmlFor,
  description,
  error,
  children,
  className,
  stacked = false,
}: {
  label: string;
  htmlFor?: string;
  description?: ReactNode;
  error?: string | null;
  children: ReactNode;
  className?: string;
  stacked?: boolean;
}) {
  return (
    <div
      data-slot="field"
      data-invalid={error ? true : undefined}
      className={cn(
        "flex gap-x-8 gap-y-2 border-b py-4 first:pt-0 last:border-b-0 last:pb-0",
        stacked ? "flex-col" : "items-start justify-between",
        className,
      )}
    >
      <div className="flex min-w-0 flex-col gap-1">
        <Label htmlFor={htmlFor}>{label}</Label>
        {description ? (
          <p className="text-[13px] text-pretty text-muted-foreground">{description}</p>
        ) : null}
        {error ? (
          <p role="alert" className="text-[13px] text-red-fg">
            {error}
          </p>
        ) : null}
      </div>
      <div className={cn("flex shrink-0 items-center gap-2", stacked && "w-full")}>{children}</div>
    </div>
  );
}

export { Field, FieldGroup };

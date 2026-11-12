import { cva, type VariantProps } from "class-variance-authority";
import type { ComponentProps } from "react";
import { cn } from "@/shared/lib/cn";

const alertVariants = cva(
  "grid grid-cols-[auto_1fr] items-start gap-x-2.5 gap-y-0.5 rounded-lg border px-3.5 py-3 text-[13px] [&>svg]:mt-0.5 [&>svg]:size-4",
  {
    variants: {
      tone: {
        neutral: "bg-muted/60 text-foreground [&>svg]:text-muted-foreground",
        warning: "border-amber-line bg-amber-soft text-amber-fg",
        danger: "border-red-line bg-red-soft text-red-fg",
      },
    },
    defaultVariants: { tone: "neutral" },
  },
);

function Alert({ className, tone, ...props }: ComponentProps<"div"> & VariantProps<typeof alertVariants>) {
  return <div data-slot="alert" role="alert" className={cn(alertVariants({ tone }), className)} {...props} />;
}

function AlertTitle({ className, ...props }: ComponentProps<"p">) {
  return <p className={cn("col-start-2 font-medium", className)} {...props} />;
}

function AlertDescription({ className, ...props }: ComponentProps<"div">) {
  return <div className={cn("selectable col-start-2 opacity-90", className)} {...props} />;
}

export { Alert, AlertDescription, AlertTitle };

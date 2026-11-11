import { cva, type VariantProps } from "class-variance-authority";
import type { ComponentProps } from "react";
import { cn } from "@/shared/lib/cn";

const badgeVariants = cva(
  "inline-flex h-5 shrink-0 items-center gap-1 rounded-full border px-2 text-xs font-medium whitespace-nowrap [&_svg]:size-3 [&_svg]:shrink-0",
  {
    variants: {
      tone: {
        red: "border-red-line bg-red-soft text-red-fg",
        orange: "border-orange-line bg-orange-soft text-orange-fg",
        amber: "border-amber-line bg-amber-soft text-amber-fg",
        green: "border-green-line bg-green-soft text-green-fg",
        blue: "border-blue-line bg-blue-soft text-blue-fg",
        violet: "border-violet-line bg-violet-soft text-violet-fg",
        gray: "border-gray-line bg-gray-soft text-gray-fg",
        outline: "border-border-strong bg-transparent text-muted-foreground",
      },
    },
    defaultVariants: { tone: "gray" },
  },
);

export type Tone = NonNullable<VariantProps<typeof badgeVariants>["tone"]>;

function Badge({
  className,
  tone,
  ...props
}: ComponentProps<"span"> & VariantProps<typeof badgeVariants>) {
  return <span data-slot="badge" className={cn(badgeVariants({ tone }), className)} {...props} />;
}

export { Badge };

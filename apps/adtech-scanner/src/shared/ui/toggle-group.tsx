import { ToggleGroup as ToggleGroupPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "@/shared/lib/cn";

/** Segmented control: single-select, always has a value. */
function ToggleGroup<T extends string>({
  className,
  value,
  onValueChange,
  size = "default",
  ...props
}: Omit<ComponentProps<typeof ToggleGroupPrimitive.Root>, "type" | "value" | "onValueChange" | "defaultValue"> & {
  value: T;
  onValueChange: (value: T) => void;
  size?: "default" | "sm";
}) {
  return (
    <ToggleGroupPrimitive.Root
      type="single"
      data-slot="toggle-group"
      data-size={size}
      value={value}
      onValueChange={(next) => {
        if (next) onValueChange(next as T);
      }}
      className={cn("group/toggle inline-flex items-center gap-0.5 rounded-lg bg-muted p-0.5", className)}
      {...props}
    />
  );
}

function ToggleGroupItem({ className, ...props }: ComponentProps<typeof ToggleGroupPrimitive.Item>) {
  return (
    <ToggleGroupPrimitive.Item
      data-slot="toggle-group-item"
      className={cn(
        "inline-flex h-7 items-center justify-center gap-1.5 rounded-md px-2.5 text-[13px] font-medium whitespace-nowrap text-muted-foreground transition-[background-color,color,box-shadow] group-data-[size=sm]/toggle:h-6 group-data-[size=sm]/toggle:px-2 group-data-[size=sm]/toggle:text-xs hover:text-foreground data-[state=on]:bg-background data-[state=on]:text-foreground data-[state=on]:shadow-xs dark:data-[state=on]:bg-accent [&_svg]:size-3.5",
        className,
      )}
      {...props}
    />
  );
}

export { ToggleGroup, ToggleGroupItem };

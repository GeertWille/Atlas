import { Tabs as TabsPrimitive } from "radix-ui";
import { type ComponentProps, useState } from "react";
import { cn } from "@/shared/lib/cn";

const Tabs = TabsPrimitive.Root;

function TabsList({ className, ...props }: ComponentProps<typeof TabsPrimitive.List>) {
  return (
    <TabsPrimitive.List
      data-slot="tabs-list"
      className={cn("flex items-center gap-5 border-b", className)}
      {...props}
    />
  );
}

function TabsTrigger({ className, ...props }: ComponentProps<typeof TabsPrimitive.Trigger>) {
  // Radix focuses the trigger from script on mouse down, and WebKit shows the focus
  // ring for every scripted focus. A tab focused by the pointer must not get one.
  const [focusedByPointer, setFocusedByPointer] = useState(false);
  return (
    <TabsPrimitive.Trigger
      data-slot="tabs-trigger"
      className={cn(
        "relative -mb-px flex h-9 items-center gap-1.5 border-b-2 border-transparent text-sm font-medium whitespace-nowrap text-muted-foreground transition-colors hover:text-foreground data-[state=active]:border-primary-text data-[state=active]:text-foreground [&_svg]:size-4",
        focusedByPointer && "outline-none",
        className,
      )}
      {...props}
      onPointerDown={() => setFocusedByPointer(true)}
      onBlur={() => setFocusedByPointer(false)}
    />
  );
}

function TabsContent({ className, ...props }: ComponentProps<typeof TabsPrimitive.Content>) {
  return <TabsPrimitive.Content data-slot="tabs-content" className={cn("outline-none", className)} {...props} />;
}

export { Tabs, TabsContent, TabsList, TabsTrigger };

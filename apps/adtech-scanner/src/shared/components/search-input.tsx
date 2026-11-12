import { SearchIcon, XIcon } from "lucide-react";
import type { ComponentProps } from "react";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Hint } from "@/shared/ui/tooltip";

export function SearchInput({
  value,
  onValueChange,
  className,
  inputClassName,
  ...props
}: Omit<ComponentProps<"input">, "value" | "onChange" | "type"> & {
  value: string;
  onValueChange: (value: string) => void;
  inputClassName?: string;
}) {
  return (
    <div className={cn("relative", className)}>
      <SearchIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-subtle-foreground" />
      <Input
        type="search"
        value={value}
        onChange={(event) => onValueChange(event.target.value)}
        className={cn("px-8 [&::-webkit-search-cancel-button]:hidden", inputClassName)}
        {...props}
      />
      {value ? (
        <Hint label="Clear search">
          <button
            type="button"
            aria-label="Clear search"
            onClick={() => onValueChange("")}
            className="absolute top-1/2 right-1.5 flex size-5 -translate-y-1/2 items-center justify-center rounded-sm text-subtle-foreground hover:bg-accent hover:text-foreground"
          >
            <XIcon className="size-3.5" />
          </button>
        </Hint>
      ) : null}
    </div>
  );
}

import { ChevronDownIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

export type FilterOption<T extends string> = { value: T; label: string; icon?: ReactNode; count?: number };

/** Multi-select filter. An empty selection means "no restriction". */
export function FilterMenu<T extends string>({
  label,
  options,
  selected,
  onChange,
}: {
  label: string;
  options: FilterOption<T>[];
  selected: readonly T[];
  onChange: (next: T[]) => void;
}) {
  const toggle = (value: T) =>
    onChange(selected.includes(value) ? selected.filter((item) => item !== value) : [...selected, value]);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" aria-label={`Filter by ${label.toLowerCase()}`}>
          {label}
          {selected.length > 0 ? (
            <span className="rounded-full bg-primary-soft px-1.5 text-[11px] leading-4 font-semibold text-primary-text tabular-nums">
              {selected.length}
            </span>
          ) : null}
          <ChevronDownIcon className="text-subtle-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        <DropdownMenuGroup>
          {options.map((option) => (
            <DropdownMenuCheckboxItem
              key={option.value}
              checked={selected.includes(option.value)}
              onCheckedChange={() => toggle(option.value)}
            >
              {option.icon}
              <span className="flex-1">{option.label}</span>
              {option.count !== undefined ? (
                <span className="text-xs text-subtle-foreground tabular-nums">{option.count}</span>
              ) : null}
            </DropdownMenuCheckboxItem>
          ))}
        </DropdownMenuGroup>
        {selected.length > 0 ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => onChange([])}>Clear filter</DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

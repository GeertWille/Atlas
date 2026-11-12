import { MinusIcon, PlusIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/shared/lib/cn";

const STEP_BUTTON =
  "flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-background hover:text-foreground hover:shadow-xs disabled:pointer-events-none disabled:opacity-40 dark:hover:bg-accent [&_svg]:size-3.5";

/** Whole-number control for a small range: one step down, the value, one step up. */
function Stepper({
  value,
  min,
  max,
  onValueChange,
  decreaseLabel,
  increaseLabel,
  children,
  className,
  id,
}: {
  value: number;
  min: number;
  max: number;
  onValueChange: (value: number) => void;
  decreaseLabel: string;
  increaseLabel: string;
  /** The value as shown, e.g. "3 pages". */
  children: ReactNode;
  className?: string;
  /** Lets a label name the value. */
  id?: string;
}) {
  return (
    <div data-slot="stepper" className={cn("inline-flex items-center gap-0.5 rounded-lg bg-muted p-0.5", className)}>
      <button
        type="button"
        aria-label={decreaseLabel}
        disabled={value <= min}
        onClick={() => onValueChange(value - 1)}
        className={STEP_BUTTON}
      >
        <MinusIcon />
      </button>
      <output id={id} aria-live="polite" className="min-w-16 px-1 text-center text-[13px] font-medium tabular-nums">
        {children}
      </output>
      <button
        type="button"
        aria-label={increaseLabel}
        disabled={value >= max}
        onClick={() => onValueChange(value + 1)}
        className={STEP_BUTTON}
      >
        <PlusIcon />
      </button>
    </div>
  );
}

export { Stepper };

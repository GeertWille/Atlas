import { CheckIcon, CopyIcon } from "lucide-react";
import { useCopy } from "@/shared/hooks/use-copy";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Hint } from "@/shared/ui/tooltip";

export function CopyButton({
  text,
  label = "Copy",
  className,
}: {
  /** Evaluated on click so large payloads are only serialised when needed. */
  text: string | (() => string);
  label?: string;
  className?: string;
}) {
  const { copied, copy } = useCopy();
  return (
    <Hint label={copied ? "Copied" : label}>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={label}
        className={cn("shrink-0", className)}
        onClick={(event) => {
          event.stopPropagation();
          void copy(typeof text === "function" ? text() : text);
        }}
      >
        {copied ? <CheckIcon className="text-green-fg" /> : <CopyIcon />}
      </Button>
    </Hint>
  );
}

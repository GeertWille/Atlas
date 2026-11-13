import { RotateCwIcon, TriangleAlertIcon } from "lucide-react";
import { toApiError } from "@/lib/api";
import { cn } from "@/shared/lib/cn";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";

/**
 * A fetch failed while what was loaded before is still on screen: the content stays, and this says
 * what is wrong with it (`detail`) and offers the retry. A first load that fails is a `QueryError`.
 */
export function RetryNotice({
  title,
  detail,
  error,
  onRetry,
  className,
}: {
  title: string;
  detail: string;
  error: unknown;
  onRetry: () => void;
  className?: string;
}) {
  return (
    <Alert tone="warning" className={cn("shrink-0", className)}>
      <TriangleAlertIcon />
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <span className="flex flex-col">
          <span>{toApiError(error).message}</span>
          <span>{detail}</span>
        </span>
        <Button variant="outline" size="sm" onClick={onRetry}>
          <RotateCwIcon />
          Try again
        </Button>
      </AlertDescription>
    </Alert>
  );
}

/** What a notice says when the content under it is whole, only not confirmed as current. */
export const LOADED_EARLIER = "What you see was loaded earlier.";

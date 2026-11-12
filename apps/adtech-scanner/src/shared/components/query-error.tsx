import { RotateCwIcon, TriangleAlertIcon } from "lucide-react";
import { toApiError } from "@/lib/api";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";

export function QueryError({
  title,
  error,
  onRetry,
}: {
  title: string;
  error: unknown;
  onRetry?: () => void;
}) {
  return (
    <Empty icon={TriangleAlertIcon} tone="danger" title={title} description={toApiError(error).message}>
      {onRetry ? (
        <Button variant="outline" size="sm" onClick={onRetry}>
          <RotateCwIcon />
          Try again
        </Button>
      ) : null}
    </Empty>
  );
}

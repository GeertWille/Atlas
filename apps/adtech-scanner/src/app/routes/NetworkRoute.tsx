import { Link, useNavigate } from "@tanstack/react-router";
import { ChevronRightIcon, FileQuestionIcon, LoaderIcon } from "lucide-react";
import { NetworkWorkbench } from "@/features/network";
import { toApiError } from "@/lib/api";
import { useScan } from "@/lib/query/hooks";
import { Page } from "@/shared/components/page";
import { QueryError } from "@/shared/components/query-error";
import { isActive } from "@/shared/domain/vocabulary";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { Skeleton } from "@/shared/ui/skeleton";
import { networkRoute } from "../router";

export function NetworkRoute() {
  const { scanId } = networkRoute.useParams();
  const search = networkRoute.useSearch();
  const navigate = useNavigate({ from: networkRoute.fullPath });
  const scan = useScan(scanId);
  const hostname = scan.data?.summary.hostname;

  let body: React.ReactNode;
  if (scan.isPending) {
    body = (
      <div className="flex h-full flex-col gap-3 p-4">
        <Skeleton className="h-16" />
        <Skeleton className="min-h-0 flex-1 rounded-lg" />
      </div>
    );
  } else if (scan.isError) {
    body =
      toApiError(scan.error).code === "notFound" ? (
        <Empty
          icon={FileQuestionIcon}
          title="This scan no longer exists"
          description="It may have been deleted from history."
        >
          <Button asChild variant="outline" size="sm">
            <Link to="/history">Go to history</Link>
          </Button>
        </Empty>
      ) : (
        <QueryError title="Could not load this scan" error={scan.error} onRetry={() => void scan.refetch()} />
      );
  } else if (isActive(scan.data.summary.status)) {
    body = (
      <Empty
        icon={LoaderIcon}
        title="This scan is still running"
        description="Its requests can be read once the scan has finished."
      >
        <Button asChild variant="outline" size="sm">
          <Link to="/scans/$scanId" params={{ scanId }}>
            Watch the scan
          </Link>
        </Button>
      </Empty>
    );
  } else {
    body = (
      <NetworkWorkbench
        key={scanId}
        detail={scan.data}
        search={search}
        onSearchChange={(next) => navigate({ search: next, replace: true })}
      />
    );
  }

  return (
    <Page
      title="Network"
      documentTitle={hostname ? `Network · ${hostname}` : "Network"}
      scroll={false}
      breadcrumb={
        <>
          <Link to="/history" className="rounded-sm text-muted-foreground hover:text-foreground no-drag">
            Scans
          </Link>
          <ChevronRightIcon aria-hidden="true" className="size-3.5 shrink-0 text-subtle-foreground" />
          {hostname ? (
            <>
              <Link
                to="/scans/$scanId"
                params={{ scanId }}
                search={{ tab: "network", page: search.page }}
                className="max-w-56 truncate rounded-sm text-muted-foreground hover:text-foreground no-drag"
              >
                {hostname}
              </Link>
              <ChevronRightIcon aria-hidden="true" className="size-3.5 shrink-0 text-subtle-foreground" />
            </>
          ) : null}
        </>
      }
    >
      {body}
    </Page>
  );
}

import { Link, useNavigate } from "@tanstack/react-router";
import { ChevronRightIcon, FileQuestionIcon, LoaderIcon } from "lucide-react";
import { SourceViewer } from "@/features/source-viewer";
import { toApiError } from "@/lib/api";
import { useScan } from "@/lib/query/hooks";
import { Page } from "@/shared/components/page";
import { QueryError } from "@/shared/components/query-error";
import { isActive } from "@/shared/domain/vocabulary";
import { fileNameOf } from "@/shared/lib/url";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { Skeleton } from "@/shared/ui/skeleton";
import { sourceRoute } from "../router";

function Crumb({ children }: { children: React.ReactNode }) {
  return (
    <>
      {children}
      <ChevronRightIcon aria-hidden="true" className="size-3.5 shrink-0 text-subtle-foreground" />
    </>
  );
}

export function SourceRoute() {
  const { scanId, scriptId } = sourceRoute.useParams();
  const search = sourceRoute.useSearch();
  const navigate = useNavigate({ from: sourceRoute.fullPath });
  const scan = useScan(scanId);
  const detail = scan.data;
  const script = detail?.scripts.find((candidate) => candidate.id === scriptId);
  const hostname = detail?.summary.hostname;
  const fileName = script ? fileNameOf(script.url) : "Source";

  let body: React.ReactNode;
  if (scan.isPending) {
    body = (
      <div className="flex h-full flex-col gap-3 p-4">
        <Skeleton className="h-6 w-96" />
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
        description="Its scripts can be read once the scan has finished."
      >
        <Button asChild variant="outline" size="sm">
          <Link to="/scans/$scanId" params={{ scanId }}>
            Watch the scan
          </Link>
        </Button>
      </Empty>
    );
  } else if (!script) {
    body = (
      <Empty
        icon={FileQuestionIcon}
        title="This scan has no such script"
        description="The link names a script that is not part of this scan."
      >
        <Button asChild variant="outline" size="sm">
          <Link to="/scans/$scanId" params={{ scanId }} search={{ tab: "scripts" }}>
            All scripts of this scan
          </Link>
        </Button>
      </Empty>
    );
  } else {
    body = (
      <SourceViewer
        // Another file starts from its own state: no carried-over selection, lenses or find.
        key={script.id}
        detail={scan.data}
        script={script}
        search={search}
        onSearchChange={(patch) => navigate({ search: (current) => ({ ...current, ...patch }), replace: true })}
        onOpenScript={(next) => navigate({ to: sourceRoute.fullPath, params: { scanId, scriptId: next } })}
        onOpenFinding={(finding) =>
          navigate({ to: "/scans/$scanId", params: { scanId }, search: { tab: "findings", finding } })
        }
      />
    );
  }

  return (
    <Page
      title={fileName}
      documentTitle={hostname ? `${fileName} · ${hostname}` : fileName}
      scroll={false}
      breadcrumb={
        <>
          <Crumb>
            <Link to="/history" className="rounded-sm text-muted-foreground hover:text-foreground no-drag">
              Scans
            </Link>
          </Crumb>
          {hostname ? (
            <Crumb>
              <Link
                to="/scans/$scanId"
                params={{ scanId }}
                search={{ tab: "scripts" }}
                className="max-w-56 truncate rounded-sm text-muted-foreground hover:text-foreground no-drag"
              >
                {hostname}
              </Link>
            </Crumb>
          ) : null}
        </>
      }
    >
      {body}
    </Page>
  );
}

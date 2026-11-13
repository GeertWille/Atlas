import { Link, useNavigate } from "@tanstack/react-router";
import { ChevronRightIcon, FileQuestionIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { ScanDetail } from "@/bindings";
import { AuditTrail } from "@/features/audit-trail";
import { ActivityFeed, LiveScanView } from "@/features/live-scan";
import { NetworkDigest } from "@/features/network/NetworkDigest";
import {
  isMultiPage,
  PageSwitcher,
  pageShowingScript,
  ReportHeader,
  type ReportLinks,
  ReportLinksProvider,
  ReportOverview,
  ScanOutcomeNotice,
  scopeReport,
} from "@/features/report";
import { AuctionsTab } from "@/features/report/auctions";
import { FindingsTab } from "@/features/report/findings";
import { ScriptsTab } from "@/features/report/scripts";
import { SlotsTab } from "@/features/report/slots";
import { toApiError } from "@/lib/api";
import { useScan, useScanLogs, useSettings } from "@/lib/query/hooks";
import { Page, PageContent } from "@/shared/components/page";
import { QueryError } from "@/shared/components/query-error";
import { LOADED_EARLIER, RetryNotice } from "@/shared/components/retry-notice";
import { isActive } from "@/shared/domain/vocabulary";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { Skeleton } from "@/shared/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { REPORT_TABS, type ReportTab, scanRoute } from "../router";

const TAB_LABEL: Record<ReportTab, string> = {
  findings: "Findings",
  auctions: "Auctions",
  slots: "Ad slots",
  scripts: "Scripts",
  network: "Network",
  audit: "Audit trail",
};

function TabCount({ value }: { value: number }) {
  return (
    <span className="rounded-full bg-muted px-1.5 text-[11px] leading-4 font-medium text-muted-foreground tabular-nums">
      {value}
    </span>
  );
}

function Report({ detail, notice }: { detail: ScanDetail; notice: ReactNode }) {
  const navigate = useNavigate({ from: scanRoute.fullPath });
  const search = scanRoute.useSearch();
  const logs = useScanLogs(detail.summary.id);
  const completed = detail.summary.status === "completed";
  // A scan that ended early has no report body; its audit trail is the useful part.
  const tabs: readonly ReportTab[] = completed ? REPORT_TABS : ["findings", "audit"];
  const requested = search.tab ?? (completed ? "findings" : "audit");
  const tab = tabs.includes(requested) ? requested : "audit";
  const scope = scopeReport(detail, search.page);
  // The page selection travels with every link inside the report.
  const page = scope.selected?.index;
  const counts: Partial<Record<ReportTab, number>> = {
    findings: scope.findings.length,
    auctions: scope.auctions.length,
    slots: scope.slots.length,
    scripts: scope.scripts.length,
    // Only a scan with a request list has a number to show.
    network: detail.network?.requestCount || undefined,
  };
  const scanId = detail.summary.id;
  // A scan without a request list has nothing for its records to link to on the network side.
  const hasRequests = (detail.network?.requestCount ?? 0) > 0;
  const links: ReportLinks = {
    openSource: ({ scriptId, line, to, finding }) =>
      navigate({ to: "/scans/$scanId/source/$scriptId", params: { scanId, scriptId }, search: { line, to, finding } }),
    openRequest: hasRequests
      ? (requestId, part) =>
          navigate({ to: "/scans/$scanId/network", params: { scanId }, search: { req: requestId, dt: part, page } })
      : undefined,
    openExchanges: hasRequests
      ? (auction) =>
          navigate({ to: "/scans/$scanId/network", params: { scanId }, search: { view: "bids", auction, page } })
      : undefined,
  };

  return (
    <ReportLinksProvider value={links}>
      <PageContent className="animate-enter">
        {notice}
        <ReportHeader detail={detail} />
        {completed ? (
          <ReportOverview detail={detail} logs={logs.data} />
        ) : (
          <ScanOutcomeNotice summary={detail.summary} />
        )}
        {completed ? (
          <PageSwitcher
            pages={detail.pages}
            selected={scope.selected}
            onSelect={(next) => navigate({ search: { tab, page: next ?? undefined }, replace: true })}
          />
        ) : null}
        <Tabs
          value={tab}
          onValueChange={(next) => navigate({ search: { tab: next as ReportTab, page }, replace: true })}
          className="flex flex-col gap-4"
        >
          <TabsList>
            {tabs.map((value) => (
              <TabsTrigger key={value} value={value}>
                {TAB_LABEL[value]}
                {counts[value] !== undefined ? <TabCount value={counts[value]} /> : null}
              </TabsTrigger>
            ))}
          </TabsList>
          <TabsContent value="findings">
            <FindingsTab
              findings={scope.findings}
              pages={detail.pages}
              focusedFindingId={search.finding}
              onOpenScript={(script) =>
                navigate({ search: { tab: "scripts", script, page: pageShowingScript(scope, script) } })
              }
            />
          </TabsContent>
          <TabsContent value="auctions">
            <AuctionsTab auctions={scope.auctions} diagnostics={detail.bidderDiagnostics} pages={scope.groups} />
          </TabsContent>
          <TabsContent value="slots">
            <SlotsTab slots={scope.slots} metrics={scope.markers} pages={scope.groups} />
          </TabsContent>
          <TabsContent value="scripts">
            <ScriptsTab scripts={scope.scripts} focusedScriptUrl={search.script} showPages={isMultiPage(detail)} />
          </TabsContent>
          <TabsContent value="network">
            <NetworkDigest detail={detail} page={page} />
          </TabsContent>
          <TabsContent value="audit">
            {logs.isError ? (
              <QueryError
                title="Could not load the audit trail"
                error={logs.error}
                onRetry={() => void logs.refetch()}
              />
            ) : (
              <AuditTrail
                // A new deep link starts from an unfiltered trail, so its entry is never hidden.
                key={search.log}
                steps={detail.steps}
                logs={logs.data}
                focusedLogId={search.log}
                onOpenFinding={(finding) => navigate({ search: { tab: "findings", finding } })}
              />
            )}
          </TabsContent>
        </Tabs>
      </PageContent>
    </ReportLinksProvider>
  );
}

function LiveScan({ detail, notice }: { detail: ScanDetail; notice: ReactNode }) {
  const settings = useSettings();
  const logs = useScanLogs(detail.summary.id);
  const view = (
    <LiveScanView
      detail={detail}
      logs={logs.data}
      // A scan can carry its own cap; without one it runs under the settings'.
      budgetUsd={detail.options.budgetUsd ?? settings.data?.budgetUsd}
      activity={
        logs.isError ? (
          <QueryError title="Could not load the activity" error={logs.error} onRetry={() => void logs.refetch()} />
        ) : (
          <ActivityFeed logs={logs.data} />
        )
      }
    />
  );
  if (!notice) return view;
  return (
    <div className="flex h-full flex-col">
      <div className="mx-auto w-full max-w-[1180px] px-6 pt-5">{notice}</div>
      <div className="min-h-0 flex-1">{view}</div>
    </div>
  );
}

function ReportSkeleton() {
  return (
    <PageContent>
      <div className="flex items-center gap-4">
        <Skeleton className="size-[72px] rounded-full" />
        <div className="flex flex-col gap-2">
          <Skeleton className="h-6 w-72" />
          <Skeleton className="h-4 w-96" />
        </div>
      </div>
      <Skeleton className="h-20 rounded-xl" />
      <Skeleton className="h-40 rounded-xl" />
      <Skeleton className="h-64 rounded-xl" />
    </PageContent>
  );
}

function ScanBody({ scan }: { scan: ReturnType<typeof useScan> }) {
  if (scan.isError && toApiError(scan.error).code === "notFound") {
    return (
      <Empty
        icon={FileQuestionIcon}
        title="This scan no longer exists"
        description="It may have been deleted from history."
      >
        <Button asChild variant="outline" size="sm">
          <Link to="/history">Go to history</Link>
        </Button>
      </Empty>
    );
  }
  const retry = () => void scan.refetch();
  if (scan.data === undefined) {
    if (scan.isError) return <QueryError title="Could not load this scan" error={scan.error} onRetry={retry} />;
    return <ReportSkeleton />;
  }
  // A scan that is on screen stays there when fetching it again fails.
  const notice = scan.isError ? (
    <RetryNotice title="Could not refresh this scan" detail={LOADED_EARLIER} error={scan.error} onRetry={retry} />
  ) : null;
  if (isActive(scan.data.summary.status)) return <LiveScan detail={scan.data} notice={notice} />;
  return <Report key={scan.data.summary.id} detail={scan.data} notice={notice} />;
}

export function ScanRoute() {
  const { scanId } = scanRoute.useParams();
  const scan = useScan(scanId);
  const live = scan.data ? isActive(scan.data.summary.status) : false;
  const hostname = scan.data?.summary.hostname;

  return (
    <Page
      title={hostname ?? "Scan"}
      documentTitle={hostname ? `${hostname}${live ? " (scanning)" : ""}` : "Scan"}
      scroll={!live}
      breadcrumb={
        <>
          <Link to="/history" className="rounded-sm text-muted-foreground hover:text-foreground no-drag">
            Scans
          </Link>
          <ChevronRightIcon aria-hidden="true" className="size-3.5 text-subtle-foreground" />
        </>
      }
    >
      <ScanBody scan={scan} />
    </Page>
  );
}

import { Link, useNavigate } from "@tanstack/react-router";
import { ChevronRightIcon, FileQuestionIcon } from "lucide-react";
import type { ScanDetail } from "@/bindings";
import { AuditTrail } from "@/features/audit-trail";
import { ActivityFeed, LiveScanView } from "@/features/live-scan";
import { ReportHeader, ReportOverview, ScanOutcomeNotice } from "@/features/report";
import { AuctionsTab } from "@/features/report/auctions";
import { FindingsTab } from "@/features/report/findings";
import { ScriptsTab } from "@/features/report/scripts";
import { SlotsTab } from "@/features/report/slots";
import { toApiError } from "@/lib/api";
import { useScan, useScanLogs, useSettings } from "@/lib/query/hooks";
import { Page, PageContent } from "@/shared/components/page";
import { QueryError } from "@/shared/components/query-error";
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
  audit: "Audit trail",
};

function TabCount({ value }: { value: number }) {
  return (
    <span className="rounded-full bg-muted px-1.5 text-[11px] leading-4 font-medium text-muted-foreground tabular-nums">
      {value}
    </span>
  );
}

function Report({ detail }: { detail: ScanDetail }) {
  const navigate = useNavigate({ from: scanRoute.fullPath });
  const search = scanRoute.useSearch();
  const logs = useScanLogs(detail.summary.id);
  const completed = detail.summary.status === "completed";
  // A scan that ended early has no report body; its audit trail is the useful part.
  const tabs: readonly ReportTab[] = completed ? REPORT_TABS : ["findings", "audit"];
  const requested = search.tab ?? (completed ? "findings" : "audit");
  const tab = tabs.includes(requested) ? requested : "audit";
  const counts: Partial<Record<ReportTab, number>> = {
    findings: detail.findings.length,
    auctions: detail.auctions.length,
    slots: detail.slots.length,
    scripts: detail.scripts.length,
  };

  return (
    <PageContent className="animate-enter">
      <ReportHeader detail={detail} />
      {completed ? <ReportOverview detail={detail} /> : <ScanOutcomeNotice summary={detail.summary} />}
      <Tabs
        value={tab}
        onValueChange={(next) => navigate({ search: { tab: next as ReportTab }, replace: true })}
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
            findings={detail.findings}
            focusedFindingId={search.finding}
            onOpenScript={(script) => navigate({ search: { tab: "scripts", script } })}
          />
        </TabsContent>
        <TabsContent value="auctions">
          <AuctionsTab auctions={detail.auctions} diagnostics={detail.bidderDiagnostics} />
        </TabsContent>
        <TabsContent value="slots">
          <SlotsTab slots={detail.slots} metrics={detail.metrics} />
        </TabsContent>
        <TabsContent value="scripts">
          <ScriptsTab scripts={detail.scripts} focusedScriptUrl={search.script} />
        </TabsContent>
        <TabsContent value="audit">
          {logs.isError ? (
            <QueryError title="Could not load the audit trail" error={logs.error} onRetry={() => void logs.refetch()} />
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
  );
}

function LiveScan({ detail }: { detail: ScanDetail }) {
  const settings = useSettings();
  const logs = useScanLogs(detail.summary.id);
  return (
    <LiveScanView
      detail={detail}
      budgetUsd={settings.data?.budgetUsd}
      activity={
        logs.isError ? (
          <QueryError title="Could not load the activity" error={logs.error} onRetry={() => void logs.refetch()} />
        ) : (
          <ActivityFeed logs={logs.data} />
        )
      }
    />
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
  if (scan.isPending) return <ReportSkeleton />;
  if (scan.isError) {
    if (toApiError(scan.error).code === "notFound") {
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
    return <QueryError title="Could not load this scan" error={scan.error} onRetry={() => void scan.refetch()} />;
  }
  if (isActive(scan.data.summary.status)) return <LiveScan detail={scan.data} />;
  return <Report key={scan.data.summary.id} detail={scan.data} />;
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

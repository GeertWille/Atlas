import { useNavigate } from "@tanstack/react-router";
import { TriangleAlertIcon } from "lucide-react";
import { useState } from "react";
import { DashboardStats, RecentScans, RunningScans } from "@/features/dashboard";
import { isOnboardingDismissed, needsOnboarding, OnboardingScreen } from "@/features/onboarding";
import { ScanComposer } from "@/features/scan-composer";
import { useBrowser, useDashboardStats, useScanList, useSettings } from "@/lib/query/hooks";
import { Page, PageContent } from "@/shared/components/page";
import { QueryError } from "@/shared/components/query-error";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";

/** Same query as the sidebar, so both share one cache entry; the dashboard shows the first six. */
const RECENT_QUERY = { limit: 8 };
const RECENT_SHOWN = 6;

export function HomeRoute() {
  const navigate = useNavigate();
  const settings = useSettings();
  const browser = useBrowser();
  const stats = useDashboardStats();
  const running = useScanList({ status: "running" });
  const recent = useScanList(RECENT_QUERY);
  const [dismissed, setDismissed] = useState(isOnboardingDismissed);

  if (settings.isError) {
    return (
      <Page title="Home">
        <QueryError title="Could not load settings" error={settings.error} onRetry={() => void settings.refetch()} />
      </Page>
    );
  }
  if (!settings.data || browser.isPending) {
    return (
      <Page title="Home">
        <PageContent>
          <Skeleton className="h-[104px] rounded-xl" />
          <Skeleton className="h-[92px] rounded-xl" />
          <Skeleton className="h-64 rounded-xl" />
        </PageContent>
      </Page>
    );
  }

  if (needsOnboarding(browser.data ?? null, settings.data, dismissed)) {
    return (
      <Page title="Welcome">
        <OnboardingScreen browser={browser.data ?? null} settings={settings.data} onDone={() => setDismissed(true)} />
      </Page>
    );
  }

  return (
    <Page title="Home">
      <PageContent>
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1">
            <h2 className="text-xl font-semibold tracking-tight">Scan a page for revenue leaks</h2>
            <p className="text-[13px] text-muted-foreground">
              Captures Prebid auctions, GPT slots and every script in your local browser, then checks the wrapper logic.
            </p>
          </div>
          {browser.data === null ? (
            <Alert tone="warning">
              <TriangleAlertIcon />
              <AlertTitle>No browser found</AlertTitle>
              <AlertDescription className="flex items-center justify-between gap-4">
                Scans need a local Chrome or Chromium. Choose an executable in Settings.
                <Button variant="outline" size="sm" onClick={() => navigate({ to: "/settings" })}>
                  Open settings
                </Button>
              </AlertDescription>
            </Alert>
          ) : null}
          <ScanComposer settings={settings.data} />
        </div>
        <RunningScans scans={running.data?.items ?? []} />
        <DashboardStats stats={stats.data} />
        <RecentScans scans={recent.data?.items.filter((scan) => scan.status !== "running").slice(0, RECENT_SHOWN)} />
      </PageContent>
    </Page>
  );
}

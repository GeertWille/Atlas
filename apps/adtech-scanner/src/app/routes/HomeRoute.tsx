import { useNavigate, useSearch } from "@tanstack/react-router";
import { TriangleAlertIcon } from "lucide-react";
import { useState } from "react";
import { ConfigurationsPanel } from "@/features/configurations";
import { DashboardStats, RecentScans, RunningScans } from "@/features/dashboard";
import { isOnboardingDismissed, needsOnboarding, OnboardingScreen } from "@/features/onboarding";
import { ScanComposer } from "@/features/scan-composer";
import {
  useActiveScans,
  useBrowser,
  useDashboardStats,
  useRecentScans,
  useScanConfigurations,
  useSettings,
} from "@/lib/query/hooks";
import { Page, PageContent } from "@/shared/components/page";
import { QueryError } from "@/shared/components/query-error";
import { isActive } from "@/shared/domain/vocabulary";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";

const RECENT_SHOWN = 6;

export function HomeRoute() {
  const navigate = useNavigate();
  const settings = useSettings();
  const browser = useBrowser();
  const stats = useDashboardStats();
  const activeScans = useActiveScans();
  const recent = useRecentScans();
  const configurations = useScanConfigurations();
  const { configuration: editingId } = useSearch({ from: "/" });
  const [dismissed, setDismissed] = useState(isOnboardingDismissed);

  if (settings.isError) {
    return (
      <Page title="Home">
        <QueryError title="Could not load settings" error={settings.error} onRetry={() => void settings.refetch()} />
      </Page>
    );
  }
  // A configuration opened for editing has to be known before the composer can start from it.
  const awaitingConfiguration = editingId !== undefined && configurations.isPending;
  if (!settings.data || browser.isPending || awaitingConfiguration) {
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

  const editing = configurations.data?.find((configuration) => configuration.id === editingId);

  return (
    <Page title="Home">
      <PageContent>
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1">
            <h2 className="text-xl font-semibold tracking-tight">Scan a page for revenue leaks</h2>
            <p className="text-[13px] text-muted-foreground">
              Captures Prebid auctions, GPT slots and every script in{" "}
              {settings.data.defaultBrowser === "browserbase" ? "a Browserbase browser" : "your local browser"}, then
              checks the wrapper logic.
            </p>
          </div>
          {browser.data === null ? (
            <Alert tone={settings.data.browserbase.connected ? "neutral" : "warning"}>
              <TriangleAlertIcon />
              <AlertTitle>No browser found on this Mac</AlertTitle>
              <AlertDescription className="flex items-center justify-between gap-4">
                {settings.data.browserbase.connected
                  ? "Scans can run on Browserbase. Pages on this Mac or your network need a local Chrome or Chromium: choose an executable in Settings."
                  : "Scans need a local Chrome or Chromium: choose an executable in Settings. Or set up Browserbase there to run scans in its cloud browser."}
                <Button variant="outline" size="sm" onClick={() => navigate({ to: "/settings" })}>
                  Open settings
                </Button>
              </AlertDescription>
            </Alert>
          ) : null}
          <ScanComposer
            // Opening or closing a configuration starts the composer afresh from it.
            key={editing?.id ?? "new"}
            settings={settings.data}
            configuration={editing}
          />
        </div>
        <RunningScans scans={activeScans} />
        <ConfigurationsPanel configurations={configurations.data ?? []} settings={settings.data} />
        {stats.isError ? null : <DashboardStats stats={stats.data} />}
        {recent.isError ? (
          <QueryError title="Could not load recent scans" error={recent.error} onRetry={() => void recent.refetch()} />
        ) : (
          <RecentScans scans={recent.data?.items.filter((scan) => !isActive(scan.status)).slice(0, RECENT_SHOWN)} />
        )}
      </PageContent>
    </Page>
  );
}

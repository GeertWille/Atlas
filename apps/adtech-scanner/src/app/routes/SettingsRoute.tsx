import { SettingsScreen } from "@/features/settings";
import { useBrowser, useSettings } from "@/lib/query/hooks";
import { Page, PageContent } from "@/shared/components/page";
import { QueryError } from "@/shared/components/query-error";
import { Skeleton } from "@/shared/ui/skeleton";

export function SettingsRoute() {
  const settings = useSettings();
  const browser = useBrowser();
  return (
    <Page title="Settings">
      <PageContent className="max-w-[920px] gap-8">
        {settings.isError ? (
          <QueryError title="Could not load settings" error={settings.error} onRetry={() => void settings.refetch()} />
        ) : settings.data ? (
          <SettingsScreen settings={settings.data} browser={browser.isError ? null : browser.data} />
        ) : (
          <>
            <Skeleton className="h-72 rounded-xl" />
            <Skeleton className="h-40 rounded-xl" />
          </>
        )}
      </PageContent>
    </Page>
  );
}

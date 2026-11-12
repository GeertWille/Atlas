import { HistoryScreen } from "@/features/history";
import { useSettings } from "@/lib/query/hooks";
import { Page, PageContent } from "@/shared/components/page";
import { Skeleton } from "@/shared/ui/skeleton";

export function HistoryRoute() {
  const settings = useSettings();
  return (
    <Page title="History">
      <PageContent className="gap-3">
        {settings.data ? <HistoryScreen settings={settings.data} /> : <Skeleton className="h-96 rounded-xl" />}
      </PageContent>
    </Page>
  );
}

import { useNavigate } from "@tanstack/react-router";
import { ConfigurationsScreen } from "@/features/configurations";
import { requestComposerFocus } from "@/features/scan-composer";
import { useScanConfigurations, useSettings } from "@/lib/query/hooks";
import { Page, PageContent } from "@/shared/components/page";
import { QueryError } from "@/shared/components/query-error";
import { Skeleton } from "@/shared/ui/skeleton";

export function ConfigurationsRoute() {
  const settings = useSettings();
  const configurations = useScanConfigurations();
  const navigate = useNavigate();
  // A configuration is made in the composer: from the options of a scan.
  const newConfiguration = async () => {
    await navigate({ to: "/" });
    requestComposerFocus();
  };
  return (
    <Page title="Configurations">
      <PageContent className="gap-3">
        {configurations.isError ? (
          <QueryError
            title="Could not load the configurations"
            error={configurations.error}
            onRetry={() => void configurations.refetch()}
          />
        ) : settings.data && configurations.data ? (
          <ConfigurationsScreen
            configurations={configurations.data}
            settings={settings.data}
            onNew={newConfiguration}
          />
        ) : (
          <Skeleton className="h-72 rounded-xl" />
        )}
      </PageContent>
    </Page>
  );
}

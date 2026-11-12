import { useNavigate } from "@tanstack/react-router";
import { LogSearchScreen } from "@/features/log-search";
import { Page, PageContent } from "@/shared/components/page";
import { logSearchRoute } from "../router";

export function LogSearchRoute() {
  const { q = "" } = logSearchRoute.useSearch();
  const navigate = useNavigate({ from: logSearchRoute.fullPath });
  return (
    <Page title="Log search" documentTitle={q ? `“${q}” in audit trails` : "Log search"}>
      <PageContent className="gap-4">
        <LogSearchScreen
          query={q}
          onQueryChange={(query) => navigate({ search: query ? { q: query } : {}, replace: true })}
        />
      </PageContent>
    </Page>
  );
}

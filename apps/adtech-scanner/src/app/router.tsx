import { createHashHistory, createRootRoute, createRoute, createRouter, Link } from "@tanstack/react-router";
import { CompassIcon } from "lucide-react";
import { z } from "zod";
import { Page } from "@/shared/components/page";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { HistoryRoute } from "./routes/HistoryRoute";
import { HomeRoute } from "./routes/HomeRoute";
import { LogSearchRoute } from "./routes/LogSearchRoute";
import { ScanRoute } from "./routes/ScanRoute";
import { SettingsRoute } from "./routes/SettingsRoute";
import { AppShell } from "./shell/AppShell";

export const REPORT_TABS = ["findings", "auctions", "slots", "scripts", "audit"] as const;
export type ReportTab = (typeof REPORT_TABS)[number];

/** Deep links into a report: a tab, plus the finding, script or log entry to reveal. */
const scanSearchSchema = z.object({
  tab: z.enum(REPORT_TABS).optional().catch(undefined),
  finding: z.string().optional().catch(undefined),
  script: z.string().optional().catch(undefined),
  log: z.string().optional().catch(undefined),
});

const logSearchSchema = z.object({ q: z.string().optional().catch(undefined) });

function NotFound() {
  return (
    <Page title="Not found">
      <Empty icon={CompassIcon} title="There is nothing here" description="This screen does not exist.">
        <Button asChild variant="outline" size="sm">
          <Link to="/">Go home</Link>
        </Button>
      </Empty>
    </Page>
  );
}

const rootRoute = createRootRoute({ component: AppShell, notFoundComponent: NotFound });

const homeRoute = createRoute({ getParentRoute: () => rootRoute, path: "/", component: HomeRoute });

export const scanRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/scans/$scanId",
  validateSearch: scanSearchSchema,
  component: ScanRoute,
});

const historyRoute = createRoute({ getParentRoute: () => rootRoute, path: "/history", component: HistoryRoute });

export const logSearchRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/logs",
  validateSearch: logSearchSchema,
  component: LogSearchRoute,
});

const settingsRoute = createRoute({ getParentRoute: () => rootRoute, path: "/settings", component: SettingsRoute });

const routeTree = rootRoute.addChildren([homeRoute, scanRoute, historyRoute, logSearchRoute, settingsRoute]);

/** Hash history: the packaged app is served from a custom protocol with no server-side routing. */
export const router = createRouter({ routeTree, history: createHashHistory(), defaultPreload: false });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

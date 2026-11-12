import { createRootRoute, createRoute, createRouter, Link, lazyRouteComponent } from "@tanstack/react-router";
import { CompassIcon } from "lucide-react";
import { z } from "zod";
import { DETAIL_TABS, NETWORK_GROUPINGS, NETWORK_VIEWS } from "@/features/network/search";
import { Page } from "@/shared/components/page";
import { QueryError } from "@/shared/components/query-error";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { createFragmentHistory } from "./hash-history";
import { HomeRoute } from "./routes/HomeRoute";
import { AppShell } from "./shell/AppShell";

export const REPORT_TABS = ["findings", "auctions", "slots", "scripts", "network", "audit"] as const;
export type ReportTab = (typeof REPORT_TABS)[number];

/** The saved configuration the composer has open for editing. */
const homeSearchSchema = z.object({ configuration: z.string().optional().catch(undefined) });

/** Deep links into a report: a tab, the page view it is narrowed to, plus the finding, script or log entry to reveal. */
const scanSearchSchema = z.object({
  tab: z.enum(REPORT_TABS).optional().catch(undefined),
  page: z.number().int().min(0).optional().catch(undefined),
  finding: z.string().optional().catch(undefined),
  script: z.string().optional().catch(undefined),
  log: z.string().optional().catch(undefined),
});

const logSearchSchema = z.object({ q: z.string().optional().catch(undefined) });

const line = z.number().int().min(1).optional().catch(undefined);

/** A place in a script: lines of the view shown, or a position in the script as captured (`oline`, `ocol`). */
const sourceSearchSchema = z.object({
  line,
  to: line,
  view: z.enum(["formatted", "original"]).optional().catch(undefined),
  finding: z.string().optional().catch(undefined),
  oline: line,
  ocol: line,
});

/** The whole state of the network workbench: filter, grouping, sorting, selection and time window. */
const networkSearchSchema = z.object({
  q: z.string().optional().catch(undefined),
  page: z.number().int().min(0).optional().catch(undefined),
  group: z.enum(NETWORK_GROUPINGS).optional().catch(undefined),
  sort: z.string().optional().catch(undefined),
  req: z.string().optional().catch(undefined),
  dt: z.enum(DETAIL_TABS).optional().catch(undefined),
  view: z.enum(NETWORK_VIEWS).optional().catch(undefined),
  auction: z.string().optional().catch(undefined),
  from: z.number().min(0).optional().catch(undefined),
  to: z.number().min(0).optional().catch(undefined),
});

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

/** Keeps the shell usable when a screen throws while rendering, instead of leaving a blank window. */
function ScreenError({ error, reset }: { error: unknown; reset: () => void }) {
  return (
    <Page title="Something went wrong">
      <QueryError title="This screen could not be shown" error={error} onRetry={reset} />
    </Page>
  );
}

const rootRoute = createRootRoute({ component: AppShell, notFoundComponent: NotFound });

const homeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  validateSearch: homeSearchSchema,
  component: HomeRoute,
});

export const scanRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/scans/$scanId",
  validateSearch: scanSearchSchema,
  component: lazyRouteComponent(() => import("./routes/ScanRoute"), "ScanRoute"),
});

export const sourceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/scans/$scanId/source/$scriptId",
  validateSearch: sourceSearchSchema,
  component: lazyRouteComponent(() => import("./routes/SourceRoute"), "SourceRoute"),
});

export const networkRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/scans/$scanId/network",
  validateSearch: networkSearchSchema,
  component: lazyRouteComponent(() => import("./routes/NetworkRoute"), "NetworkRoute"),
});

const configurationsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/configurations",
  component: lazyRouteComponent(() => import("./routes/ConfigurationsRoute"), "ConfigurationsRoute"),
});

const historyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/history",
  component: lazyRouteComponent(() => import("./routes/HistoryRoute"), "HistoryRoute"),
});

export const logSearchRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/logs",
  validateSearch: logSearchSchema,
  component: lazyRouteComponent(() => import("./routes/LogSearchRoute"), "LogSearchRoute"),
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: lazyRouteComponent(() => import("./routes/SettingsRoute"), "SettingsRoute"),
});

const routeTree = rootRoute.addChildren([
  homeRoute,
  scanRoute,
  sourceRoute,
  networkRoute,
  configurationsRoute,
  historyRoute,
  logSearchRoute,
  settingsRoute,
]);

/** Hash history: the packaged app is served from a custom protocol with no server-side routing. */
export const router = createRouter({
  routeTree,
  history: createFragmentHistory(),
  defaultPreload: false,
  defaultErrorComponent: ScreenError,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { render } from "@testing-library/react";
import type { ReactNode } from "react";
import { Toaster } from "@/shared/ui/sonner";
import { TooltipProvider } from "@/shared/ui/tooltip";

/**
 * Renders `ui` at "/" inside the providers every feature expects. The other
 * routes are landing pads so navigation performed by the feature is observable.
 */
export async function renderWithApp(ui: ReactNode) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const rootRoute = createRootRoute({ component: Outlet });
  const routes = [
    createRoute({ getParentRoute: () => rootRoute, path: "/", component: () => ui }),
    createRoute({ getParentRoute: () => rootRoute, path: "/settings", component: () => <p>settings screen</p> }),
    createRoute({ getParentRoute: () => rootRoute, path: "/history", component: () => <p>history screen</p> }),
    createRoute({
      getParentRoute: () => rootRoute,
      path: "/configurations",
      component: () => <p>configurations screen</p>,
    }),
    createRoute({
      getParentRoute: () => rootRoute,
      path: "/scans/$scanId",
      component: () => <p>scan screen</p>,
    }),
  ];
  const router = createRouter({
    routeTree: rootRoute.addChildren(routes),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  await router.load();
  const view = render(
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={0}>
        {/* The app router is registered globally for typing; this one has the same paths. */}
        <RouterProvider router={router as never} />
        <Toaster theme="light" />
      </TooltipProvider>
    </QueryClientProvider>,
  );
  return { ...view, router, queryClient };
}

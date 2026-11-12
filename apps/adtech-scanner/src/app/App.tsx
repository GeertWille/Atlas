import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { useEffect } from "react";
import { api } from "@/lib/api";
import { createQueryClient } from "@/lib/query/client";
import { applyScanEvent } from "@/lib/query/scan-events";
import { TooltipProvider } from "@/shared/ui/tooltip";
import { router } from "./router";
import { announceScanOutcome } from "./scan-notifications";

const queryClient = createQueryClient();

function isViewingScan(scanId: string): boolean {
  return router.state.location.pathname === `/scans/${scanId}`;
}

function openScan(scanId: string) {
  void router.navigate({ to: "/scans/$scanId", params: { scanId } });
}

/** The app's single live subscription: every screen reacts through the query cache. */
function useScanEventSubscription() {
  useEffect(() => {
    const subscription = api.subscribeScanEvents((event) => {
      applyScanEvent(queryClient, event);
      announceScanOutcome(event, isViewingScan, openScan);
    });
    return () => {
      void subscription.then((unsubscribe) => unsubscribe());
    };
  }, []);
}

export function App() {
  useScanEventSubscription();
  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={250} skipDelayDuration={150}>
        <RouterProvider router={router} />
      </TooltipProvider>
    </QueryClientProvider>
  );
}

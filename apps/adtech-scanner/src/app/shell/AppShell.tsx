import { Outlet, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import type { ThemePreference } from "@/bindings";
import { SCAN_URL_INPUT_ID } from "@/features/scan-composer";
import { useDemoUrl, useScanList, useSettings, useUpdateSettings } from "@/lib/query/hooks";
import { defaultScanOptions } from "@/shared/domain/scan-options";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { notifyError } from "@/shared/lib/notify-error";
import { Toaster } from "@/shared/ui/sonner";
import { useAppliedTheme } from "../theme";
import { CommandPalette, type PaletteActions } from "./CommandPalette";
import { Sidebar } from "./Sidebar";
import { useGlobalShortcuts } from "./shortcuts";

const RECENT_LIMIT = 8;

export function AppShell() {
  const navigate = useNavigate();
  const settings = useSettings();
  const updateSettings = useUpdateSettings();
  const demoUrl = useDemoUrl();
  const { launch } = useLaunchScan();
  const running = useScanList({ status: "running" });
  const recent = useScanList({ limit: RECENT_LIMIT });
  const [paletteOpen, setPaletteOpen] = useState(false);
  const theme = useAppliedTheme(settings.data?.theme);

  const newScan = async () => {
    await navigate({ to: "/" });
    requestAnimationFrame(() => document.getElementById(SCAN_URL_INPUT_ID)?.focus());
  };

  const actions: PaletteActions = {
    newScan,
    scanDemo: () => {
      if (settings.data && demoUrl.data) launch(defaultScanOptions(settings.data, demoUrl.data, "desktop"));
    },
    goTo: (to) => navigate({ to }),
    openScan: (scanId) => navigate({ to: "/scans/$scanId", params: { scanId } }),
    searchLogs: (q) => navigate({ to: "/logs", search: { q } }),
    setTheme: (preference: ThemePreference) =>
      updateSettings.mutate({ theme: preference }, { onError: (reason) => notifyError("Could not change the theme", reason) }),
  };

  useGlobalShortcuts({
    onCommandPalette: () => setPaletteOpen((open) => !open),
    onNewScan: newScan,
    onSettings: () => navigate({ to: "/settings" }),
  });

  const runningScans = running.data?.items ?? [];
  const recentScans = (recent.data?.items ?? []).filter((scan) => scan.status !== "running");

  return (
    <div className="flex h-full">
      <Sidebar
        runningScans={runningScans}
        recentScans={recentScans}
        onNewScan={newScan}
        onOpenPalette={() => setPaletteOpen(true)}
      />
      <div className="min-w-0 flex-1 bg-background">
        <Outlet />
      </div>
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} recentScans={recentScans} actions={actions} />
      <Toaster theme={theme} />
    </div>
  );
}

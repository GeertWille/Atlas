import { Outlet, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import type { ThemePreference } from "@/bindings";
import { requestComposerFocus } from "@/features/scan-composer";
import { useActiveScans, useDemoUrl, useRecentScans, useSettings, useUpdateSettings } from "@/lib/query/hooks";
import { defaultScanOptions } from "@/shared/domain/scan-options";
import { isActive } from "@/shared/domain/vocabulary";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { notifyError } from "@/shared/lib/notify-error";
import { Toaster } from "@/shared/ui/sonner";
import { useAppliedTheme } from "../theme";
import { CommandPalette, type PaletteActions } from "./CommandPalette";
import { Sidebar } from "./Sidebar";
import { useGlobalShortcuts } from "./shortcuts";

export function AppShell() {
  const navigate = useNavigate();
  const settings = useSettings();
  const updateSettings = useUpdateSettings();
  const demoUrl = useDemoUrl();
  const { launch } = useLaunchScan();
  const activeScans = useActiveScans();
  const recent = useRecentScans();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const theme = useAppliedTheme(settings.data?.theme);

  const newScan = async () => {
    await navigate({ to: "/" });
    requestComposerFocus();
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
      updateSettings.mutate(
        { theme: preference },
        { onError: (reason) => notifyError("Could not change the theme", reason) },
      ),
  };

  useGlobalShortcuts({
    onCommandPalette: () => setPaletteOpen((open) => !open),
    onNewScan: newScan,
    onSettings: () => navigate({ to: "/settings" }),
  });

  const recentScans = (recent.data?.items ?? []).filter((scan) => !isActive(scan.status));

  return (
    <div className="flex h-full">
      <Sidebar
        activeScans={activeScans}
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

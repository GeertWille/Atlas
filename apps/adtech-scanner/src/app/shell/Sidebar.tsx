import { Link } from "@tanstack/react-router";
import {
  HistoryIcon,
  HomeIcon,
  type LucideIcon,
  PlusIcon,
  ScrollTextIcon,
  SearchIcon,
  SettingsIcon,
} from "lucide-react";
import type { ScanSummary } from "@/bindings";
import { HealthRing } from "@/shared/components/health-ring";
import { DeviceIcon } from "@/shared/components/status";
import { DEVICE_LABEL, SCAN_STATUS_META } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Kbd } from "@/shared/ui/kbd";
import { IS_MAC, MOD_KEY } from "./shortcuts";

const NAV_ITEM =
  "flex h-8 items-center gap-2.5 rounded-md px-2 text-[13px] font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-foreground data-[status=active]:bg-sidebar-accent data-[status=active]:text-foreground";

function NavLink({
  to,
  icon: Icon,
  label,
  exact = false,
}: {
  to: "/" | "/history" | "/logs" | "/settings";
  icon: LucideIcon;
  label: string;
  exact?: boolean;
}) {
  return (
    <Link to={to} activeOptions={{ exact }} className={NAV_ITEM}>
      <Icon className="size-4" />
      {label}
    </Link>
  );
}

function ScanLink({ scan }: { scan: ScanSummary }) {
  const running = scan.status === "running" || scan.status === "queued";
  return (
    <li>
      <Link to="/scans/$scanId" params={{ scanId: scan.id }} className={cn(NAV_ITEM, "font-normal")}>
        {running ? (
          <span className="flex size-4 items-center justify-center" role="img" aria-label="Running">
            <span className="size-2 animate-pulse-dot rounded-full bg-violet" />
          </span>
        ) : (
          <HealthRing score={scan.healthScore} size={16} className="[&>span]:hidden" />
        )}
        <span className="min-w-0 flex-1 truncate">{scan.hostname.replace(/^www\./, "")}</span>
        <span className="sr-only">
          {DEVICE_LABEL[scan.device]}, {SCAN_STATUS_META[scan.status].label}
        </span>
        <DeviceIcon device={scan.device} className="size-3.5 shrink-0 text-subtle-foreground" />
      </Link>
    </li>
  );
}

function ScanGroup({ title, scans }: { title: string; scans: ScanSummary[] }) {
  if (scans.length === 0) return null;
  return (
    <section aria-label={title} className="flex flex-col gap-0.5">
      <h2 className="px-2 pb-1 text-[11px] font-medium tracking-wide text-subtle-foreground uppercase">{title}</h2>
      <ul className="flex flex-col gap-0.5">
        {scans.map((scan) => (
          <ScanLink key={scan.id} scan={scan} />
        ))}
      </ul>
    </section>
  );
}

export function Sidebar({
  runningScans,
  recentScans,
  onNewScan,
  onOpenPalette,
}: {
  runningScans: ScanSummary[];
  recentScans: ScanSummary[];
  onNewScan: () => void;
  onOpenPalette: () => void;
}) {
  return (
    <aside className="flex w-60 shrink-0 flex-col border-r bg-sidebar">
      {/* Title strip: draggable, with room for the macOS traffic lights on the left. */}
      <div
        data-tauri-drag-region
        className={cn("drag-region flex h-12 shrink-0 items-center px-3", IS_MAC && "justify-end pl-[76px]")}
      >
        <span data-tauri-drag-region className="flex items-center gap-2 text-[13px] font-semibold tracking-tight">
          <span className="flex size-5 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <svg viewBox="0 0 16 16" className="size-3" aria-hidden="true">
              <path
                d="M3 12V9m3.3 3V5m3.4 7V7.5M13 12V3.5"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                fill="none"
              />
            </svg>
          </span>
          Ad-Tech Scanner
        </span>
      </div>

      <div className="flex flex-col gap-1.5 px-3 pb-3">
        <Button onClick={onNewScan} className="justify-between">
          <span className="flex items-center gap-1.5">
            <PlusIcon />
            New scan
          </span>
          <span className="text-xs font-normal">{MOD_KEY}N</span>
        </Button>
        <button
          type="button"
          onClick={onOpenPalette}
          className="flex h-8 items-center gap-2 rounded-md border bg-background px-2 text-[13px] text-muted-foreground shadow-xs transition-colors hover:text-foreground"
        >
          <SearchIcon className="size-4" />
          <span className="flex-1 text-left">Search</span>
          <Kbd>{MOD_KEY}K</Kbd>
        </button>
      </div>

      <nav aria-label="Main" className="flex flex-col gap-0.5 px-3">
        <NavLink to="/" icon={HomeIcon} label="Home" exact />
        <NavLink to="/history" icon={HistoryIcon} label="History" />
        <NavLink to="/logs" icon={ScrollTextIcon} label="Log search" />
      </nav>

      <div className="mt-5 flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto px-3 pb-3">
        <ScanGroup title="Running" scans={runningScans} />
        <ScanGroup title="Recent" scans={recentScans} />
      </div>

      <nav aria-label="Preferences" className="border-t p-3">
        <NavLink to="/settings" icon={SettingsIcon} label="Settings" />
      </nav>
    </aside>
  );
}

import { HistoryIcon, SearchXIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import type { DeviceProfile, ScanStatus, ScanSummary, Settings } from "@/bindings";
import { useDeleteAllScans, useDeleteScan, useScanHistory } from "@/lib/query/hooks";
import { Panel } from "@/shared/components/panel";
import { QueryError } from "@/shared/components/query-error";
import { SearchInput } from "@/shared/components/search-input";
import { defaultScanOptions } from "@/shared/domain/scan-options";
import { SCAN_STATUS_META } from "@/shared/domain/vocabulary";
import { useDebouncedValue } from "@/shared/hooks/use-debounced-value";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { pluralize } from "@/shared/lib/format";
import { notifyError } from "@/shared/lib/notify-error";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { HistoryTable, HistoryTableSkeleton } from "./HistoryTable";
import { HISTORY_SORT_LABEL, type HistorySort, sortScans } from "./history-sort";

const STATUS_FILTERS: readonly ScanStatus[] = ["completed", "running", "failed", "cancelled"];

type Confirmation = { kind: "one"; scan: ScanSummary } | { kind: "all" } | null;

export function HistoryScreen({ settings }: { settings: Settings }) {
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<ScanStatus | "all">("all");
  const [device, setDevice] = useState<DeviceProfile | "all">("all");
  const [sort, setSort] = useState<HistorySort>("newest");
  const [confirmation, setConfirmation] = useState<Confirmation>(null);
  const debouncedSearch = useDebouncedValue(search.trim(), 200);

  const history = useScanHistory({
    search: debouncedSearch || undefined,
    status: status === "all" ? undefined : status,
    device: device === "all" ? undefined : device,
  });
  const deleteScan = useDeleteScan();
  const deleteAll = useDeleteAllScans();
  const { launch } = useLaunchScan();

  const loaded = history.data?.pages.flatMap((page) => page.items) ?? [];
  const total = history.data?.pages[0]?.total ?? 0;
  const scans = sortScans(loaded, sort);
  const filtering = debouncedSearch !== "" || status !== "all" || device !== "all";
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = history;

  const sentinel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const element = sentinel.current;
    if (!element || !hasNextPage) return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting) && !isFetchingNextPage) void fetchNextPage();
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  const confirmDelete = () => {
    if (confirmation?.kind === "one") {
      const { scan } = confirmation;
      deleteScan.mutate(scan.id, {
        onSuccess: () => toast.success(`Deleted the scan of ${scan.hostname}`),
        onError: (reason) => notifyError("Could not delete the scan", reason),
      });
    } else if (confirmation?.kind === "all") {
      deleteAll.mutate(undefined, {
        onSuccess: () => toast.success("All scans deleted"),
        onError: (reason) => notifyError("Could not delete the scans", reason),
      });
    }
    setConfirmation(null);
  };

  const clearFilters = () => {
    setSearch("");
    setStatus("all");
    setDevice("all");
  };

  return (
    <>
      <div className="flex items-center gap-2">
        <SearchInput
          aria-label="Search scans"
          placeholder="Search by URL"
          value={search}
          onValueChange={setSearch}
          className="w-72"
        />
        <Select value={status} onValueChange={(value) => setStatus(value as ScanStatus | "all")}>
          <SelectTrigger aria-label="Status" className="w-36">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem value="all">All statuses</SelectItem>
              {STATUS_FILTERS.map((value) => (
                <SelectItem key={value} value={value}>
                  {SCAN_STATUS_META[value].label}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
        <ToggleGroup<DeviceProfile | "all"> aria-label="Device" value={device} onValueChange={setDevice}>
          <ToggleGroupItem value="all">All</ToggleGroupItem>
          <ToggleGroupItem value="desktop">Desktop</ToggleGroupItem>
          <ToggleGroupItem value="mobile">Mobile</ToggleGroupItem>
        </ToggleGroup>
        <div className="ml-auto">
          <Select value={sort} onValueChange={(value) => setSort(value as HistorySort)}>
            <SelectTrigger aria-label="Sort" className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent align="end">
              <SelectGroup>
                {(Object.keys(HISTORY_SORT_LABEL) as HistorySort[]).map((value) => (
                  <SelectItem key={value} value={value}>
                    {HISTORY_SORT_LABEL[value]}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        </div>
      </div>

      <Panel aria-label="Scans" className="overflow-hidden">
        {history.isPending ? (
          <HistoryTableSkeleton />
        ) : history.isError ? (
          <QueryError
            title="Could not load scan history"
            error={history.error}
            onRetry={() => void history.refetch()}
          />
        ) : scans.length === 0 ? (
          filtering ? (
            <Empty icon={SearchXIcon} title="No scans match" description="Try a different search, status or device.">
              <Button variant="outline" size="sm" onClick={clearFilters}>
                Clear filters
              </Button>
            </Empty>
          ) : (
            <Empty
              icon={HistoryIcon}
              title="No scans yet"
              description="Scans you run are kept here, on this computer only."
            />
          )
        ) : (
          <HistoryTable
            scans={scans}
            onRescan={(scan) => launch(defaultScanOptions(settings, scan.url, scan.device))}
            onDelete={(scan) => setConfirmation({ kind: "one", scan })}
          />
        )}
      </Panel>

      {scans.length > 0 ? (
        <div className="flex items-center justify-between text-[13px] text-muted-foreground">
          <span className="tabular-nums" aria-live="polite">
            Showing {scans.length} of {pluralize(total, "scan")}
            {sort !== "newest" && sort !== "oldest" && hasNextPage ? ". Sorting applies to the loaded scans." : ""}
          </span>
          {hasNextPage ? (
            <Button variant="outline" size="sm" disabled={isFetchingNextPage} onClick={() => void fetchNextPage()}>
              {isFetchingNextPage ? "Loading…" : "Load more"}
            </Button>
          ) : null}
        </div>
      ) : null}
      <div ref={sentinel} aria-hidden="true" />

      {total > 0 && !filtering ? (
        <section
          aria-label="Danger zone"
          className="mt-4 flex items-center justify-between gap-6 rounded-xl border border-red-line px-4 py-3.5"
        >
          <div className="flex flex-col gap-0.5">
            <h2 className="text-sm font-semibold">Delete all scans</h2>
            <p className="text-[13px] text-muted-foreground">
              Removes every report and audit trail from this computer. This cannot be undone.
            </p>
          </div>
          <Button variant="destructive-outline" onClick={() => setConfirmation({ kind: "all" })}>
            Delete all…
          </Button>
        </section>
      ) : null}

      <AlertDialog open={confirmation !== null} onOpenChange={(open) => !open && setConfirmation(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {confirmation?.kind === "all"
                ? `Delete all ${total} scans?`
                : `Delete the scan of ${confirmation?.kind === "one" ? confirmation.scan.hostname : ""}?`}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirmation?.kind === "all"
                ? "Every report, finding and audit trail is removed from this computer. This cannot be undone."
                : "Its report, findings and audit trail are removed from this computer. This cannot be undone."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction destructive onClick={confirmDelete}>
              {confirmation?.kind === "all" ? "Delete all" : "Delete"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

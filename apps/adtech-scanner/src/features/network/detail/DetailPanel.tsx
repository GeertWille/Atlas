import { ClipboardCopyIcon, Maximize2Icon, Minimize2Icon, PanelBottomIcon, PanelRightIcon, XIcon } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import type { BodySide, NetworkRow, ScanDetail } from "@/bindings";
import { readNetworkBody, useNetworkRequest } from "@/lib/query/hooks";
import { QueryError } from "@/shared/components/query-error";
import { useCopy } from "@/shared/hooks/use-copy";
import { cn } from "@/shared/lib/cn";
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Skeleton } from "@/shared/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { Hint } from "@/shared/ui/tooltip";
import { DETAIL_TABS, type DetailTab } from "../search";
import { carriesIdentifiers, toCurl } from "./curl";
import { HeadersTab, InitiatorTab, PayloadTab, ResponseTab, SummaryTab, TimingTab } from "./tabs";

export type DetailDock = "right" | "bottom";

const TAB_LABEL: Record<DetailTab, string> = {
  summary: "Summary",
  headers: "Headers",
  payload: "Payload",
  response: "Response",
  timing: "Timing",
  initiator: "Initiator",
};

/** The warning about visitor identifiers is shown once per run of the app, not on every copy. */
let identifierWarningAccepted = false;

function shortAddress(row: NetworkRow): string {
  try {
    const { host, pathname } = new URL(row.url);
    return `${host}${pathname}`;
  } catch {
    return row.url;
  }
}

export function DetailPanel({
  scan,
  row,
  tab,
  onTabChange,
  dock,
  onDockChange,
  maximized,
  onMaximizedChange,
  onClose,
  onOpenRequest,
}: {
  scan: ScanDetail;
  /** The list entry, so the facts show at once; headers, bodies and the ad context follow. */
  row: NetworkRow;
  tab: DetailTab;
  onTabChange: (tab: DetailTab) => void;
  dock: DetailDock;
  onDockChange: (dock: DetailDock) => void;
  maximized: boolean;
  onMaximizedChange: (maximized: boolean) => void;
  onClose: () => void;
  onOpenRequest: (requestId: string) => void;
}) {
  const scanId = scan.summary.id;
  const request = useNetworkRequest(scanId, row.id);
  const detail = request.data;
  const { copy } = useCopy();
  const [pendingCopy, setPendingCopy] = useState<(() => void) | null>(null);

  // The text is still to be read when the menu item is chosen: the copy starts with a promise of it.
  const copyBody = (side: BodySide) => {
    const body = readNetworkBody(scanId, row.id, side).then((text) => {
      if (text !== null) return text;
      toast.info(side === "request" ? "This request has no stored body" : "The response body is not stored");
      throw new Error("no stored body");
    });
    body.catch((reason) => {
      if (!(reason instanceof Error && reason.message === "no stored body"))
        notifyError("Could not read the body", reason);
    });
    void copy(body, side === "request" ? "Payload copied" : "Response copied");
  };

  const copyCurl = async () => {
    if (!detail) return;
    const hasBody = detail.requestBody.state === "kept" || detail.requestBody.state === "truncated";
    const built = (hasBody ? readNetworkBody(scanId, row.id, "request") : Promise.resolve(null)).then((body) =>
      toCurl(detail, body),
    );
    built.catch((reason) => notifyError("Could not build the command", reason));
    if (!(await copy(built.then(({ command }) => command)))) return;
    const { redactedHeaders, bodyIncomplete } = await built;
    if (redactedHeaders.length > 0) {
      toast.warning("Copied, with placeholders", {
        description: `${redactedHeaders.join(" and ")} ${redactedHeaders.length === 1 ? "was" : "were"} not stored, so the command will not reproduce a signed-in request.`,
      });
    } else if (bodyIncomplete) {
      toast.warning("Copied without the full request body", { description: "The body is not stored whole." });
    } else {
      toast.success("Copied as cURL");
    }
  };

  /** Copies of bid requests carry the visitor's consent string and ids: say so once before the first one. */
  const guarded = (action: () => void) => {
    if (!detail || identifierWarningAccepted || !carriesIdentifiers(detail)) action();
    else setPendingCopy(() => action);
  };

  return (
    <section aria-label="Request details" className="flex h-full min-h-0 min-w-0 flex-col bg-background">
      <header className="flex h-9 shrink-0 items-center gap-1 border-b pr-1 pl-3">
        <h2 className="selectable min-w-0 flex-1 truncate font-mono text-xs" title={row.url}>
          <span className="font-semibold">{row.method}</span> {shortAddress(row)}
        </h2>
        <DropdownMenu>
          <Hint label="Copy">
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-sm" aria-label="Copy">
                <ClipboardCopyIcon />
              </Button>
            </DropdownMenuTrigger>
          </Hint>
          <DropdownMenuContent align="end">
            <DropdownMenuGroup>
              <DropdownMenuItem onSelect={() => void copy(row.url, "URL copied")}>Copy URL</DropdownMenuItem>
              <DropdownMenuItem disabled={!detail?.available} onSelect={() => guarded(() => void copyCurl())}>
                Copy as cURL
              </DropdownMenuItem>
              <DropdownMenuItem disabled={!row.hasRequestBody} onSelect={() => guarded(() => copyBody("request"))}>
                Copy payload
              </DropdownMenuItem>
              <DropdownMenuItem disabled={!row.hasResponseBody} onSelect={() => copyBody("response")}>
                Copy response
              </DropdownMenuItem>
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <Hint label={dock === "right" ? "Dock below the list" : "Dock beside the list"}>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={dock === "right" ? "Dock below the list" : "Dock beside the list"}
            onClick={() => onDockChange(dock === "right" ? "bottom" : "right")}
          >
            {dock === "right" ? <PanelBottomIcon /> : <PanelRightIcon />}
          </Button>
        </Hint>
        <Hint label={maximized ? "Show the list again (Esc)" : "Hide the list"}>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={maximized ? "Show the list again" : "Hide the list"}
            aria-pressed={maximized}
            onClick={() => onMaximizedChange(!maximized)}
          >
            {maximized ? <Minimize2Icon /> : <Maximize2Icon />}
          </Button>
        </Hint>
        <Hint label="Close (Esc)">
          <Button variant="ghost" size="icon-sm" aria-label="Close request details" onClick={onClose}>
            <XIcon />
          </Button>
        </Hint>
      </header>
      <Tabs
        value={tab}
        onValueChange={(next) => onTabChange(next as DetailTab)}
        className="flex min-h-0 flex-1 flex-col"
      >
        <TabsList className="shrink-0 gap-4 overflow-x-auto px-3">
          {DETAIL_TABS.map((value) => (
            <TabsTrigger key={value} value={value} className="h-8 text-xs">
              {TAB_LABEL[value]}
            </TabsTrigger>
          ))}
        </TabsList>
        <div className="min-h-0 flex-1 overflow-y-auto">
          <TabsContent value="summary">
            <SummaryTab scan={scan} row={row} detail={detail} onOpenRequest={onOpenRequest} />
          </TabsContent>
          {DETAIL_TABS.filter((value) => value !== "summary").map((value) => (
            <TabsContent key={value} value={value} className={cn(tab === value && "flex min-h-full flex-col")}>
              {request.isError ? (
                <QueryError
                  title="Could not load this request"
                  error={request.error}
                  onRetry={() => void request.refetch()}
                />
              ) : !detail ? (
                <div className="flex flex-col gap-2 p-3">
                  <Skeleton className="h-3 w-2/3" />
                  <Skeleton className="h-3 w-1/2" />
                  <Skeleton className="h-3 w-3/5" />
                </div>
              ) : !detail.available ? (
                <p className="p-3 text-xs text-pretty text-muted-foreground">
                  Not stored any more: the network data of this scan was removed to stay under the storage limit.
                </p>
              ) : value === "headers" ? (
                <HeadersTab detail={detail} />
              ) : value === "payload" ? (
                <PayloadTab key={row.id} scanId={scanId} detail={detail} />
              ) : value === "response" ? (
                <ResponseTab key={row.id} scanId={scanId} detail={detail} />
              ) : value === "timing" ? (
                <TimingTab detail={detail} />
              ) : (
                <InitiatorTab scanId={scanId} detail={detail} />
              )}
            </TabsContent>
          ))}
        </div>
      </Tabs>
      <AlertDialog open={pendingCopy !== null} onOpenChange={(open) => !open && setPendingCopy(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>This copy identifies the visit</AlertDialogTitle>
            <AlertDialogDescription>
              A bid request carries the consent string and the user ids of the browser that made the scan, and a stored
              cookie or token is a credential. Paste it only where you would share those. Running the command sends a
              real request to the exchange in your name.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                identifierWarningAccepted = true;
                pendingCopy?.();
                setPendingCopy(null);
              }}
            >
              Copy anyway
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}

import {
  ArrowRightIcon,
  CheckIcon,
  CircleDashedIcon,
  ExternalLinkIcon,
  FolderOpenIcon,
  PlayIcon,
  TriangleAlertIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import type { BrowserInfo, Settings } from "@/bindings";
import { useDemoUrl, useUpdateSettings } from "@/lib/query/hooks";
import { ApiKeyForm } from "@/shared/components/api-key-form";
import { defaultScanOptions } from "@/shared/domain/scan-options";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { cn } from "@/shared/lib/cn";
import { notifyError } from "@/shared/lib/notify-error";
import { chooseBrowserExecutable, openApiKeysPage } from "@/shared/lib/system";
import { Button } from "@/shared/ui/button";
import { dismissOnboarding } from "./onboarding-state";

type StepState = "done" | "optional" | "required";

function Step({ state, title, children }: { state: StepState; title: string; children: ReactNode }) {
  const Icon = state === "done" ? CheckIcon : state === "required" ? TriangleAlertIcon : CircleDashedIcon;
  return (
    <li className="flex gap-3.5 border-b p-4 last:border-b-0">
      <span
        className={cn(
          "mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full border",
          state === "done" && "border-green-line bg-green-soft text-green-fg",
          state === "required" && "border-amber-line bg-amber-soft text-amber-fg",
          state === "optional" && "bg-muted text-subtle-foreground",
        )}
      >
        <Icon className="size-3.5" strokeWidth={2.5} />
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <h2 className="flex items-center gap-2 text-sm font-semibold">
          {title}
          {state === "optional" ? <span className="text-xs font-normal text-subtle-foreground">Optional</span> : null}
        </h2>
        {children}
      </div>
    </li>
  );
}

function BrowserStep({ browser }: { browser: BrowserInfo | null }) {
  const updateSettings = useUpdateSettings();
  const choose = async () => {
    const path = await chooseBrowserExecutable();
    if (path) {
      updateSettings.mutate(
        { chromePath: path },
        { onError: (reason) => notifyError("Could not use that browser", reason) },
      );
    }
  };
  if (browser) {
    return (
      <Step state="done" title="Browser detected">
        <p className="text-[13px] text-muted-foreground">
          Scans run in your local <span className="font-medium text-foreground">{browser.version}</span>.
        </p>
        <p className="selectable truncate font-mono text-xs text-subtle-foreground">{browser.path}</p>
      </Step>
    );
  }
  return (
    <Step state="required" title="No Chrome or Chromium found">
      <p className="text-[13px] text-muted-foreground">
        Scans drive a local Chromium-based browser. Install Google Chrome, or point to an existing executable.
      </p>
      <div>
        <Button variant="outline" size="sm" onClick={choose}>
          <FolderOpenIcon />
          Choose browser…
        </Button>
      </div>
    </Step>
  );
}

function ApiKeyStep({ hasApiKey }: { hasApiKey: boolean }) {
  if (hasApiKey) {
    return (
      <Step state="done" title="Anthropic API key saved">
        <p className="text-[13px] text-muted-foreground">
          AI logic analysis is enabled. The key is stored in your system keychain.
        </p>
      </Step>
    );
  }
  return (
    <Step state="optional" title="Add an Anthropic API key">
      <p className="text-[13px] text-pretty text-muted-foreground">
        Scans work without a key: auctions, slots and deterministic revenue-leak checks need no AI. With a key, the
        scanner also reads your wrapper code to find logic bugs, for about $0.50 per scan.
      </p>
      <ApiKeyForm submitLabel="Save key" submitVariant="outline" />
      <div>
        <Button variant="link" size="sm" className="h-auto p-0" onClick={openApiKeysPage}>
          Get a key from the Anthropic Console
          <ExternalLinkIcon />
        </Button>
      </div>
    </Step>
  );
}

export function OnboardingScreen({
  browser,
  settings,
  onDone,
}: {
  browser: BrowserInfo | null;
  settings: Settings;
  onDone: () => void;
}) {
  const demoUrl = useDemoUrl();
  const { launch, isPending } = useLaunchScan();

  const finish = () => {
    dismissOnboarding();
    onDone();
  };

  const scanDemo = () => {
    if (!demoUrl.data) return;
    dismissOnboarding();
    launch(defaultScanOptions(settings, demoUrl.data, "desktop"));
  };

  return (
    <div className="mx-auto flex w-full max-w-[600px] animate-enter flex-col gap-6 px-6 py-12">
      <div className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight">Find the revenue your ad stack is leaking</h1>
        <p className="text-[15px] text-pretty text-muted-foreground">
          Ad-Tech Scanner loads a page in your own browser, records every Prebid auction and GPT slot, and looks for the
          logic bugs that leave ad containers empty. Everything stays on this Mac.
        </p>
      </div>

      <ol className="rounded-xl border bg-card shadow-xs">
        <BrowserStep browser={browser} />
        <ApiKeyStep hasApiKey={settings.hasApiKey} />
        <Step state="optional" title="Try it on the built-in demo site">
          <p className="text-[13px] text-pretty text-muted-foreground">
            A sample publisher page with a real wrapper bug: desktop visitors are treated as mobile and half the slots
            never bid.
          </p>
          <div>
            <Button size="sm" disabled={!browser || !demoUrl.data || isPending} onClick={scanDemo}>
              <PlayIcon />
              Scan the demo site
            </Button>
          </div>
        </Step>
      </ol>

      <div className="flex justify-end">
        <Button variant="ghost" onClick={finish}>
          {settings.hasApiKey ? "Continue" : "Continue without AI"}
          <ArrowRightIcon />
        </Button>
      </div>
    </div>
  );
}

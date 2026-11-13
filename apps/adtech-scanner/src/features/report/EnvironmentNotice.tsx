import type { ReactNode } from "react";
import type { ScanDetail, ScanOptions } from "@/bindings";
import { rescanOptions } from "@/shared/domain/scan-options";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { pluralize } from "@/shared/lib/format";
import { environmentNotice } from "./environment-notice";

function RepeatButton({
  disabled,
  onClick,
  children,
}: {
  disabled: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="rounded-sm font-medium text-foreground underline decoration-border-strong underline-offset-2 hover:decoration-current disabled:opacity-60"
    >
      {children}
    </button>
  );
}

const sentence = (text: string) => `${text.charAt(0).toUpperCase()}${text.slice(1)}`;

/** The same scan in this Mac's Chrome: what only a Browserbase browser has is left to the settings. */
function onThisMac(options: ScanOptions): ScanOptions {
  const { headless: _headless, persistentProfile: _profile, proxy, ...rest } = options;
  return {
    ...rest,
    browser: { kind: "local" },
    ...(proxy && proxy.kind !== "browserbaseLocation" ? { proxy } : {}),
  };
}

/**
 * One quiet line of the visit summary about how the visit looked to the pages. On this Mac: the pages
 * could recognise the visit as automated, what that can do to the numbers, and how to compare with a
 * visit a person drives. On Browserbase: that it ran there, and the one thing about that browser which can
 * change the result, since it gives the pages no sign of automation. Every scripted visit gets it, so it is
 * a remark, not a warning; when the backend finds a reason to suspect filtering it says so itself, in the
 * overview and as a finding.
 */
export function EnvironmentNotice({
  detail,
  serviceScripts = null,
}: {
  detail: ScanDetail;
  /** How many scripts of its own Browserbase ran in the pages, when the audit trail says. */
  serviceScripts?: number | null;
}) {
  const { launch, isPending } = useLaunchScan();
  const notice = environmentNotice(detail);
  if (!notice) return null;
  const options = rescanOptions(detail.options);
  if (notice.kind === "browserbase") {
    return (
      <p role="note" aria-label="Scan environment" className="selectable text-xs text-pretty text-muted-foreground">
        This visit ran in a Browserbase browser{notice.manual ? ", which you drove through its live view" : ""}; what
        the pages could read there is under Browser details. Browserbase ran its own scripts inside every page
        {serviceScripts === null ? "" : ` (${pluralize(serviceScripts, "script")} in this visit)`}; the scanner leaves
        them out of its results, but they can change how a page behaves.
        {notice.detectors.length > 0 ? ` ${sentence(notice.detectors.join(", "))}.` : ""} To compare, repeat the scan{" "}
        <RepeatButton disabled={isPending} onClick={() => launch(onThisMac(options))}>
          on this Mac
        </RepeatButton>
        .
      </p>
    );
  }
  return (
    <p role="note" aria-label="Scan environment" className="selectable text-xs text-pretty text-muted-foreground">
      This visit was automated and the pages could tell: {notice.signals.join(", ")}. Ad partners can bid less, or not
      at all, for automated visits. To compare, repeat the scan{" "}
      <RepeatButton
        disabled={isPending}
        onClick={() => launch({ ...options, visitor: "manual", extraPages: 0, headless: false, acceptConsent: false })}
      >
        as “I'll browse myself”
      </RepeatButton>
      {notice.canShowBrowser ? (
        <>
          {" or "}
          <RepeatButton disabled={isPending} onClick={() => launch({ ...options, headless: false })}>
            with the browser shown
          </RepeatButton>
        </>
      ) : null}
      .
    </p>
  );
}

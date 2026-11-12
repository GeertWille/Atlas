import type { ReactNode } from "react";
import type { ScanDetail } from "@/bindings";
import { rescanOptions } from "@/shared/domain/scan-options";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
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

/**
 * One quiet line of the visit summary: the pages could recognise the visit as automated, what that
 * can do to the numbers, and how to compare with a visit a person drives. Every scripted visit gets
 * it, so it is a remark, not a warning; when the backend finds a reason to suspect filtering it says
 * so itself, in the overview and as a finding.
 */
export function EnvironmentNotice({ detail }: { detail: ScanDetail }) {
  const { launch, isPending } = useLaunchScan();
  const notice = environmentNotice(detail);
  if (!notice) return null;
  const options = rescanOptions(detail.options);
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

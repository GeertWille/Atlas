import { BotIcon } from "lucide-react";
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
      className="rounded-sm font-medium underline underline-offset-2 hover:no-underline disabled:opacity-60"
    >
      {children}
    </button>
  );
}

/**
 * Says, without softening it, that the scanned pages could recognise the visit as automated, what
 * that can do to the numbers below, and how to check them with a visit a person drives.
 */
export function EnvironmentNotice({ detail }: { detail: ScanDetail }) {
  const { launch, isPending } = useLaunchScan();
  const notice = environmentNotice(detail);
  if (!notice) return null;
  const options = rescanOptions(detail.options);
  return (
    // A standing remark about the scan, not an interruption: a note, not an alert.
    <aside
      role="note"
      aria-label="Scan environment"
      className="flex items-start gap-2.5 rounded-lg border border-amber-line bg-amber-soft px-3.5 py-2.5 text-[13px] text-amber-fg"
    >
      <BotIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
      <p className="selectable text-pretty">
        <span className="font-medium">{notice.title}.</span>{" "}
        {notice.signals.length > 0 ? `${notice.signals.join(", ")}. ` : ""}
        Ad partners can bid less, or not at all, for automated visits, so low fill or missing bids in this report may
        not match your visitors'. Before concluding that a bidder is broken, repeat the scan{" "}
        <RepeatButton
          disabled={isPending}
          onClick={() =>
            launch({ ...options, visitor: "manual", extraPages: 0, headless: false, acceptConsent: false })
          }
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
    </aside>
  );
}

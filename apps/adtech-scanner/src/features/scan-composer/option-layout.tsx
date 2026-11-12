import { type ReactNode, useId } from "react";
import { cn } from "@/shared/lib/cn";
import { Label } from "@/shared/ui/label";

/** One group of options under the question it answers: the question on the left, the controls on the right. */
export function OptionsSection({
  title,
  question,
  children,
}: {
  title: string;
  question: string;
  children: ReactNode;
}) {
  const headingId = useId();
  return (
    <section
      aria-labelledby={headingId}
      className="grid grid-cols-[132px_minmax(0,1fr)] gap-x-6 border-t px-4 py-4 xl:grid-cols-[168px_minmax(0,1fr)]"
    >
      <div className="flex flex-col gap-1 pt-1.5">
        <h3 id={headingId} className="text-[13px] leading-none font-semibold">
          {title}
        </h3>
        <p className="text-xs text-pretty text-muted-foreground">{question}</p>
      </div>
      <div className="flex min-w-0 flex-col gap-4">{children}</div>
    </section>
  );
}

export function OptionHelp({
  className,
  ...props
}: {
  id?: string;
  role?: "alert";
  className?: string;
  children: ReactNode;
}) {
  return <p className={cn("text-xs text-pretty text-muted-foreground", className)} {...props} />;
}

/** One option: its name with the control (or its fixed value) on the same line, and what it means below. */
export function OptionCell({
  label,
  htmlFor,
  control,
  children,
}: {
  label: string;
  /** The control the label names; without one the label is plain text beside a fixed value. */
  htmlFor?: string;
  control: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <div className="flex min-h-7 items-center justify-between gap-3">
        {htmlFor ? (
          <Label htmlFor={htmlFor}>{label}</Label>
        ) : (
          <span className="text-[13px] leading-none font-medium">{label}</span>
        )}
        {control}
      </div>
      {children}
    </div>
  );
}

/** What an option is fixed to while the chosen visit does not let it vary. */
export function FixedValue({ children }: { children: ReactNode }) {
  return <span className="text-[13px] whitespace-nowrap text-subtle-foreground">{children}</span>;
}

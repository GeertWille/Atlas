import { Fragment } from "react";
import { cn } from "@/shared/lib/cn";
import { SettingsLink } from "./SettingsLink";
import type { SummaryPart } from "./scan-summary";

/** One line that says what pressing Scan will do. `id` lets the Scan button point at it as its description. */
export function ComposerSummary({ id, parts, needsKey }: { id: string; parts: SummaryPart[]; needsKey: boolean }) {
  return (
    <div className="min-w-0 text-[13px] text-pretty text-muted-foreground">
      <span id={id}>
        {parts.map((part, index) => (
          <Fragment key={part.text}>
            {index > 0 ? " · " : null}
            <span className={cn(index === 0 && "font-medium text-foreground", part.warning && "text-amber-fg")}>
              {part.text}
            </span>
          </Fragment>
        ))}
      </span>
      {needsKey ? (
        <>
          {". "}
          <SettingsLink>Add a key</SettingsLink>
        </>
      ) : null}
    </div>
  );
}

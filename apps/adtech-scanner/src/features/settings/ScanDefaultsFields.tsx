import { MonitorIcon, SmartphoneIcon } from "lucide-react";
import { useId } from "react";
import type { Settings, SettingsPatch, VisitorBehavior } from "@/bindings";
import { OBSERVE_SECONDS, VISIT_PAGES, VISITOR_BEHAVIORS, VISITOR_META } from "@/shared/domain/scan-options";
import { pluralize } from "@/shared/lib/format";
import { Field } from "@/shared/ui/field";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { Stepper } from "@/shared/ui/stepper";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { CommitInput } from "./CommitInput";
import { parseObserveSeconds } from "./settings-schema";

/** What the new-scan composer starts from: device, visitor, pages of a browse visit and observation time. */
export function ScanDefaultsFields({ settings, save }: { settings: Settings; save: (patch: SettingsPatch) => void }) {
  const ids = { observe: useId(), pages: useId() };
  return (
    <>
      <Field label="Device">
        <ToggleGroup
          aria-label="Default device"
          value={settings.defaultDevice}
          onValueChange={(defaultDevice) => save({ defaultDevice })}
        >
          <ToggleGroupItem value="desktop">
            <MonitorIcon />
            Desktop
          </ToggleGroupItem>
          <ToggleGroupItem value="mobile">
            <SmartphoneIcon />
            Mobile
          </ToggleGroupItem>
        </ToggleGroup>
      </Field>
      <Field
        label="Visitor"
        description={`${VISITOR_META[settings.defaultVisitor].description} Scripted visits are automated and say so to the page.`}
      >
        <Select
          value={settings.defaultVisitor}
          onValueChange={(defaultVisitor) => save({ defaultVisitor: defaultVisitor as VisitorBehavior })}
        >
          <SelectTrigger aria-label="Default visitor" className="w-48">
            <SelectValue />
          </SelectTrigger>
          <SelectContent align="end">
            <SelectGroup>
              {VISITOR_BEHAVIORS.map((visitor) => (
                <SelectItem key={visitor} value={visitor}>
                  {VISITOR_META[visitor].label}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
      </Field>
      <Field
        label="Pages to visit"
        htmlFor={ids.pages}
        description="For “Browse like a visitor”: the entry page plus the pages reached by following links on the same site."
      >
        <Stepper
          id={ids.pages}
          value={settings.defaultExtraPages + 1}
          min={VISIT_PAGES.min}
          max={VISIT_PAGES.max}
          onValueChange={(pages) => save({ defaultExtraPages: pages - 1 })}
          decreaseLabel="Fewer pages"
          increaseLabel="More pages"
        >
          {pluralize(settings.defaultExtraPages + 1, "page")}
        </Stepper>
      </Field>
      <Field
        label="Observe after load"
        htmlFor={ids.observe}
        description={`Seconds to keep watching each page, ${OBSERVE_SECONDS.min} to ${OBSERVE_SECONDS.max}.`}
      >
        <CommitInput
          trailing="s"
          id={ids.observe}
          inputMode="numeric"
          className="w-24 pr-7 text-right tabular-nums"
          value={settings.defaultObserveSeconds}
          parse={parseObserveSeconds}
          onCommit={(defaultObserveSeconds) => save({ defaultObserveSeconds })}
        />
      </Field>
    </>
  );
}

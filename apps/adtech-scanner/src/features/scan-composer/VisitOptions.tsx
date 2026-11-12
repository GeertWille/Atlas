import { type ReactNode, useId } from "react";
import type { Settings, VisitorBehavior } from "@/bindings";
import type { AiAvailability } from "@/shared/domain/ai-availability";
import {
  isScripted,
  OBSERVE_SECONDS,
  VISIT_PAGES,
  VISITOR_BEHAVIORS,
  VISITOR_META,
} from "@/shared/domain/scan-options";
import { pluralize } from "@/shared/lib/format";
import { Label } from "@/shared/ui/label";
import { Slider } from "@/shared/ui/slider";
import { Stepper } from "@/shared/ui/stepper";
import { Switch } from "@/shared/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { SettingsLink } from "./SettingsLink";
import type { ScanForm } from "./scan-form";

function OptionSwitch({
  label,
  description,
  checked,
  disabled,
  onCheckedChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  const id = useId();
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="flex flex-col gap-0.5">
        <Label htmlFor={id}>{label}</Label>
        <p className="text-xs text-pretty text-muted-foreground">{description}</p>
      </div>
      <Switch id={id} checked={checked} disabled={disabled} onCheckedChange={onCheckedChange} />
    </div>
  );
}

/** What the page can tell about the visit. Scripted visits never pretend to be a person. */
function AutomationNote({ form }: { form: ScanForm }) {
  if (!isScripted(form.visitor)) {
    return (
      <p className="text-xs text-pretty text-muted-foreground">
        Chrome opens; browse the site, then press <span className="font-medium text-foreground">Finish capture</span>{" "}
        (up to 15 minutes). You drive the browser and handle the consent banner yourself. The page can still notice the
        debugger that records it.
      </p>
    );
  }
  return (
    <p className="text-xs text-pretty text-muted-foreground">
      This visit is automated and says so to the page (navigator.webdriver is true). Ad partners can treat it
      differently from a person's visit.
    </p>
  );
}

/** How the scan visits the site: who the visitor is, for how long and how far, and what runs afterwards. */
export function VisitOptions({
  form,
  settings,
  ai,
  aiEnabled,
  update,
  onVisitorChange,
  children,
}: {
  form: ScanForm;
  settings: Settings;
  ai: AiAvailability;
  /** The AI switch as it takes effect: off while no key is stored, whatever the form says. */
  aiEnabled: boolean;
  update: (patch: Partial<ScanForm>) => void;
  onVisitorChange: (visitor: VisitorBehavior) => void;
  /** What follows the options in the same grid. */
  children: ReactNode;
}) {
  const ids = { observe: useId(), pages: useId(), visitor: useId() };
  const manual = form.visitor === "manual";
  const browse = form.visitor === "browse";
  const triageOnly = ai.triage !== null && !ai.logicAnalysis;
  return (
    <div className="grid grid-cols-2 gap-x-10 gap-y-4 border-t px-4 py-4">
      <div className="col-span-2 flex flex-col gap-2">
        <Label id={ids.visitor}>Visitor</Label>
        <ToggleGroup
          aria-labelledby={ids.visitor}
          value={form.visitor}
          onValueChange={onVisitorChange}
          className="self-start"
        >
          {VISITOR_BEHAVIORS.map((visitor) => (
            <ToggleGroupItem key={visitor} value={visitor}>
              {VISITOR_META[visitor].label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <p className="text-xs text-pretty text-foreground">{VISITOR_META[form.visitor].description}</p>
        <AutomationNote form={form} />
      </div>

      {manual ? null : (
        <div className="flex flex-col gap-2">
          <div className="flex items-baseline justify-between">
            <Label htmlFor={ids.observe}>{browse ? "Time on each page" : "Observe after load"}</Label>
            <span className="text-[13px] text-muted-foreground tabular-nums">{form.observeSeconds} s</span>
          </div>
          <Slider
            id={ids.observe}
            aria-label={browse ? "Time on each page, seconds" : "Observe after load, seconds"}
            min={OBSERVE_SECONDS.min}
            max={OBSERVE_SECONDS.max}
            step={OBSERVE_SECONDS.step}
            value={[form.observeSeconds]}
            onValueChange={([observeSeconds]) => {
              if (observeSeconds !== undefined) update({ observeSeconds });
            }}
          />
          <p className="text-xs text-muted-foreground">Longer windows catch refreshes and late lazy-loaded auctions.</p>
        </div>
      )}
      {browse ? (
        <div className="flex items-start justify-between gap-4">
          <div className="flex flex-col gap-0.5">
            <Label htmlFor={ids.pages}>Pages to visit</Label>
            <p className="text-xs text-pretty text-muted-foreground">
              {form.pages === 1
                ? "The entry page only; no link is followed."
                : `The entry page, then up to ${pluralize(form.pages - 1, "link")} on the same site. Ads, sign-in and checkout links are never clicked.`}
            </p>
          </div>
          <Stepper
            id={ids.pages}
            value={form.pages}
            min={VISIT_PAGES.min}
            max={VISIT_PAGES.max}
            onValueChange={(pages) => update({ pages })}
            decreaseLabel="Fewer pages"
            increaseLabel="More pages"
          >
            {pluralize(form.pages, "page")}
          </Stepper>
        </div>
      ) : manual ? null : (
        <div />
      )}

      {manual ? null : (
        <>
          <OptionSwitch
            label="Show the browser"
            description="Watch the visit in a Chrome window. Headless Chrome is easier for ad partners to recognise as automated."
            checked={form.showBrowser}
            onCheckedChange={(showBrowser) => update({ showBrowser })}
          />
          <OptionSwitch
            label="Accept consent banner"
            description="Click through the CMP so bidding is not blocked."
            checked={form.acceptConsent}
            onCheckedChange={(acceptConsent) => update({ acceptConsent })}
          />
        </>
      )}
      <OptionSwitch
        label={triageOnly ? "AI script triage" : "AI logic analysis"}
        description={
          triageOnly
            ? "Classify scripts with Jev. Logic analysis needs an Anthropic key."
            : "Classify scripts, then hunt for wrapper logic bugs."
        }
        checked={aiEnabled}
        disabled={ai.triage === null}
        onCheckedChange={(aiEnabled) => update({ aiEnabled })}
      />
      <div className="flex flex-col gap-0.5">
        <span className="text-[13px] leading-none font-medium">Scan profile</span>
        <p className="text-xs text-pretty text-muted-foreground">
          {(form.profile === "settings" ? settings.persistentProfile : form.profile === "kept")
            ? "Kept between scans: cookies and consent persist, like a returning visitor's."
            : "A fresh profile for this scan: no cookies, like a first-time visitor."}{" "}
          {form.profile === "settings" ? <SettingsLink>Change the default</SettingsLink> : null}
        </p>
      </div>

      {children}
    </div>
  );
}

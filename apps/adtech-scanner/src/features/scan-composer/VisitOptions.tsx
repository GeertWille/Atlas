import { useId } from "react";
import type { VisitorBehavior } from "@/bindings";
import {
  isScripted,
  MANUAL_SESSION_LIMIT_MS,
  OBSERVE_SECONDS,
  VISIT_PAGES,
  VISITOR_BEHAVIORS,
  VISITOR_META,
} from "@/shared/domain/scan-options";
import { pluralize } from "@/shared/lib/format";
import { Slider } from "@/shared/ui/slider";
import { Stepper } from "@/shared/ui/stepper";
import { Switch } from "@/shared/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { FixedValue, OptionCell, OptionHelp, OptionsSection } from "./option-layout";
import type { ScanForm } from "./scan-form";

const MANUAL_LIMIT_MINUTES = MANUAL_SESSION_LIMIT_MS / 60_000;

/** What the page can tell about the visit. Scripted visits never pretend to be a person. */
function AutomationNote({ visitor, remote }: { visitor: VisitorBehavior; remote: boolean }) {
  if (!isScripted(visitor)) {
    return <OptionHelp>You drive the browser. The page can still notice the debugger that records it.</OptionHelp>;
  }
  // No longer than the note for this Mac, so that it takes no more lines.
  return (
    <OptionHelp>
      {remote
        ? "This visit is automated, but Browserbase's browser does not tell the page (navigator.webdriver is false). The report records it."
        : "This visit is automated and says so to the page (navigator.webdriver is true). Ad partners can treat it differently from a person's visit."}
    </OptionHelp>
  );
}

function ObserveTime({ form, update }: { form: ScanForm; update: (patch: Partial<ScanForm>) => void }) {
  const id = useId();
  const label = form.visitor === "browse" ? "Time on each page" : "Observe after load";
  return (
    <OptionCell
      label={label}
      htmlFor={id}
      control={<span className="text-[13px] text-muted-foreground tabular-nums">{form.observeSeconds} s</span>}
    >
      <Slider
        id={id}
        aria-label={`${label}, seconds`}
        min={OBSERVE_SECONDS.min}
        max={OBSERVE_SECONDS.max}
        step={OBSERVE_SECONDS.step}
        value={[form.observeSeconds]}
        onValueChange={([observeSeconds]) => {
          if (observeSeconds !== undefined) update({ observeSeconds });
        }}
      />
      <OptionHelp>Longer windows catch refreshes and late lazy-loaded auctions.</OptionHelp>
    </OptionCell>
  );
}

function PagesToVisit({ form, update }: { form: ScanForm; update: (patch: Partial<ScanForm>) => void }) {
  const id = useId();
  if (form.visitor !== "browse") {
    return (
      <OptionCell label="Pages to visit" control={<FixedValue>1 page</FixedValue>}>
        <OptionHelp>Only “{VISITOR_META.browse.label}” follows links to further pages.</OptionHelp>
      </OptionCell>
    );
  }
  return (
    <OptionCell
      label="Pages to visit"
      htmlFor={id}
      control={
        <Stepper
          id={id}
          value={form.pages}
          min={VISIT_PAGES.min}
          max={VISIT_PAGES.max}
          onValueChange={(pages) => update({ pages })}
          decreaseLabel="Fewer pages"
          increaseLabel="More pages"
        >
          {pluralize(form.pages, "page")}
        </Stepper>
      }
    >
      <OptionHelp>
        {form.pages === 1
          ? "The entry page only; no link is followed."
          : `The entry page, then up to ${pluralize(form.pages - 1, "link")} on the same site. Ads, sign-in and checkout links are never clicked.`}
      </OptionHelp>
    </OptionCell>
  );
}

function SwitchOption({
  label,
  description,
  checked,
  onCheckedChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  const id = useId();
  return (
    <OptionCell
      label={label}
      htmlFor={id}
      control={<Switch id={id} checked={checked} onCheckedChange={onCheckedChange} />}
    >
      <OptionHelp>{description}</OptionHelp>
    </OptionCell>
  );
}

/** A cloud browser has no window on this Mac to show or hide. */
function NoBrowserWindow() {
  return (
    <OptionCell label="Show the browser" control={<FixedValue>No window</FixedValue>}>
      <OptionHelp>A cloud browser has no window. The report shows what it loaded.</OptionHelp>
    </OptionCell>
  );
}

/** A manual session leaves nothing to set: each option keeps its place and says what holds instead. */
function ManualSessionFacts({ remote }: { remote: boolean }) {
  return (
    <>
      <OptionCell label="Session length" control={<FixedValue>Up to {MANUAL_LIMIT_MINUTES} minutes</FixedValue>}>
        <OptionHelp>
          {remote ? "Browserbase's live view opens in your own browser" : "Chrome opens"}; browse the site, then press{" "}
          <span className="font-medium text-foreground">Finish capture</span>.
        </OptionHelp>
      </OptionCell>
      <OptionCell label="Pages to visit" control={<FixedValue>As you go</FixedValue>}>
        <OptionHelp>Every page you open in that {remote ? "browser" : "window"} is recorded.</OptionHelp>
      </OptionCell>
      {remote ? (
        <OptionCell label="Show the browser" control={<FixedValue>In the live view</FixedValue>}>
          <OptionHelp>Browserbase bills the session for as long as it is open.</OptionHelp>
        </OptionCell>
      ) : (
        <OptionCell label="Show the browser" control={<FixedValue>Always</FixedValue>}>
          <OptionHelp>There is nobody to browse a hidden window.</OptionHelp>
        </OptionCell>
      )}
      <OptionCell label="Accept consent banner" control={<FixedValue>By you</FixedValue>}>
        <OptionHelp>You handle the consent banner yourself; nothing is clicked for you.</OptionHelp>
      </OptionCell>
    </>
  );
}

/** How the visit behaves: who the visitor is, for how long and how far, and what the browser does meanwhile. */
export function VisitOptions({
  form,
  remote,
  update,
  onVisitorChange,
}: {
  form: ScanForm;
  /** The scan runs in a Browserbase browser. */
  remote: boolean;
  update: (patch: Partial<ScanForm>) => void;
  onVisitorChange: (visitor: VisitorBehavior) => void;
}) {
  return (
    <OptionsSection title="Visit" question="How should the visit behave?">
      <div className="flex flex-col gap-2">
        <ToggleGroup aria-label="Visitor" value={form.visitor} onValueChange={onVisitorChange} className="self-start">
          {VISITOR_BEHAVIORS.map((visitor) => (
            <ToggleGroupItem key={visitor} value={visitor}>
              {VISITOR_META[visitor].label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <div className="flex flex-col gap-0.5">
          <p className="text-xs text-pretty text-foreground">
            {remote && form.visitor === "manual"
              ? "You use the site in Browserbase's live view while it records. Nothing is clicked for you."
              : VISITOR_META[form.visitor].description}
          </p>
          <AutomationNote visitor={form.visitor} remote={remote} />
        </div>
      </div>
      <div className="grid auto-rows-[minmax(5.5rem,auto)] grid-cols-2 gap-x-8 gap-y-2 xl:auto-rows-[minmax(4.5rem,auto)]">
        {form.visitor === "manual" ? (
          <ManualSessionFacts remote={remote} />
        ) : (
          <>
            <ObserveTime form={form} update={update} />
            <PagesToVisit form={form} update={update} />
            {remote ? (
              <NoBrowserWindow />
            ) : (
              <SwitchOption
                label="Show the browser"
                description="Watch the visit in a Chrome window. Headless Chrome is easier for ad partners to recognise as automated."
                checked={form.showBrowser}
                onCheckedChange={(showBrowser) => update({ showBrowser })}
              />
            )}
            <SwitchOption
              label="Accept consent banner"
              description="Click through the CMP so bidding is not blocked."
              checked={form.acceptConsent}
              onCheckedChange={(acceptConsent) => update({ acceptConsent })}
            />
          </>
        )}
      </div>
    </OptionsSection>
  );
}

import type { ScanConfiguration, ScanSummary, Settings } from "@/bindings";
import { regionPlace } from "@/shared/domain/browserbase";
import { hasMissingProxy, proxyChoiceLabel } from "@/shared/domain/proxy";
import { VISITOR_META, visitorOf } from "@/shared/domain/scan-options";
import { pluralize } from "@/shared/lib/format";

/** Why a configuration cannot run as it is saved, in words that say how to fix it. `null` when it can. */
export function configurationProblem(
  configuration: ScanConfiguration,
  settings: Pick<Settings, "proxies" | "browserbase">,
): string | null {
  const { proxy, browser } = configuration.options;
  if (browser?.kind === "browserbase" && !settings.browserbase.connected) {
    return "Browserbase is not set up any more. Add its key in Settings, or edit the configuration to run on this Mac.";
  }
  if (!hasMissingProxy(configuration.options, settings)) return null;
  const name = proxy?.kind === "profile" && proxy.name ? `“${proxy.name}”` : "it used";
  return `The proxy ${name} was deleted. Edit the configuration and choose another proxy.`;
}

/** "Browse like a visitor, 3 pages". */
export function visitorSummary(configuration: ScanConfiguration): string {
  const { options } = configuration;
  const visitor = visitorOf(options);
  const label = VISITOR_META[visitor].label;
  if (visitor !== "browse" || options.extraPages === undefined) return label;
  return `${label}, ${pluralize(options.extraPages + 1, "page")}`;
}

/**
 * "Browserbase Frankfurt" for a configuration that runs there, and the same without the name for where a
 * mark already says it; `null` for one that runs on this Mac or follows the settings.
 */
export function browserSummary(
  configuration: ScanConfiguration,
  settings: Pick<Settings, "browserbase">,
): { full: string; place: string } | null {
  const { browser } = configuration.options;
  if (browser?.kind !== "browserbase") return null;
  const place = regionPlace(browser.region ?? settings.browserbase.region);
  return { full: `Browserbase ${place}`, place };
}

/** The proxy a run would use; a configuration that names none follows the settings at run time. */
export function proxySummary(configuration: ScanConfiguration, settings: Pick<Settings, "proxies">): string {
  const { proxy } = configuration.options;
  if (!proxy) return "Settings default";
  const label = proxyChoiceLabel(proxy, settings.proxies);
  return hasMissingProxy(configuration.options, settings) ? `${label} (deleted)` : label;
}

/** "Name copy", then "Name copy 2", …: the first one no configuration has yet (names compare without case). */
export function duplicateName(name: string, existing: string[]): string {
  const taken = new Set(existing.map((other) => other.trim().toLowerCase()));
  for (let copy = 1; ; copy += 1) {
    const candidate = `${name} copy${copy === 1 ? "" : ` ${copy}`}`;
    if (!taken.has(candidate.toLowerCase())) return candidate;
  }
}

/**
 * How the health score moved between a configuration's two newest scans (newest first). `null` unless
 * both completed with a score: a failed or running scan has nothing to compare.
 */
export function healthChange(scans: Pick<ScanSummary, "status" | "healthScore">[]): number | null {
  const [latest, previous] = scans;
  if (latest?.status !== "completed" || previous?.status !== "completed") return null;
  if (latest.healthScore === null || previous.healthScore === null) return null;
  return latest.healthScore - previous.healthScore;
}

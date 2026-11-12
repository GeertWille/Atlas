import type { ScanConfiguration, Settings } from "@/bindings";
import { hasMissingProxy, proxyChoiceLabel } from "@/shared/domain/proxy";
import { VISITOR_META, visitorOf } from "@/shared/domain/scan-options";
import { pluralize } from "@/shared/lib/format";

/** Why a configuration cannot run as it is saved, in words that say how to fix it. `null` when it can. */
export function configurationProblem(
  configuration: ScanConfiguration,
  settings: Pick<Settings, "proxies">,
): string | null {
  const { proxy } = configuration.options;
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

/** The proxy a run would use; a configuration that names none follows the settings at run time. */
export function proxySummary(configuration: ScanConfiguration, settings: Pick<Settings, "proxies">): string {
  const { proxy } = configuration.options;
  return proxy ? proxyChoiceLabel(proxy, settings.proxies) : "Settings default";
}

/** "Name copy", then "Name copy 2", …: the first one no configuration has yet (names compare without case). */
export function duplicateName(name: string, existing: string[]): string {
  const taken = new Set(existing.map((other) => other.trim().toLowerCase()));
  for (let copy = 1; ; copy += 1) {
    const candidate = `${name} copy${copy === 1 ? "" : ` ${copy}`}`;
    if (!taken.has(candidate.toLowerCase())) return candidate;
  }
}

import type { ScanConfiguration, Settings } from "@/bindings";
import { DEVICE_LABEL } from "@/shared/domain/vocabulary";
import { hostnameOf } from "@/shared/lib/url";
import { proxySummary, visitorSummary } from "./configuration-summary";

export type ConfigurationSort = "recent" | "name";

export const SORT_LABEL: Record<ConfigurationSort, string> = {
  recent: "Recently used",
  name: "Name",
};

/** The site a configuration scans, as people say it: no "www.". */
export function siteOf(configuration: ScanConfiguration): string {
  return hostnameOf(configuration.options.url).replace(/^www\./, "");
}

/** Configurations whose name, address, device, proxy or visit contain every word of the search. */
export function searchConfigurations(
  configurations: ScanConfiguration[],
  search: string,
  settings: Pick<Settings, "proxies">,
): ScanConfiguration[] {
  const terms = search.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return configurations;
  return configurations.filter((configuration) => {
    const haystack = [
      configuration.name,
      configuration.options.url,
      DEVICE_LABEL[configuration.options.device],
      proxySummary(configuration, settings),
      visitorSummary(configuration),
    ]
      .join(" ")
      .toLowerCase();
    return terms.every((term) => haystack.includes(term));
  });
}

/** When a configuration was last of interest: its last run, or the last change while it has never run. */
function lastActivity(configuration: ScanConfiguration): number {
  return configuration.lastRunAt ?? configuration.updatedAt;
}

export function sortConfigurations(configurations: ScanConfiguration[], sort: ConfigurationSort): ScanConfiguration[] {
  return [...configurations].sort((a, b) =>
    sort === "name"
      ? a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true })
      : lastActivity(b) - lastActivity(a),
  );
}

export type SiteGroup = { site: string; configurations: ScanConfiguration[] };

/** One group per site, in the order the sites first appear, so the sort also orders the groups. */
export function groupBySite(configurations: ScanConfiguration[]): SiteGroup[] {
  const groups = new Map<string, ScanConfiguration[]>();
  for (const configuration of configurations) {
    const site = siteOf(configuration);
    groups.set(site, [...(groups.get(site) ?? []), configuration]);
  }
  return [...groups].map(([site, grouped]) => ({ site, configurations: grouped }));
}

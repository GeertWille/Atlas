import type { ProxyChoice, ProxyProfile, ProxyUse, ScanOptions, Settings } from "@/bindings";
import { locationFromRecord, locationLabel } from "./browserbase";

/** A proxy profile's name can be this long; the backend enforces the same. */
export const PROXY_NAME_MAX = 60;

const HOST_NAME = /^(?=.{1,253}$)[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*$/i;
// At least two colons: "10.0.0.7:8080" and "cafe.de:8080" are a host with a port, not an address.
const IPV6 = /^(?=(?:[^:]*:){2})[0-9a-f:.]+$/i;

/** A bare host name or IP address: no scheme, port, path or credentials. */
export function isProxyHost(host: string): boolean {
  return HOST_NAME.test(host) || IPV6.test(host);
}

/** A choice that needs nothing but its key to be made again: `none`, `system` or `profile:<id>`. */
export type SavedProxyKey = "none" | "system" | `profile:${string}`;

/** A proxy choice as one string, for select controls. A Browserbase location keeps its place apart from the key. */
export type ProxyChoiceKey = SavedProxyKey | "browserbaseLocation";

export function proxyChoiceKey(choice: Exclude<ProxyChoice, { kind: "browserbaseLocation" }>): SavedProxyKey;
export function proxyChoiceKey(choice: ProxyChoice): ProxyChoiceKey;
export function proxyChoiceKey(choice: ProxyChoice): ProxyChoiceKey {
  return choice.kind === "profile" ? `profile:${choice.id}` : choice.kind;
}

export function proxyChoiceFromKey(key: SavedProxyKey): ProxyChoice {
  if (key === "none" || key === "system") return { kind: key };
  return { kind: "profile", id: key.slice("profile:".length) };
}

const KIND_LABEL = { none: "No proxy", system: "System proxy" } as const;

/** Name of a choice. A profile that no longer exists falls back to the name stored with the choice. */
export function proxyChoiceLabel(choice: ProxyChoice, proxies: ProxyProfile[]): string {
  if (choice.kind === "browserbaseLocation") return `${locationLabel(choice)} (Browserbase proxy)`;
  if (choice.kind !== "profile") return KIND_LABEL[choice.kind];
  return proxies.find((proxy) => proxy.id === choice.id)?.name ?? choice.name ?? "Deleted proxy";
}

/** The proxy a finished scan ran through, as recorded with it. */
export function proxyUseLabel(proxy: ProxyUse): string {
  if (proxy.kind === "browserbaseLocation") {
    return proxy.name ? `${locationLabel(locationFromRecord(proxy.name))} (Browserbase proxy)` : "Browserbase proxy";
  }
  if (proxy.kind !== "profile") return KIND_LABEL[proxy.kind];
  const name = proxy.name ?? "Proxy";
  return proxy.host ? `${name} (${proxy.host})` : name;
}

/** `http://be.proxy.example:8080`, for display. Never includes credentials. */
export function proxyAddress(proxy: Pick<ProxyProfile, "scheme" | "host" | "port">): string {
  const host = proxy.host.includes(":") ? `[${proxy.host}]` : proxy.host;
  return `${proxy.scheme}://${host}:${proxy.port}`;
}

/** Options that name a proxy profile which has been deleted cannot start a scan. */
export function hasMissingProxy(options: Pick<ScanOptions, "proxy">, settings: Pick<Settings, "proxies">): boolean {
  const { proxy } = options;
  return proxy?.kind === "profile" && !settings.proxies.some((profile) => profile.id === proxy.id);
}

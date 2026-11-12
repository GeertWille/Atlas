import type { ProxyChoice, ProxyProfile, ProxyUse, ScanOptions, Settings } from "@/bindings";

/** A proxy choice as one string, for select controls: `none`, `system` or `profile:<id>`. */
export type ProxyChoiceKey = "none" | "system" | `profile:${string}`;

export function proxyChoiceKey(choice: ProxyChoice): ProxyChoiceKey {
  return choice.kind === "profile" ? `profile:${choice.id}` : choice.kind;
}

export function proxyChoiceFromKey(key: ProxyChoiceKey): ProxyChoice {
  if (key === "none" || key === "system") return { kind: key };
  return { kind: "profile", id: key.slice("profile:".length) };
}

const KIND_LABEL = { none: "No proxy", system: "System proxy" } as const;

/** Name of a choice. A profile that no longer exists falls back to the name stored with the choice. */
export function proxyChoiceLabel(choice: ProxyChoice, proxies: ProxyProfile[]): string {
  if (choice.kind !== "profile") return KIND_LABEL[choice.kind];
  return proxies.find((proxy) => proxy.id === choice.id)?.name ?? choice.name ?? "Deleted proxy";
}

/** The proxy a finished scan ran through, as recorded with it. */
export function proxyUseLabel(proxy: ProxyUse): string {
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

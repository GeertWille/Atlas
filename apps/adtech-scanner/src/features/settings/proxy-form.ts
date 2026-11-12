import type { ProxyProfile, ProxyProfileDraft, ProxyScheme } from "@/bindings";

export const PROXY_SCHEMES: readonly ProxyScheme[] = ["http", "https", "socks5"];

export const PROXY_SCHEME_LABEL: Record<ProxyScheme, string> = { http: "HTTP", https: "HTTPS", socks5: "SOCKS5" };

export const PROXY_NAME_MAX = 60;

/** The proxy dialog as typed. The password never leaves the dialog except on its way to the keychain. */
export type ProxyForm = {
  name: string;
  scheme: ProxyScheme;
  host: string;
  port: string;
  username: string;
  password: string;
};

export type ProxyFormErrors = Partial<Record<keyof ProxyForm, string>>;

export type ProxyFormResult =
  | { ok: true; draft: ProxyProfileDraft /** `null`: leave the stored password as it is. */; password: string | null }
  | { ok: false; errors: ProxyFormErrors };

const DEFAULT_PORT: Record<ProxyScheme, string> = { http: "8080", https: "443", socks5: "1080" };

const HOST_NAME = /^(?=.{1,253}$)[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*$/i;
const IPV6 = /^[0-9a-f:.]+$/i;

export function newProxyForm(): ProxyForm {
  return { name: "", scheme: "http", host: "", port: DEFAULT_PORT.http, username: "", password: "" };
}

export function proxyFormOf(proxy: ProxyProfile): ProxyForm {
  return {
    name: proxy.name,
    scheme: proxy.scheme,
    host: proxy.host,
    port: String(proxy.port),
    username: proxy.username ?? "",
    password: "",
  };
}

/** Switching the scheme moves the port along, unless the user already typed one of their own. */
export function withScheme(form: ProxyForm, scheme: ProxyScheme): ProxyForm {
  const untouched = form.port.trim() === "" || form.port === DEFAULT_PORT[form.scheme];
  return { ...form, scheme, port: untouched ? DEFAULT_PORT[scheme] : form.port };
}

function isHost(host: string): boolean {
  return HOST_NAME.test(host) || (host.includes(":") && IPV6.test(host));
}

/**
 * Checks the dialog before anything is sent. `others` are the names of the other proxies, and
 * `id` the proxy being edited, if any.
 */
export function validateProxyForm(form: ProxyForm, others: string[], id?: string): ProxyFormResult {
  const name = form.name.trim();
  const host = form.host.trim();
  const username = form.username.trim();
  const port = Number(form.port.trim());
  const errors: ProxyFormErrors = {};

  if (name === "") errors.name = "Give the proxy a name, like the country it is in.";
  else if (others.some((other) => other.trim().toLowerCase() === name.toLowerCase())) {
    errors.name = `A proxy named “${name}” already exists.`;
  }
  if (host === "") errors.host = "Enter the proxy's host name or IP address.";
  else if (!isHost(host)) {
    errors.host = "Only the host name or IP address, like proxy.example.com: no http://, port, path or password.";
  }
  if (!/^\d+$/.test(form.port.trim()) || port < 1 || port > 65_535) errors.port = "A port from 1 to 65535.";
  if (form.scheme === "socks5" && username !== "") {
    errors.username = "Chrome cannot sign in to a SOCKS5 proxy. Remove the user name, or use an HTTP or HTTPS proxy.";
  } else if (username === "" && form.password !== "") {
    errors.username = "Enter the user name this password belongs to.";
  }

  if (Object.keys(errors).length > 0) return { ok: false, errors };
  return {
    ok: true,
    draft: { ...(id === undefined ? {} : { id }), name, scheme: form.scheme, host, port, username: username || null },
    password: form.password === "" ? null : form.password,
  };
}

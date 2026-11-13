/** `https://www.example.com/a/b?x=1` → `/a/b?x=1`; empty for the bare origin. */
export function pathOf(url: string): string {
  try {
    const parsed = new URL(url);
    const path = `${parsed.pathname}${parsed.search}`;
    return path === "/" ? "" : path;
  } catch {
    return "";
  }
}

/** Hostname of a URL; the input itself when it cannot be parsed, so a malformed value still shows. */
export function hostnameOf(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return url;
  }
}

/** Last path segment of a script URL, for compact display. */
export function fileNameOf(url: string): string {
  try {
    const parsed = new URL(url.replace(/^inline:/, "https:"));
    const segment = parsed.pathname.split("/").filter(Boolean).pop();
    if (url.startsWith("inline:")) return `inline script ${parsed.hash}`;
    return segment ? `${segment}${parsed.search}` : parsed.hostname;
  } catch {
    return url;
  }
}

function isPrivateIpv4(host: string): boolean {
  const parts = host.split(".").map(Number);
  if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)) return false;
  const [a = 0, b = 0] = parts;
  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 100 && b >= 64 && b <= 127) ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168)
  );
}

function isPrivateIpv6(address: string): boolean {
  // An IPv4 address written as IPv6, which the URL parser spells in hex: `::ffff:a00:1` is 10.0.0.1.
  const mapped = /^::ffff:([0-9a-f]{1,4}):([0-9a-f]{1,4})$/.exec(address);
  if (mapped) {
    const [high, low] = [Number.parseInt(mapped[1] ?? "", 16), Number.parseInt(mapped[2] ?? "", 16)];
    return isPrivateIpv4(`${high >> 8}.${high & 255}.${low >> 8}.${low & 255}`);
  }
  // Loopback and unspecified, unique local (fc00::/7) and link-local (fe80::/10).
  return address === "::1" || address === "::" || /^f[cd]/.test(address) || /^fe[89ab]/.test(address);
}

/**
 * Whether an address is on this computer or on a private network: loopback, private, link-local and
 * shared ranges, and the names `localhost`, `*.localhost` and `*.local`. A browser that runs somewhere
 * else cannot reach such a page. Says false for anything that is not a URL.
 */
export function isLocalOrPrivateUrl(url: string): boolean {
  let hostname: string;
  try {
    hostname = new URL(url).hostname.toLowerCase().replace(/\.$/, "");
  } catch {
    return false;
  }
  if (hostname.startsWith("[")) return isPrivateIpv6(hostname.slice(1, -1));
  if (hostname === "localhost" || hostname.endsWith(".localhost") || hostname.endsWith(".local")) return true;
  return isPrivateIpv4(hostname);
}

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
    // "This network", private, loopback, carrier-grade NAT, link-local, multicast, and the broadcast address.
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 100 && b >= 64 && b <= 127) ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a >= 224 && a <= 239) ||
    host === "255.255.255.255"
  );
}

/** The eight 16-bit groups of an IPv6 address as the URL parser writes it (hex, at most one `::`). */
function ipv6Groups(address: string): number[] | null {
  const halves = address.split("::");
  if (halves.length > 2) return null;
  const groupsOf = (part: string) => (part === "" ? [] : part.split(":").map((group) => Number.parseInt(group, 16)));
  const [head, tail] = [groupsOf(halves[0] ?? ""), groupsOf(halves[1] ?? "")];
  const gap = halves.length === 2 ? 8 - head.length - tail.length : 0;
  const groups = [...head, ...new Array<number>(Math.max(0, gap)).fill(0), ...tail];
  return groups.length === 8 && groups.every((group) => group >= 0 && group <= 0xffff) ? groups : null;
}

/**
 * The backend's rule for IPv6. An address that only wraps an IPv4 address is as local as that address:
 * IPv4-mapped (`::ffff:a.b.c.d`), IPv4-compatible (`::a.b.c.d`, which covers `::` and `::1`), NAT64
 * (`64:ff9b::a.b.c.d`) and 6to4 (`2002:aabb:ccdd::/48`). Otherwise: multicast, unique local (fc00::/7),
 * link-local (fe80::/10) and the NAT64 range a network keeps for itself (64:ff9b:1::/48).
 */
function isPrivateIpv6(address: string): boolean {
  const groups = ipv6Groups(address);
  if (!groups) return false;
  const [g0 = 0, g1 = 0, g2 = 0, g3 = 0, g4 = 0, g5 = 0, g6 = 0, g7 = 0] = groups;
  const ipv4 = (high: number, low: number) => `${high >> 8}.${high & 255}.${low >> 8}.${low & 255}`;
  const leadingZeros = g0 === 0 && g1 === 0 && g2 === 0 && g3 === 0 && g4 === 0;
  if (leadingZeros && (g5 === 0xffff || g5 === 0)) return isPrivateIpv4(ipv4(g6, g7));
  if (g0 === 0x64 && g1 === 0xff9b && g2 === 0 && g3 === 0 && g4 === 0 && g5 === 0) return isPrivateIpv4(ipv4(g6, g7));
  if (g0 === 0x2002) return isPrivateIpv4(ipv4(g1, g2));
  const multicast = g0 >> 8 === 0xff;
  const uniqueLocal = (g0 & 0xfe00) === 0xfc00;
  const linkLocal = (g0 & 0xffc0) === 0xfe80;
  const localNat64 = g0 === 0x64 && g1 === 0xff9b && g2 === 1;
  return multicast || uniqueLocal || linkLocal || localNat64;
}

/**
 * Whether an address is on this computer or on a private network: loopback, private, link-local, shared
 * and multicast ranges, in whatever form the address is written, and the names `localhost`, `*.localhost`
 * and `*.local`. A browser that runs somewhere else cannot reach such a page. The same answers as the
 * backend's `is_local_only` for an address. Says false for anything that is not a URL.
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

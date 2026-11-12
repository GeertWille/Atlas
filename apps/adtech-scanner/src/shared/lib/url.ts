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

import { useEffect, useState } from "react";
import type { ThemePreference } from "@/bindings";

/** Also read by `public/theme-boot.js` before first paint. */
const STORAGE_KEY = "adscan.theme";
const DARK_QUERY = "(prefers-color-scheme: dark)";

export type ResolvedTheme = "light" | "dark";

function resolve(preference: ThemePreference, systemDark: boolean): ResolvedTheme {
  if (preference === "system") return systemDark ? "dark" : "light";
  return preference;
}

/** Applies the theme preference to the document and keeps following the OS while it is "system". */
export function useAppliedTheme(preference: ThemePreference | undefined): ResolvedTheme {
  const [systemDark, setSystemDark] = useState(() => window.matchMedia(DARK_QUERY).matches);

  useEffect(() => {
    const media = window.matchMedia(DARK_QUERY);
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, []);

  // Until settings load, keep whatever the boot script applied.
  const resolved: ResolvedTheme =
    preference === undefined
      ? document.documentElement.classList.contains("dark")
        ? "dark"
        : "light"
      : resolve(preference, systemDark);

  useEffect(() => {
    if (preference === undefined) return;
    document.documentElement.classList.toggle("dark", resolved === "dark");
    try {
      localStorage.setItem(STORAGE_KEY, preference);
    } catch {
      // The preference still applies for this session.
    }
  }, [preference, resolved]);

  return resolved;
}

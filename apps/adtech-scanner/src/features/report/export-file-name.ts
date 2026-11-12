import type { ExportFormat, ScanSummary } from "@/bindings";

const EXTENSION: Record<ExportFormat, string> = { markdown: "md", json: "json" };

/** `adscan-www.example.com-desktop-2026-11-12.md` */
export function exportFileName(
  scan: Pick<ScanSummary, "hostname" | "device" | "startedAt">,
  format: ExportFormat,
): string {
  const day = new Date(scan.startedAt).toISOString().slice(0, 10);
  const host = scan.hostname.replace(/[^a-z0-9.-]/gi, "_");
  return `adscan-${host}-${scan.device}-${day}.${EXTENSION[format]}`;
}

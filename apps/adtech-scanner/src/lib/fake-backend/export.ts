import type { ExportFormat, Finding, ScanDetail } from "@/bindings";

function findingMarkdown(finding: Finding, index: number): string {
  const evidence = finding.evidence
    .map((item) => {
      const source = item.scriptUrl ? ` (${item.scriptUrl}${item.line ? `:${item.line}` : ""})` : "";
      const body = item.kind === "code" ? `\`\`\`js\n${item.body}\n\`\`\`` : `> ${item.body}`;
      return `**${item.label}**${source}\n\n${body}`;
    })
    .join("\n\n");
  const impact = finding.estimatedRevenueImpactPct === null ? "" : ` · est. ${finding.estimatedRevenueImpactPct}% of revenue`;
  return [
    `### ${index + 1}. ${finding.title}`,
    `*${finding.severity.toUpperCase()} · ${finding.category} · ${finding.source} · confidence ${Math.round(finding.confidence * 100)}%${impact}*`,
    finding.description,
    `**Recommendation**\n\n${finding.recommendation}`,
    evidence,
  ]
    .filter(Boolean)
    .join("\n\n");
}

function scanMarkdown(detail: ScanDetail): string {
  const { summary, metrics } = detail;
  return [
    `# Ad stack scan: ${summary.hostname}`,
    `- URL: ${summary.url}\n- Device: ${summary.device}\n- Scanned: ${new Date(summary.startedAt).toISOString()}\n- Health score: ${summary.healthScore ?? "n/a"}/100\n- LLM cost: $${summary.costUsd.toFixed(3)}`,
    detail.overview ?? "",
    `## Key metrics\n\n| Metric | Value |\n|---|---|\n| Fill rate | ${Math.round(metrics.fillRate * 100)}% |\n| Bid timeout rate | ${Math.round(metrics.timeoutRate * 100)}% |\n| Auctions | ${metrics.auctionCount} |\n| Bidders | ${metrics.bidderCount} |\n| Slots | ${metrics.slotCount} (${metrics.emptySlotCount} not filled) |\n| Scripts | ${metrics.scriptCount} (${metrics.adScriptCount} ad-related) |`,
    `## Findings (${detail.findings.length})`,
    ...detail.findings.map(findingMarkdown),
  ]
    .filter(Boolean)
    .join("\n\n");
}

export function renderExport(detail: ScanDetail, format: ExportFormat): string {
  return format === "json" ? JSON.stringify(detail, null, 2) : scanMarkdown(detail);
}

/** Browser stand-in for "write to `path`": hand the document to the user as a download. */
export function downloadDocument(fileName: string, format: ExportFormat, content: string) {
  const type = format === "json" ? "application/json" : "text/markdown";
  const href = URL.createObjectURL(new Blob([content], { type }));
  const anchor = Object.assign(document.createElement("a"), { href, download: fileName });
  anchor.click();
  URL.revokeObjectURL(href);
}

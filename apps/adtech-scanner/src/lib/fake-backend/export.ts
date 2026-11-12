import type { ExportFormat, Finding, PageView, ScanDetail } from "@/bindings";
import { PAGE_REACH_LABEL, pagePath } from "@/shared/domain/page-views";
import { proxyUseLabel } from "@/shared/domain/proxy";
import { VISITOR_META } from "@/shared/domain/scan-options";

function visitMarkdown(detail: ScanDetail): string | null {
  const { environment, pages } = detail;
  if (!environment) return null;
  const facts = [
    `- Visitor: ${VISITOR_META[environment.visitor].label}`,
    `- Browser: ${environment.headless ? "headless" : "shown"}, ${environment.persistentProfile ? "kept scan profile" : "fresh profile"}`,
    `- Proxy: ${proxyUseLabel(environment.proxy)}`,
    `- navigator.webdriver: ${environment.webdriver ?? "unknown"}`,
  ];
  const table =
    pages.length > 1
      ? `\n\n### Page views\n\n| # | Page | Reached by | Auctions | Slots |\n|---|---|---|---|---|\n${pages
          .map(
            (page) =>
              `| ${page.index + 1} | ${pagePath(page.url)} | ${PAGE_REACH_LABEL[page.reachedBy]} | ${page.metrics.auctionCount} | ${page.metrics.slotCount} |`,
          )
          .join("\n")}`
      : "";
  return `## Visit\n\n${facts.join("\n")}${table}`;
}

function findingMarkdown(finding: Finding, index: number, pages: PageView[]): string {
  const evidence = finding.evidence
    .map((item) => {
      const source = item.scriptUrl ? ` (${item.scriptUrl}${item.line ? `:${item.line}` : ""})` : "";
      const body = item.kind === "code" ? `\`\`\`js\n${item.body}\n\`\`\`` : `> ${item.body}`;
      return `**${item.label}**${source}\n\n${body}`;
    })
    .join("\n\n");
  const impact =
    finding.estimatedRevenueImpactPct === null ? "" : ` · est. ${finding.estimatedRevenueImpactPct}% of revenue`;
  return [
    `### ${index + 1}. ${finding.title}`,
    `*${finding.severity.toUpperCase()} · ${finding.category} · ${finding.source} · confidence ${Math.round(finding.confidence * 100)}%${impact}*`,
    finding.description,
    pages.length > 1 && finding.pages.length > 0
      ? `Seen on: ${finding.pages.map((page) => `page ${page + 1}`).join(", ")}`
      : "",
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
    `- URL: ${summary.url}\n- Device: ${summary.device}\n- Scanned: ${new Date(summary.startedAt).toISOString()}${summary.configuration ? `\n- Configuration: ${summary.configuration.name}` : ""}\n- Health score: ${summary.healthScore ?? "n/a"}/100\n- LLM cost: $${summary.costUsd.toFixed(3)}`,
    detail.overview ?? "",
    visitMarkdown(detail) ?? "",
    `## Key metrics\n\n| Metric | Value |\n|---|---|\n| Fill rate | ${Math.round(metrics.fillRate * 100)}% |\n| Bid timeout rate | ${Math.round(metrics.timeoutRate * 100)}% |\n| Auctions | ${metrics.auctionCount} |\n| Bidders | ${metrics.bidderCount} |\n| Slots | ${metrics.slotCount} (${metrics.emptySlotCount} empty) |\n| Scripts | ${metrics.scriptCount} (${metrics.adScriptCount} ad-related) |`,
    `## Findings (${detail.findings.length})`,
    ...detail.findings.map((finding, index) => findingMarkdown(finding, index, detail.pages)),
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

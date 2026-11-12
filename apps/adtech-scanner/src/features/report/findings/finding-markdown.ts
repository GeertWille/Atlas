import type { Evidence, Finding, PageView } from "@/bindings";
import { pagePath } from "@/shared/domain/page-views";
import { FINDING_CATEGORY_LABEL, FINDING_SOURCE_LABEL, SEVERITY_META } from "@/shared/domain/vocabulary";
import { formatPercent } from "@/shared/lib/format";

function evidenceMarkdown(evidence: Evidence): string {
  const location = evidence.scriptUrl ? ` (${evidence.scriptUrl}${evidence.line ? `:${evidence.line}` : ""})` : "";
  const body = evidence.kind === "code" ? `\`\`\`js\n${evidence.body}\n\`\`\`` : `> ${evidence.body}`;
  return `**${evidence.label}**${location}\n\n${body}`;
}

/** The page views a finding was seen on, out of those of a multi-page visit. Empty for a single-page scan. */
export function findingPages(finding: Finding, pages: PageView[]): PageView[] {
  return pages.length > 1 ? pages.filter((page) => finding.pages.includes(page.index)) : [];
}

/**
 * A finding as a self-contained markdown note, ready to paste into a ticket. `pages` are the page
 * views of the scan; a multi-page visit adds where the finding was seen.
 */
export function findingToMarkdown(finding: Finding, pages: PageView[] = []): string {
  const seenOn = findingPages(finding, pages);
  const facts = [
    SEVERITY_META[finding.severity].label,
    FINDING_CATEGORY_LABEL[finding.category],
    FINDING_SOURCE_LABEL[finding.source],
    `confidence ${formatPercent(finding.confidence)}`,
    finding.estimatedRevenueImpactPct === null
      ? null
      : `est. ${finding.estimatedRevenueImpactPct}% of ad revenue at risk`,
  ].filter(Boolean);
  const affected = [
    finding.affectedSlots.length > 0 ? `Slots: ${finding.affectedSlots.join(", ")}` : null,
    finding.affectedBidders.length > 0 ? `Bidders: ${finding.affectedBidders.join(", ")}` : null,
    seenOn.length > 0
      ? `Seen on: ${seenOn.map((page) => `page ${page.index + 1} (${pagePath(page.url)})`).join(", ")}`
      : null,
  ].filter(Boolean);
  return [
    `## ${finding.title}`,
    `*${facts.join(" · ")}*`,
    `**Impact:** ${finding.impact}`,
    finding.description,
    `### Recommendation\n\n${finding.recommendation}`,
    finding.evidence.length > 0 ? `### Evidence\n\n${finding.evidence.map(evidenceMarkdown).join("\n\n")}` : null,
    affected.length > 0 ? affected.join("\n") : null,
  ]
    .filter(Boolean)
    .join("\n\n");
}

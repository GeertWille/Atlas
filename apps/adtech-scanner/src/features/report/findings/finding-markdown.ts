import type { Evidence, Finding } from "@/bindings";
import { FINDING_CATEGORY_LABEL, FINDING_SOURCE_LABEL, SEVERITY_META } from "@/shared/domain/vocabulary";
import { formatPercent } from "@/shared/lib/format";

function evidenceMarkdown(evidence: Evidence): string {
  const location = evidence.scriptUrl ? ` (${evidence.scriptUrl}${evidence.line ? `:${evidence.line}` : ""})` : "";
  const body = evidence.kind === "code" ? `\`\`\`js\n${evidence.body}\n\`\`\`` : `> ${evidence.body}`;
  return `**${evidence.label}**${location}\n\n${body}`;
}

/** A finding as a self-contained markdown note, ready to paste into a ticket. */
export function findingToMarkdown(finding: Finding): string {
  const facts = [
    SEVERITY_META[finding.severity].label,
    FINDING_CATEGORY_LABEL[finding.category],
    FINDING_SOURCE_LABEL[finding.source],
    `confidence ${formatPercent(finding.confidence)}`,
    finding.estimatedRevenueImpactPct === null ? null : `est. ${finding.estimatedRevenueImpactPct}% of ad revenue at risk`,
  ].filter(Boolean);
  const affected = [
    finding.affectedSlots.length > 0 ? `Slots: ${finding.affectedSlots.join(", ")}` : null,
    finding.affectedBidders.length > 0 ? `Bidders: ${finding.affectedBidders.join(", ")}` : null,
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

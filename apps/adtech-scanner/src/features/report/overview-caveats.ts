export type OverviewParts = {
  summary: string;
  /** What limited the scan (a budget stop, a failed AI step, a classifier fallback), without the markdown label. */
  caveats: string[];
};

const CAVEAT = /^\*\*(?:Note|Warning):\*\*\s*/;

/** The backend appends each caveat to the overview as its own `**Note:** …` or `**Warning:** …` paragraph. */
export function splitOverview(overview: string): OverviewParts {
  const paragraphs = overview.split(/\n{2,}/).map((paragraph) => paragraph.trim());
  return {
    summary: paragraphs.filter((paragraph) => !CAVEAT.test(paragraph)).join("\n\n"),
    caveats: paragraphs.filter((paragraph) => CAVEAT.test(paragraph)).map((paragraph) => paragraph.replace(CAVEAT, "")),
  };
}

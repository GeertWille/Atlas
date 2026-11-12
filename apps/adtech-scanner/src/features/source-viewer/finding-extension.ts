import { type Extension, Prec, type Range, RangeSet } from "@codemirror/state";
import { Decoration, EditorView, GutterMarker, gutter, WidgetType } from "@codemirror/view";
import type { Severity } from "@/bindings";
import { SEVERITY_META } from "@/shared/domain/vocabulary";
import { jumpToLine, svgIcon } from "@/shared/editor";
import { adjacentMark, type FindingMark, type GutterMark, gutterMarks, type LineRange, lineTints } from "./anchors";

/** One shape per severity, so a marker reads without its colour. The same shapes as the severity badges. */
const SEVERITY_SHAPE: Record<Severity, readonly string[]> = {
  critical: [
    "M2.586 16.726A2 2 0 0 1 2 15.312V8.688a2 2 0 0 1 .586-1.414l4.688-4.688A2 2 0 0 1 8.688 2h6.624a2 2 0 0 1 1.414.586l4.688 4.688A2 2 0 0 1 22 8.688v6.624a2 2 0 0 1-.586 1.414l-4.688 4.688a2 2 0 0 1-1.414.586H8.688a2 2 0 0 1-1.414-.586z",
    "M12 8v4",
    "M12 16h.01",
  ],
  high: ["m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3", "M12 9v4", "M12 17h.01"],
  medium: ["M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20z", "M12 11a1 1 0 1 0 0 2 1 1 0 0 0 0-2z"],
  low: ["M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20z"],
  info: ["M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20z", "M12 16v-4", "M12 8h.01"],
};

const ICON_CLASS = "size-3.5 fill-none stroke-current stroke-2 [stroke-linecap:round] [stroke-linejoin:round]";

export function lensKey(mark: Pick<FindingMark, "findingId" | "evidenceIndex">): string {
  return `${mark.findingId}:${mark.evidenceIndex}`;
}

export type FindingActions = {
  /** Opens or closes the one-line summary under a marked range. */
  toggleLens: (key: string) => void;
  showLens: (key: string) => void;
  openFinding: (findingId: string) => void;
};

class FindingGutterMarker extends GutterMarker {
  constructor(private readonly mark: GutterMark) {
    super();
  }

  override eq(other: FindingGutterMarker) {
    return other.mark.severity === this.mark.severity && other.mark.located === this.mark.located;
  }

  override toDOM() {
    const { severity, located, marks } = this.mark;
    const label = SEVERITY_META[severity].label;
    const titles = marks.map((mark) => mark.title).join("; ");
    const element = document.createElement("span");
    element.className = `cm-findingMarker cm-severity-${severity}${located ? "" : " cm-findingMarker-unlocated"}`;
    element.setAttribute("role", "img");
    element.setAttribute(
      "aria-label",
      `${label} finding: ${titles}${located ? "" : " (excerpt not found again; stored lines shown)"}`,
    );
    element.title = `${label}: ${titles}${located ? "" : "\nThe excerpt was not found again; the stored lines are shown."}`;
    element.append(svgIcon(SEVERITY_SHAPE[severity], ICON_CLASS));
    return element;
  }
}

class LensWidget extends WidgetType {
  constructor(
    private readonly mark: FindingMark,
    private readonly actions: FindingActions,
    /** The finding F8 would go to from here; `null` when this is the file's only one. */
    private readonly next: FindingMark | null,
  ) {
    super();
  }

  override eq(other: LensWidget) {
    return lensKey(other.mark) === lensKey(this.mark) && other.mark.title === this.mark.title;
  }

  override toDOM(view: EditorView) {
    const { mark, actions, next } = this;
    const lens = document.createElement("div");
    lens.className = `cm-findingLens cm-severity-${mark.severity}`;
    lens.setAttribute("role", "note");
    lens.setAttribute("aria-label", `Finding: ${mark.title}`);

    const severity = document.createElement("span");
    severity.className = "cm-findingLens-severity";
    severity.append(svgIcon(SEVERITY_SHAPE[mark.severity], ICON_CLASS), SEVERITY_META[mark.severity].label);

    const title = document.createElement("span");
    title.className = "cm-findingLens-title";
    // Finding text is the model's or a rule's wording: text only, never markup.
    title.textContent = mark.title;
    title.title = mark.title;

    const button = (label: string, onClick: () => void) => {
      const element = document.createElement("button");
      element.type = "button";
      element.className = "cm-findingLens-action";
      element.textContent = label;
      element.addEventListener("click", onClick);
      return element;
    };
    lens.append(
      severity,
      title,
      button("Open finding", () => actions.openFinding(mark.findingId)),
    );
    if (next) {
      lens.append(
        button("Next (F8)", () => {
          jumpToLine(view, next.startLine, next.endLine);
          actions.showLens(lensKey(next));
        }),
      );
    }
    return lens;
  }

  override ignoreEvent() {
    // The buttons handle their own clicks; the editor must not move the caret for them.
    return true;
  }
}

const TONE: Record<Severity, string> = {
  critical: "red",
  high: "orange",
  medium: "amber",
  low: "blue",
  info: "gray",
};

const severityRules = Object.fromEntries(
  (Object.keys(TONE) as Severity[]).flatMap((severity) => {
    const tone = TONE[severity];
    return [
      [`.cm-severity-${severity}`, { color: `var(--tone-${tone}-fg)` }],
      [
        `.cm-line.cm-findingLine-${severity}`,
        { backgroundColor: `var(--tone-${tone}-soft)`, boxShadow: `inset 3px 0 0 var(--tone-${tone})` },
      ],
      [
        `.cm-findingLens.cm-severity-${severity}`,
        { borderColor: `var(--tone-${tone}-line)`, backgroundColor: `var(--tone-${tone}-soft)` },
      ],
    ];
  }),
);

const findingTheme = EditorView.theme({
  ".cm-findingGutter": { width: "20px" },
  ".cm-findingGutter .cm-gutterElement": { display: "flex", alignItems: "center", justifyContent: "center" },
  ".cm-findingMarker": {
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    width: "18px",
    height: "18px",
    borderRadius: "4px",
    cursor: "default",
  },
  ".cm-findingMarker:hover": { backgroundColor: "var(--accent)" },
  // An excerpt that could not be found again keeps its stored lines; the dashed ring says it is approximate.
  ".cm-findingMarker-unlocated": { outline: "1px dashed currentColor", outlineOffset: "-2px" },
  ".cm-findingLens": {
    display: "flex",
    alignItems: "center",
    gap: "10px",
    margin: "3px 12px 5px 6px",
    padding: "0 4px 0 8px",
    height: "26px",
    border: "1px solid",
    borderRadius: "6px",
    fontFamily: "var(--font-sans)",
    fontSize: "12px",
    lineHeight: "1",
    userSelect: "none",
    WebkitUserSelect: "none",
    cursor: "default",
  },
  ".cm-findingLens-severity": { display: "flex", alignItems: "center", gap: "5px", fontWeight: "600", flexShrink: "0" },
  ".cm-findingLens-title": {
    flex: "1",
    minWidth: "0",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    color: "var(--foreground)",
  },
  ".cm-findingLens-action": {
    flexShrink: "0",
    height: "20px",
    padding: "0 7px",
    borderRadius: "4px",
    color: "var(--foreground)",
    fontWeight: "500",
  },
  ".cm-findingLens-action:hover": { backgroundColor: "color-mix(in oklab, var(--foreground) 9%, transparent)" },
  ".cm-findingLens-action:focus-visible": { outline: "2px solid var(--ring)", outlineOffset: "1px" },
  // With "dim unread" on, what the AI was shown keeps full strength and a violet edge; the rest steps back.
  "&.cm-dimUnread .cm-line:not(.cm-readLine)": { opacity: "0.42" },
  "&.cm-dimUnread .cm-line.cm-readLine:not([class*='cm-findingLine'])": {
    boxShadow: "inset 2px 0 0 var(--tone-violet-line)",
  },
  ...severityRules,
});

export type FindingExtensionInput = {
  marks: FindingMark[];
  /** Keys (`lensKey`) of the marks whose summary line is shown. */
  openLenses: ReadonlySet<string>;
  /** Lines the AI was shown. */
  read: LineRange[];
  dimUnread: boolean;
  actions: FindingActions;
};

/**
 * Everything the viewer draws on top of the code: a gutter marker where a finding starts, a tint
 * over its lines, an optional one-line summary under it, and the dimming of lines the AI never saw.
 */
export function findingExtension({ marks, openLenses, read, dimUnread, actions }: FindingExtensionInput): Extension {
  const gutterEntries = gutterMarks(marks);
  const tints = lineTints(marks);
  const lenses = marks.filter((mark) => openLenses.has(lensKey(mark)));

  const decorations = EditorView.decorations.compute(["doc"], (state) => {
    const lineCount = state.doc.lines;
    const ranges: Range<Decoration>[] = [];
    const lineClasses = new Map<number, string[]>();
    const addClass = (line: number, name: string) => lineClasses.set(line, [...(lineClasses.get(line) ?? []), name]);
    for (const tint of tints) if (tint.line <= lineCount) addClass(tint.line, `cm-findingLine-${tint.severity}`);
    if (dimUnread) {
      for (const range of read) {
        for (let line = range.startLine; line <= Math.min(range.endLine, lineCount); line += 1)
          addClass(line, "cm-readLine");
      }
    }
    for (const [line, names] of lineClasses) {
      ranges.push(Decoration.line({ class: names.join(" ") }).range(state.doc.line(line).from));
    }
    for (const mark of lenses) {
      if (mark.endLine > lineCount) continue;
      const next = marks.length > 1 ? adjacentMark(marks, mark.startLine, 1) : null;
      const widget = new LensWidget(mark, actions, next);
      ranges.push(Decoration.widget({ widget, block: true, side: 1 }).range(state.doc.line(mark.endLine).to));
    }
    return Decoration.set(ranges, true);
  });

  const findingGutter = gutter({
    class: "cm-findingGutter",
    markers: (view) =>
      RangeSet.of(
        gutterEntries
          .filter((entry) => entry.line <= view.state.doc.lines)
          .map((entry) => new FindingGutterMarker(entry).range(view.state.doc.line(entry.line).from)),
      ),
    domEventHandlers: {
      mousedown(view, block) {
        const line = view.state.doc.lineAt(block.from).number;
        const entry = gutterEntries.find((candidate) => candidate.line === line);
        if (!entry) return false;
        for (const mark of entry.marks) actions.toggleLens(lensKey(mark));
        return true;
      },
    },
  });

  return [
    findingTheme,
    // Before the line numbers, where an editor shows its breakpoints.
    Prec.high(findingGutter),
    decorations,
    EditorView.editorAttributes.of({ class: dimUnread ? "cm-dimUnread" : "" }),
  ];
}

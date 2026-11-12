const SVG = "http://www.w3.org/2000/svg";

/** A 24-unit outline icon as an SVG element, for the DOM that CodeMirror owns (panels, gutters, widgets). */
export function svgIcon(paths: readonly string[], className: string): SVGSVGElement {
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("class", className);
  for (const d of paths) {
    const path = document.createElementNS(SVG, "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}

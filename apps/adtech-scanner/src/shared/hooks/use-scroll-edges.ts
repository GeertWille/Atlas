import { useCallback, useEffect, useRef, useState } from "react";

export type ScrollEdges = { start: boolean; end: boolean };

/**
 * Whether there is more of a horizontally scrolling element to either side of what shows. Overlay
 * scrollbars are invisible until used, so a scroller says with a shadow that it goes on.
 */
export function useScrollEdges<T extends HTMLElement>() {
  const element = useRef<T | null>(null);
  const [edges, setEdges] = useState<ScrollEdges>({ start: false, end: false });

  const measure = useCallback(() => {
    const node = element.current;
    if (!node) return;
    const start = node.scrollLeft > 1;
    const end = node.scrollLeft + node.clientWidth < node.scrollWidth - 1;
    setEdges((current) => (current.start === start && current.end === end ? current : { start, end }));
  }, []);

  useEffect(() => {
    const node = element.current;
    if (!node) return;
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    for (const child of node.children) observer.observe(child);
    node.addEventListener("scroll", measure, { passive: true });
    return () => {
      observer.disconnect();
      node.removeEventListener("scroll", measure);
    };
  }, [measure]);

  return [element, edges] as const;
}

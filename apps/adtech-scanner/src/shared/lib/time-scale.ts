export type TimeScale = {
  /** Upper bound of the axis in ms; the lower bound is always 0. */
  domainMs: number;
  ticks: number[];
  /** Position of a time on the axis as a 0-100 percentage, clamped to the axis. */
  toPercent(ms: number): number;
};

const NICE_STEPS = [1, 2, 2.5, 5, 10];

/** Smallest "nice" step (1, 2, 2.5, 5 × 10ⁿ) that yields at most `maxTicks` intervals. */
export function niceStep(domainMs: number, maxTicks: number): number {
  const rough = domainMs / Math.max(1, maxTicks);
  const magnitude = 10 ** Math.floor(Math.log10(rough));
  const factor = NICE_STEPS.find((step) => step * magnitude >= rough) ?? 10;
  return factor * magnitude;
}

/**
 * Linear axis from 0 to just past `maxMs`, extended to a whole number of nice
 * steps so the last tick sits on the right edge.
 */
export function createTimeScale(maxMs: number, maxTicks = 6): TimeScale {
  const safeMax = maxMs > 0 ? maxMs : 1;
  const step = niceStep(safeMax, maxTicks);
  const domainMs = Math.ceil(safeMax / step) * step;
  const ticks: number[] = [];
  for (let tick = 0; tick <= domainMs + step / 1000; tick += step) ticks.push(Math.round(tick));
  return {
    domainMs,
    ticks,
    toPercent: (ms) => Math.min(100, Math.max(0, (ms / domainMs) * 100)),
  };
}

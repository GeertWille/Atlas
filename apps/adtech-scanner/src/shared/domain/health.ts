import type { Tone } from "@/shared/ui/badge";

export type HealthBand = { label: string; tone: Extract<Tone, "green" | "amber" | "orange" | "red"> };

export function healthBand(score: number): HealthBand {
  if (score >= 80) return { label: "Healthy", tone: "green" };
  if (score >= 60) return { label: "Needs attention", tone: "amber" };
  if (score >= 40) return { label: "At risk", tone: "orange" };
  return { label: "Leaking revenue", tone: "red" };
}

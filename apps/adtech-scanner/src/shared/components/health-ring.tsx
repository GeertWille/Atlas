import { healthBand } from "@/shared/domain/health";
import { cn } from "@/shared/lib/cn";

const STROKE = { green: "stroke-green", amber: "stroke-amber", orange: "stroke-orange", red: "stroke-red" } as const;

/** Health score as a ring. The number is always printed, so colour is never the only cue. */
export function HealthRing({
  score,
  size = 64,
  className,
}: {
  score: number | null;
  size?: number;
  className?: string;
}) {
  const strokeWidth = size >= 56 ? 6 : 4;
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const band = score === null ? null : healthBand(score);
  return (
    <div
      role="img"
      aria-label={score === null ? "No health score" : `Health score ${score} of 100: ${band?.label}`}
      className={cn("relative shrink-0", className)}
      style={{ width: size, height: size }}
    >
      <svg width={size} height={size} className="-rotate-90" aria-hidden="true">
        <circle cx={size / 2} cy={size / 2} r={radius} fill="none" strokeWidth={strokeWidth} className="stroke-muted" />
        {score !== null && band ? (
          <circle
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            strokeWidth={strokeWidth}
            strokeLinecap="round"
            strokeDasharray={circumference}
            strokeDashoffset={circumference * (1 - score / 100)}
            className={cn("transition-[stroke-dashoffset] duration-700 ease-out", STROKE[band.tone])}
          />
        ) : null}
      </svg>
      <span
        className={cn(
          "absolute inset-0 flex items-center justify-center font-semibold tabular-nums",
          size >= 56 ? "text-lg" : "text-[11px]",
          score === null && "text-subtle-foreground",
        )}
      >
        {score ?? "–"}
      </span>
    </div>
  );
}

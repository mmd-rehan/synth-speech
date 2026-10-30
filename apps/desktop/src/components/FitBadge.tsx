import { cn } from "@/lib/utils";
import { verdictClasses, verdictLabel } from "@/lib/fit";
import type { Verdict } from "@/bindings";

export function FitBadge({ verdict, className }: { verdict: Verdict; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium",
        verdictClasses(verdict),
        className,
      )}
    >
      {verdictLabel(verdict)}
    </span>
  );
}

/** Quality tier shown as filled/empty dots (1–5). */
export function QualityStars({ tier }: { tier: number }) {
  return (
    <span className="inline-flex gap-0.5" aria-label={`quality ${tier} of 5`}>
      {[1, 2, 3, 4, 5].map((n) => (
        <span
          key={n}
          className={cn(
            "h-1.5 w-1.5 rounded-full",
            n <= tier ? "bg-primary" : "bg-muted",
          )}
        />
      ))}
    </span>
  );
}

import type { Verdict } from "@/bindings";

/** Display label for a verdict. */
export function verdictLabel(v: Verdict): string {
  switch (v) {
    case "excellent":
      return "Excellent";
    case "good":
      return "Good";
    case "slow":
      return "Slow";
    case "risky":
      return "Risky";
    case "not_recommended":
      return "Not recommended";
  }
}

/** Tailwind classes for a verdict badge. */
export function verdictClasses(v: Verdict): string {
  switch (v) {
    case "excellent":
      return "bg-green-600 text-white";
    case "good":
      return "bg-green-500/15 text-green-700 dark:text-green-400 border border-green-500/30";
    case "slow":
      return "bg-yellow-500/15 text-yellow-700 dark:text-yellow-400 border border-yellow-500/30";
    case "risky":
      return "bg-orange-500/15 text-orange-700 dark:text-orange-400 border border-orange-500/30";
    case "not_recommended":
      return "bg-destructive/15 text-destructive border border-destructive/30";
  }
}

/** Higher is better; used for sorting (recommended-ish first). */
export function verdictRank(v: Verdict): number {
  switch (v) {
    case "excellent":
      return 4;
    case "good":
      return 3;
    case "slow":
      return 2;
    case "risky":
      return 1;
    case "not_recommended":
      return 0;
  }
}

/** Short speed label from an estimated real-time factor. */
export function speedLabel(rtf: number | null | undefined): string {
  if (rtf == null || !Number.isFinite(rtf) || rtf <= 0) return "—";
  const factor = 1 / rtf;
  if (factor >= 1.5) return `~${Math.round(factor)}× realtime`;
  if (factor >= 0.9) return "~realtime";
  return `~${rtf.toFixed(1)}× slower`;
}

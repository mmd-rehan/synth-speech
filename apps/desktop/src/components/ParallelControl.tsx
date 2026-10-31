import { commands, type GovernorStatus } from "@/bindings";
import { useSettings } from "@/lib/settings";
import { gaugeMessage } from "@/lib/jobs";
import { cn } from "@/lib/utils";

const LEVEL_STYLE = {
  safe: "bg-green-600",
  tight: "bg-yellow-500",
  too_many: "bg-destructive",
} as const;

const LEVEL_LABEL = { safe: "Safe", tight: "Tight", too_many: "Too many" };

/** The "Parallel jobs" control: Auto/Manual, the limit, and the gauge. */
export function ParallelControl({ status }: { status: GovernorStatus }) {
  const manual = status.mode === "manual";
  const limit = status.parallel_limit;
  const gauge = status.gauge;
  const remember = useSettings((s) => s.update);

  function choose(mode: GovernorStatus["mode"], max: number) {
    void commands.governorSetParallel(mode, max);
    void remember({ parallel_mode: mode, max_parallel_jobs: max });
  }

  return (
    <section className="rounded-lg border p-4">
      <div className="flex flex-wrap items-center gap-3">
        <h2 className="text-sm font-semibold">Parallel jobs</h2>
        <div role="group" aria-label="Mode" className="flex rounded-md border">
          {(["auto", "manual"] as const).map((mode) => (
            <button
              key={mode}
              aria-pressed={status.mode === mode}
              className={cn(
                "px-3 py-1 text-xs capitalize",
                status.mode === mode && "bg-accent font-medium",
              )}
              onClick={() => choose(mode, limit)}
            >
              {mode}
            </button>
          ))}
        </div>
        <label className="flex items-center gap-2 text-sm">
          <span className="text-muted-foreground">
            {manual ? "Max at once" : "Now running up to"}
          </span>
          <input
            type="number"
            min={1}
            disabled={!manual}
            className="h-8 w-16 rounded-md border bg-background px-2 text-sm disabled:opacity-60"
            value={limit}
            onChange={(e) => {
              const next = Number(e.target.value);
              if (next >= 1) choose("manual", next);
            }}
          />
        </label>
        {gauge && (
          <span className="flex items-center gap-2 text-sm">
            <span
              aria-hidden
              className={cn("h-2.5 w-2.5 rounded-full", LEVEL_STYLE[gauge.level])}
            />
            {LEVEL_LABEL[gauge.level]}
          </span>
        )}
      </div>
      {gauge && (
        <p className="mt-2 text-sm text-muted-foreground">
          {gaugeMessage(gauge.level, limit, gauge.max_safe)}
        </p>
      )}
    </section>
  );
}

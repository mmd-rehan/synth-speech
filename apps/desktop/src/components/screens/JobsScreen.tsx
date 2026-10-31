import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronRight,
  FileText,
  Pause,
  Play,
  RotateCcw,
  Trash2,
  X,
} from "lucide-react";

import { AddFiles } from "@/components/AddFiles";
import { ErrorNotice } from "@/components/ErrorNotice";
import { ParallelControl } from "@/components/ParallelControl";
import { Button } from "@/components/ui/button";
import { commands, type Job, type Priority } from "@/bindings";
import { failureAction } from "@/lib/errors";
import {
  fileName,
  formatDuration,
  formatTimestamp,
  groupJobs,
  useJobs,
} from "@/lib/jobs";

const STATE_LABEL: Record<Job["state"], string> = {
  queued: "Waiting",
  starting: "Starting",
  running: "Running",
  paused: "Paused",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
};

/** The live text stream under a row. */
function Transcript({ job }: { job: Job }) {
  const segments = useJobs((s) => s.segments[job.id]);
  const loadSegments = useJobs((s) => s.loadSegments);

  useEffect(() => {
    void loadSegments(job.id);
  }, [job.id, loadSegments]);

  return (
    <div className="border-t bg-muted/30 px-4 py-3">
      {segments && segments.length > 0 ? (
        <ol className="max-h-64 space-y-1 overflow-y-auto text-sm">
          {segments.map((s) => (
            <li key={s.idx} className="flex gap-3">
              <span className="w-14 shrink-0 font-mono text-xs leading-5 text-muted-foreground">
                {formatTimestamp(s.start_ms ?? 0)}
              </span>
              <span>{s.text}</span>
            </li>
          ))}
        </ol>
      ) : (
        <p className="text-sm text-muted-foreground">No text yet.</p>
      )}
    </div>
  );
}

function JobRow({
  job,
  onOpen,
  onNeedModel,
}: {
  job: Job;
  onOpen?: (id: string) => void;
  onNeedModel?: () => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const eta = useJobs((s) => s.etaSecs[job.id]);
  const live = job.state === "running" || job.state === "starting";
  const pct = Math.round((job.progress ?? 0) * 100);

  async function act(run: () => Promise<{ status: string; error?: string }>) {
    const res = await run();
    setError(res.status === "ok" ? null : (res.error ?? "That did not work."));
  }

  const name = fileName(job.input_path);
  const elapsed =
    job.started_at_ms != null && job.finished_at_ms != null
      ? (job.finished_at_ms - job.started_at_ms) / 1000
      : null;
  const action = failureAction(job.error_code);
  const detail =
    job.state === "completed" && elapsed != null
      ? `Took ${formatDuration(elapsed)}`
      : job.state === "paused" && job.pause_origin === "governor"
        ? "Paused to protect your computer"
        : job.state === "queued"
          ? job.wait_reason
          : job.state === "running" && eta != null
            ? `About ${formatDuration(eta)} left`
            : null;

  return (
    <li className="rounded-lg border">
      <div className="flex items-center gap-2 p-3">
        <Button
          size="icon"
          variant="ghost"
          aria-label={open ? `Hide transcript of ${name}` : `Show transcript of ${name}`}
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          {open ? <ChevronDown /> : <ChevronRight />}
        </Button>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium" title={job.input_path}>
            {name}
          </p>
          <p className="truncate text-xs text-muted-foreground">
            {STATE_LABEL[job.state]} · {job.model_id}
            {detail ? ` · ${detail}` : ""}
          </p>
          {(live || job.state === "paused") && (
            <div
              role="progressbar"
              aria-label={`Progress of ${name}`}
              aria-valuenow={pct}
              aria-valuemin={0}
              aria-valuemax={100}
              className="mt-2 h-1.5 overflow-hidden rounded-full bg-secondary"
            >
              <div className="h-full bg-primary" style={{ width: `${pct}%` }} />
            </div>
          )}
        </div>

        {(job.progress ?? 0) > 0 && (
          <Button
            size="icon"
            variant="ghost"
            aria-label={`Open transcript of ${name}`}
            onClick={() => onOpen?.(job.id)}
          >
            <FileText />
          </Button>
        )}
        {(job.state === "queued" || job.state === "paused") && (
          <select
            aria-label={`Priority of ${name}`}
            className="h-8 rounded-md border bg-background px-1 text-xs"
            value={job.priority}
            onChange={(e) =>
              void act(() =>
                commands.jobsSetPriority(job.id, e.target.value as Priority),
              )
            }
          >
            <option value="high">High</option>
            <option value="normal">Normal</option>
            <option value="low">Low</option>
          </select>
        )}
        {(live || job.state === "queued") && (
          <Button
            size="icon"
            variant="ghost"
            aria-label={`Pause ${name}`}
            onClick={() => void act(() => commands.jobsPause(job.id))}
          >
            <Pause />
          </Button>
        )}
        {job.state === "paused" && (
          <Button
            size="icon"
            variant="ghost"
            aria-label={`Resume ${name}`}
            onClick={() => void act(() => commands.jobsResume(job.id))}
          >
            <Play />
          </Button>
        )}
        {(job.state === "failed" || job.state === "cancelled") && (
          <Button
            size="icon"
            variant="ghost"
            aria-label={`Retry ${name}`}
            onClick={() => void act(() => commands.jobsRetry(job.id))}
          >
            <RotateCcw />
          </Button>
        )}
        {(live || job.state === "queued" || job.state === "paused") && (
          <Button
            size="icon"
            variant="ghost"
            aria-label={`Cancel ${name}`}
            onClick={() => void act(() => commands.jobsCancel(job.id))}
          >
            <X />
          </Button>
        )}
        <Button
          size="icon"
          variant="ghost"
          aria-label={`Delete ${name}`}
          onClick={() => void act(() => commands.jobsDelete(job.id))}
        >
          <Trash2 />
        </Button>
      </div>
      {job.state === "failed" && (
        <div className="px-3 pb-3">
          <ErrorNotice
            message={job.error_message ?? t("error.unknown")}
            actionLabel={
              action === "smaller_model"
                ? t("error.smallerModel")
                : action === "models"
                  ? t("error.models")
                  : t("error.retry")
            }
            onAction={
              action === "retry"
                ? () => void act(() => commands.jobsRetry(job.id))
                : onNeedModel
            }
          />
        </div>
      )}
      {error && (
        <div className="px-3 pb-3">
          <ErrorNotice message={error} />
        </div>
      )}
      {open && <Transcript job={job} />}
    </li>
  );
}

export function JobsScreen({
  onNeedModel,
  onOpenJob,
}: {
  onNeedModel?: () => void;
  onOpenJob?: (id: string) => void;
}) {
  const byId = useJobs((s) => s.byId);
  const status = useJobs((s) => s.status);
  const warnings = useJobs((s) => s.warnings);
  const dismissWarning = useJobs((s) => s.dismissWarning);
  const refresh = useJobs((s) => s.refresh);

  // Rows disappear on delete, for which there is no event.
  useEffect(() => {
    void refresh().catch(() => {});
  }, [refresh]);

  const groups = useMemo(() => groupJobs(Object.values(byId)), [byId]);
  const hasJobs = groups.length > 0;
  const hasCompleted = groups.some((g) => g.state === "completed");

  async function clearCompleted() {
    await commands.jobsClearCompleted();
    await refresh();
  }

  return (
    <div className="mx-auto max-w-4xl space-y-4 px-8 py-10">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="text-2xl font-semibold tracking-tight">Jobs</h1>
        <AddFiles onNeedModel={onNeedModel} />
      </div>

      {status && <ParallelControl status={status} />}

      {warnings.map((w) => (
        <div
          key={w.at_ms}
          role="status"
          className="flex items-center justify-between gap-3 rounded-lg border border-yellow-500/50 bg-yellow-500/10 px-4 py-2 text-sm"
        >
          {w.message}
          <Button
            size="icon"
            variant="ghost"
            aria-label="Dismiss"
            onClick={() => dismissWarning(w.at_ms ?? 0)}
          >
            <X />
          </Button>
        </div>
      ))}

      {hasJobs && (
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={() => void commands.jobsPauseAll()}
          >
            Pause all
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void commands.jobsResumeAll()}
          >
            Resume all
          </Button>
          {hasCompleted && (
            <Button size="sm" variant="outline" onClick={() => void clearCompleted()}>
              Clear completed
            </Button>
          )}
          {status?.queue_eta_secs != null && (
            <span className="ml-auto text-sm text-muted-foreground">
              Queue will finish in about {formatDuration(status.queue_eta_secs)}
            </span>
          )}
        </div>
      )}

      {!hasJobs && (
        <p className="rounded-lg border border-dashed p-10 text-center text-sm text-muted-foreground">
          No jobs yet. Add audio or video files to start.
        </p>
      )}

      {groups.map((group) => (
        <section key={group.state}>
          <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {STATE_LABEL[group.state]} ({group.jobs.length})
          </h2>
          <ul className="space-y-2">
            {group.jobs.map((job) => (
              <JobRow
                key={job.id}
                job={job}
                onOpen={onOpenJob}
                onNeedModel={onNeedModel}
              />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

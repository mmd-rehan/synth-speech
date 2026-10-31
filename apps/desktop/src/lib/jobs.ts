import { create } from "zustand";

import {
  commands,
  events,
  type GovernorStatus,
  type GovernorWarning,
  type Job,
  type JobSegment,
} from "@/bindings";

const MAX_WARNINGS = 5;
const STATUS_POLL_MS = 1000;

interface JobsStore {
  byId: Record<string, Job>;
  segments: Record<string, JobSegment[]>;
  etaSecs: Record<string, number | null>;
  stage: Record<string, string>;
  status: GovernorStatus | null;
  warnings: GovernorWarning[];
  init: () => void;
  refresh: () => Promise<void>;
  loadSegments: (id: string) => Promise<void>;
  dismissWarning: (atMs: number) => void;
}

let started = false;

export const useJobs = create<JobsStore>((set, get) => ({
  byId: {},
  segments: {},
  etaSecs: {},
  stage: {},
  status: null,
  warnings: [],

  init: () => {
    if (started) return;
    started = true;

    get()
      .refresh()
      .catch(() => {});

    events.jobStateChanged
      .listen((e) => {
        const job = e.payload.job as Job;
        set((s) => ({ byId: { ...s.byId, [job.id]: job } }));
      })
      .catch(() => {});

    events.jobProgressChanged
      .listen((e) => {
        const p = e.payload.progress;
        set((s) => {
          const job = s.byId[p.job_id];
          return {
            byId: job
              ? { ...s.byId, [p.job_id]: { ...job, progress: p.progress } }
              : s.byId,
            etaSecs: { ...s.etaSecs, [p.job_id]: p.eta_secs ?? null },
            stage: { ...s.stage, [p.job_id]: p.stage },
          };
        });
      })
      .catch(() => {});

    events.jobSegmentsAdded
      .listen((e) => {
        const { job_id, segments } = e.payload;
        set((s) => {
          const known = s.segments[job_id];
          // Until the transcript is opened there is nothing to append to; it
          // is loaded whole from the database at that point.
          if (!known) return s;
          return {
            segments: { ...s.segments, [job_id]: [...known, ...segments] },
          };
        });
      })
      .catch(() => {});

    events.governorWarned
      .listen((e) => {
        const warning = e.payload.warning;
        set((s) => ({
          warnings: [warning, ...s.warnings].slice(0, MAX_WARNINGS),
        }));
      })
      .catch(() => {});

    // The gauge and ETA depend on live memory, so they are polled.
    setInterval(() => {
      commands
        .governorGetStatus()
        .then((status) => set({ status }))
        .catch(() => {});
    }, STATUS_POLL_MS);
  },

  refresh: async () => {
    const [jobs, status] = await Promise.all([
      commands.jobsList(),
      commands.governorGetStatus(),
    ]);
    set({
      byId: Object.fromEntries(jobs.map((j) => [j.id, j])),
      status,
    });
  },

  loadSegments: async (id) => {
    const res = await commands.jobsSegments(id);
    if (res.status === "ok") {
      set((s) => ({ segments: { ...s.segments, [id]: res.data } }));
    }
  },

  dismissWarning: (atMs) =>
    set((s) => ({ warnings: s.warnings.filter((w) => w.at_ms !== atMs) })),
}));

const GROUP_ORDER: Job["state"][] = [
  "running",
  "starting",
  "paused",
  "queued",
  "failed",
  "cancelled",
  "completed",
];

/** Jobs grouped by state, active work first. */
export function groupJobs(jobs: Job[]): { state: Job["state"]; jobs: Job[] }[] {
  return GROUP_ORDER.map((state) => ({
    state,
    jobs: jobs.filter((j) => j.state === state).sort(compareQueue),
  })).filter((group) => group.jobs.length > 0);
}

const PRIORITY_WEIGHT: Record<Job["priority"], number> = {
  high: 0,
  normal: 1,
  low: 2,
};

function compareQueue(a: Job, b: Job): number {
  return (
    PRIORITY_WEIGHT[a.priority] - PRIORITY_WEIGHT[b.priority] ||
    (a.rank ?? 0) - (b.rank ?? 0)
  );
}

/** The file name from a path on any platform. */
export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

/** Seconds as "42 s", "3 min", or "1 h 5 min". */
export function formatDuration(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs)) return "—";
  if (secs < 60) return `${Math.max(1, Math.round(secs))} s`;
  const minutes = Math.round(secs / 60);
  if (minutes < 60) return `${minutes} min`;
  const rest = minutes % 60;
  return rest
    ? `${Math.floor(minutes / 60)} h ${rest} min`
    : `${Math.floor(minutes / 60)} h`;
}

/** A transcript timestamp as m:ss or h:mm:ss. */
export function formatTimestamp(ms: number): string {
  const total = Math.floor(ms / 1000);
  const seconds = String(total % 60).padStart(2, "0");
  const minutes = Math.floor(total / 60) % 60;
  const hours = Math.floor(total / 3600);
  return hours
    ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}`
    : `${minutes}:${seconds}`;
}

/** The sentence for a gauge reading (doc 06). */
export function gaugeMessage(
  level: "safe" | "tight" | "too_many",
  jobs: number,
  maxSafe: number,
): string {
  switch (level) {
    case "safe":
      return `Your machine can run ${jobs} ${jobs === 1 ? "job" : "jobs"} at once.`;
    case "tight":
      return `${jobs} jobs will work but expect other apps to slow down.`;
    case "too_many":
      return `Running ${jobs} jobs at once will likely freeze or crash your computer. Max safe is ${maxSafe}.`;
  }
}

import { create } from "zustand";

import {
  commands,
  events,
  type GovernorStatus,
  type GovernorWarning,
  type Job,
  type JobProgress,
  type JobSegment,
  type JobSegmentsAdded,
} from "@/bindings";

const MAX_WARNINGS = 5;
const STATUS_POLL_MS = 1000;
/** Doc 07: event-driven updates reach the store at most 10 times a second. */
const STORE_FLUSH_MS = 100;

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
  setSegmentText: (id: string, idx: number, text: string) => void;
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

    events.jobRemoved
      .listen((e) => {
        const { job_id } = e.payload;
        set((s) => {
          const byId = { ...s.byId };
          delete byId[job_id];
          return { byId };
        });
      })
      .catch(() => {});

    // Live updates are collected and applied together, so many busy jobs
    // cause one render per flush rather than one per event.
    let pendingProgress: JobProgress[] = [];
    let pendingSegments: JobSegmentsAdded[] = [];
    let flushTimer: ReturnType<typeof setTimeout> | null = null;

    const flush = () => {
      flushTimer = null;
      const progress = pendingProgress;
      const added = pendingSegments;
      pendingProgress = [];
      pendingSegments = [];
      set((s) => {
        const byId = { ...s.byId };
        const etaSecs = { ...s.etaSecs };
        const stage = { ...s.stage };
        for (const p of progress) {
          const job = byId[p.job_id];
          if (job) byId[p.job_id] = { ...job, progress: p.progress };
          etaSecs[p.job_id] = p.eta_secs ?? null;
          stage[p.job_id] = p.stage;
        }
        const segments = { ...s.segments };
        for (const { job_id, segments: lines } of added) {
          // Until a transcript is opened there is nothing to append to; it is
          // loaded whole from the database at that point.
          const known = segments[job_id];
          if (known) segments[job_id] = [...known, ...lines];
        }
        return { byId, etaSecs, stage, segments };
      });
    };
    const schedule = () => {
      flushTimer ??= setTimeout(flush, STORE_FLUSH_MS);
    };

    events.jobProgressChanged
      .listen((e) => {
        pendingProgress.push(e.payload.progress as JobProgress);
        schedule();
      })
      .catch(() => {});

    events.jobSegmentsAdded
      .listen((e) => {
        pendingSegments.push(e.payload);
        schedule();
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

  setSegmentText: (id, idx, text) =>
    set((s) => ({
      segments: {
        ...s.segments,
        [id]: (s.segments[id] ?? []).map((line) =>
          line.idx === idx ? { ...line, text } : line,
        ),
      },
    })),
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

/** Time to process `durations` seconds, or null if any length is unknown. */
export function estimateSecs(
  durations: (number | null)[],
  rtf: number | null | undefined,
): number | null {
  if (rtf == null || durations.length === 0) return null;
  if (durations.some((d) => d == null)) return null;
  return durations.reduce<number>((sum, d) => sum + (d ?? 0) * rtf, 0);
}

/** Lines matching a search, case-insensitively. */
export function filterSegments(
  segments: JobSegment[],
  query: string,
): JobSegment[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return segments;
  return segments.filter((s) => s.text.toLowerCase().includes(needle));
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

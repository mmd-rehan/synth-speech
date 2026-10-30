import { create } from "zustand";

import { events } from "@/bindings";

export interface DlState {
  state: string; // downloading | paused | completed | cancelled | failed
  downloaded?: number;
  total?: number;
  speedBps?: number;
  etaSecs?: number | null;
  error?: string;
}

interface DownloadsStore {
  byId: Record<string, DlState>;
  init: () => void;
}

let started = false;

export const useDownloads = create<DownloadsStore>((set) => ({
  byId: {},
  init: () => {
    if (started) return;
    started = true;

    events.downloadProgress
      .listen((e) => {
        const p = e.payload;
        set((s) => ({
          byId: {
            ...s.byId,
            [p.id]: {
              state: s.byId[p.id]?.state ?? "downloading",
              downloaded: p.downloaded ?? 0,
              total: p.total ?? 0,
              speedBps: p.speed_bps ?? 0,
              etaSecs: p.eta_secs,
            },
          },
        }));
      })
      .catch(() => {});

    events.downloadState
      .listen((e) => {
        const p = e.payload;
        set((s) => ({
          byId: {
            ...s.byId,
            [p.id]: {
              ...s.byId[p.id],
              state: p.state,
              error: p.error_message ?? undefined,
            },
          },
        }));
      })
      .catch(() => {});
  },
}));

/** Download progress as a 0–100 percentage, or null if unknown. */
export function downloadPct(d: DlState | undefined): number | null {
  if (!d || !d.total || d.total <= 0 || d.downloaded == null) return null;
  return Math.min(100, (d.downloaded / d.total) * 100);
}

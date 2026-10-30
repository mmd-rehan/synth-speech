import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";

import { commands, type UsageSample } from "@/bindings";

const MAX_SAMPLES = 120;

interface UsageState {
  history: UsageSample[];
  latest: UsageSample | null;
  push: (sample: UsageSample) => void;
  /** Fetch the backlog and subscribe to live samples (idempotent). */
  init: () => void;
}

let started = false;

export const useUsage = create<UsageState>((set, get) => ({
  history: [],
  latest: null,
  push: (sample) =>
    set((state) => ({
      history: [...state.history, sample].slice(-MAX_SAMPLES),
      latest: sample,
    })),
  init: () => {
    if (started) return;
    started = true;

    commands
      .hardwareUsageHistory()
      .then((history) =>
        set({ history, latest: history[history.length - 1] ?? null }),
      )
      .catch(() => {
        // Not running under Tauri (e.g. plain browser); stay empty.
      });

    listen<UsageSample>("hardware:usage", (event) => {
      get().push(event.payload);
    }).catch(() => {
      // Event system unavailable outside Tauri.
    });
  },
}));

/** Memory used as a 0–100 fraction of (used + available). */
export function ramUsedPct(sample: UsageSample | null): number {
  if (!sample) return 0;
  // Numeric fields are `number | null` in the generated types (floats can be
  // null in JSON), so coalesce defensively.
  const used = sample.ram_used_bytes ?? 0;
  const available = sample.ram_available_bytes ?? 0;
  const total = used + available;
  return total > 0 ? (used / total) * 100 : 0;
}

// Stands in for the Rust core: answers IPC calls and emits events, so the real
// UI runs in a real browser. Injected into the page, so it cannot import.

export interface MockOptions {
  models: unknown[];
  firstRunDone: boolean;
  installed: string[];
  segmentCount: number;
}

export function installMockTauri(options: MockOptions) {
  type Job = Record<string, unknown> & { id: string };
  const callbacks = new Map<number, (message: unknown) => void>();
  const listeners = new Map<string, number[]>();
  let nextCallback = 1;
  const state = {
    settings: {
      theme: "light",
      language: "en",
      first_run_done: options.firstRunDone,
      models_dir: null,
      parallel_mode: "auto",
      max_parallel_jobs: 2,
      thread_cap_per_job: 8,
      gpu_preference: "auto",
      pause_on_battery_below: 20,
      offline_mode: false,
      retention_days: 0,
    } as Record<string, unknown>,
    installed: options.installed.map((id) => ({
      id,
      file_name: "ggml.bin",
      size_bytes: 1,
      sha256: "ab",
      installed_at_ms: 1,
    })),
    jobs: [] as Job[],
    segments: new Map<string, unknown[]>(),
    exports: [] as unknown[],
  };

  function emit(event: string, payload: unknown) {
    for (const id of listeners.get(event) ?? []) {
      callbacks.get(id)?.({ event, id, payload });
    }
  }

  const fit = (id: string) => ({
    model_id: id,
    verdict: "good",
    rtf_est: 0.2,
    needed_bytes: 1,
    pool_bytes: 2,
    headroom_bytes: 1,
    accelerator: { kind: "cpu_neon" },
    measured: false,
    hard_blocked: false,
    summary: "A good fit.",
    reasons: [],
  });

  const lines = (count: number) =>
    Array.from({ length: count }, (_, idx) => ({
      idx,
      start_ms: idx * 2000,
      end_ms: idx * 2000 + 1900,
      text: `Transcript line number ${idx}`,
    }));

  const handlers: Record<string, (args: Record<string, never>) => unknown> = {
    app_version: () => ({ name: "Open Audio Studio", version: "0.1.0" }),
    settings_get: () => state.settings,
    settings_set: (a) => {
      state.settings = a.settings;
      return null;
    },
    hardware_get_profile: () => ({
      memory: { total_bytes: 17179869184 },
      cpu: { brand: "Apple M3 Pro" },
    }),
    hardware_usage_history: () => [],
    catalog_list: () => options.models,
    catalog_fit_all: () =>
      options.models.map((m) => fit((m as { id: string }).id)),
    catalog_recommend: () => "whisper-tiny",
    models_list_installed: () => state.installed,
    models_download: (a) => {
      const id = a.modelId as string;
      state.installed.push({
        id,
        file_name: "ggml.bin",
        size_bytes: 1,
        sha256: "ab",
        installed_at_ms: 1,
      });
      setTimeout(() => {
        emit("download-state", { id, state: "completed" });
      }, 10);
      return null;
    },
    jobs_list: () => state.jobs,
    jobs_probe_durations: (a) => (a.paths as string[]).map(() => 300),
    jobs_create: (a) => {
      const request = a.request as { paths: string[]; model_id: string };
      return request.paths.map((path, i) => {
        const id = `job-${state.jobs.length + 1}`;
        const job: Job = {
          id,
          model_id: request.model_id,
          input_path: path,
          duration_ms: 300000,
          priority: "normal",
          rank: state.jobs.length + 1,
          state: "running",
          pause_origin: null,
          wait_reason: null,
          progress: 0.1,
          created_at_ms: 1,
          started_at_ms: 1000,
          finished_at_ms: null,
          error_code: null,
          error_message: null,
          retries: 0,
          rtf: null,
          language: null,
          translate: false,
          word_timestamps: false,
        };
        state.jobs.push(job);
        state.segments.set(id, lines(options.segmentCount));
        setTimeout(
          () => {
            const done = {
              ...job,
              state: "completed",
              progress: 1,
              finished_at_ms: 61000,
            };
            state.jobs = state.jobs.map((j) => (j.id === id ? done : j));
            emit("job-state-changed", { job: done });
          },
          50 + i * 10,
        );
        return job;
      });
    },
    jobs_segments: (a) => state.segments.get(a.id as string) ?? [],
    jobs_allow_playback: (a) =>
      state.jobs.find((j) => j.id === a.id)?.input_path,
    jobs_update_segment: (a) => {
      const line = (
        state.segments.get(a.id as string) as { idx: number; text: string }[]
      ).find((l) => l.idx === a.idx);
      if (line) line.text = a.text as string;
      return null;
    },
    jobs_export: (a) => {
      state.exports.push(a);
      return null;
    },
    governor_get_status: () => ({
      mode: "auto",
      parallel_limit: 1,
      running: 0,
      queued: 0,
      paused: 0,
      mem_budget: 1,
      reserved_mem: 0,
      cpu_budget: 7,
      reserved_cpu: 0,
      gauge: null,
      queue_eta_secs: null,
    }),
    "plugin:dialog|open": () => ["/clips/team meeting é.wav"],
    "plugin:dialog|save": () => "/exports/team meeting.srt",
    "plugin:event|listen": (a) => {
      const handler = a.handler as number;
      const event = a.event as string;
      listeners.set(event, [...(listeners.get(event) ?? []), handler]);
      return handler;
    },
    "plugin:event|unlisten": () => null,
  };

  const w = window as unknown as Record<string, unknown>;
  w.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main", windowLabel: "main" },
    },
    transformCallback(callback: (message: unknown) => void) {
      const id = nextCallback++;
      callbacks.set(id, callback);
      return id;
    },
    convertFileSrc: (path: string) => `asset://localhost/${path}`,
    invoke: (command: string, args: Record<string, never>) => {
      const handler = handlers[command];
      if (!handler) {
        return Promise.reject(new Error(`mock has no command: ${command}`));
      }
      return Promise.resolve(handler(args ?? {}));
    },
  };
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  w.__mock = { state, emit };
}

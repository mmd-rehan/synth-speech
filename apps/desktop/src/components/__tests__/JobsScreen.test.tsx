import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const jobsPause = vi.fn();
const jobsRetry = vi.fn();
const governorSetParallel = vi.fn();
const jobsList = vi.fn();
const governorGetStatus = vi.fn();

vi.mock("@/bindings", () => ({
  commands: {
    jobsList: () => jobsList(),
    governorGetStatus: () => governorGetStatus(),
    jobsPause: (id: string) => jobsPause(id),
    jobsRetry: (id: string) => jobsRetry(id),
    governorSetParallel: (mode: string, max: number) =>
      governorSetParallel(mode, max),
    modelsListInstalled: () =>
      Promise.resolve({
        status: "ok",
        data: [{ id: "whisper-tiny" }],
      }),
  },
  events: {},
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

import { JobsScreen } from "@/components/screens/JobsScreen";
import { useJobs } from "@/lib/jobs";

const STATUS = {
  mode: "auto",
  parallel_limit: 2,
  running: 1,
  queued: 0,
  paused: 0,
  mem_budget: 1,
  reserved_mem: 0,
  cpu_budget: 7,
  reserved_cpu: 1,
  gauge: { level: "too_many", max_safe: 1 },
  queue_eta_secs: 150,
};

function job(id: string, state: string, extra: object = {}) {
  return {
    id,
    state,
    input_path: `/clips/${id}.wav`,
    model_id: "whisper-tiny",
    priority: "normal",
    rank: 1,
    progress: 0.5,
    ...extra,
  };
}

describe("JobsScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    governorGetStatus.mockResolvedValue(STATUS);
    jobsPause.mockResolvedValue({ status: "ok", data: null });
    jobsRetry.mockResolvedValue({ status: "ok", data: null });
    useJobs.setState({ warnings: [] });
  });

  it("invites the user to add files when there are no jobs", async () => {
    jobsList.mockResolvedValue([]);
    render(<JobsScreen />);
    expect(await screen.findByText(/no jobs yet/i)).toBeInTheDocument();
  });

  it("shows the gauge's warning and the queue estimate", async () => {
    jobsList.mockResolvedValue([job("a", "running")]);
    render(<JobsScreen />);
    expect(await screen.findByText(/Max safe is 1/)).toBeInTheDocument();
    expect(screen.getByText("Too many")).toBeInTheDocument();
    expect(
      screen.getByText(/Queue will finish in about 3 min/),
    ).toBeInTheDocument();
  });

  it("explains why a job waits, and that the governor paused another", async () => {
    jobsList.mockResolvedValue([
      job("a", "queued", { wait_reason: "Waiting: not enough free memory" }),
      job("b", "paused", { pause_origin: "governor" }),
    ]);
    render(<JobsScreen />);
    expect(
      await screen.findByText(/not enough free memory/),
    ).toBeInTheDocument();
    expect(screen.getByText(/protect your computer/)).toBeInTheDocument();
  });

  it("offers the right action for a job's state", async () => {
    jobsList.mockResolvedValue([
      job("run", "running"),
      job("bad", "failed", { error_message: "Could not read the audio" }),
    ]);
    const user = userEvent.setup();
    render(<JobsScreen />);

    await user.click(await screen.findByRole("button", { name: "Pause run.wav" }));
    expect(jobsPause).toHaveBeenCalledWith("run");
    await user.click(screen.getByRole("button", { name: "Retry bad.wav" }));
    expect(jobsRetry).toHaveBeenCalledWith("bad");
    expect(screen.getByText(/Could not read the audio/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Retry run.wav" })).toBeNull();
  });

  it("switches to manual mode", async () => {
    jobsList.mockResolvedValue([]);
    const user = userEvent.setup();
    render(<JobsScreen />);
    await user.click(await screen.findByRole("button", { name: "manual" }));
    expect(governorSetParallel).toHaveBeenCalledWith("manual", 2);
  });
});

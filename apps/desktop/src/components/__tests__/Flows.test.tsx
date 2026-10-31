import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const c = vi.hoisted(() => ({
  hardwareGetProfile: vi.fn(),
  catalogRecommend: vi.fn(),
  catalogList: vi.fn(),
  catalogFitAll: vi.fn(),
  modelsDownload: vi.fn(),
  jobsCreate: vi.fn(),
  jobsProbeDurations: vi.fn(),
  settingsGet: vi.fn(),
  settingsSet: vi.fn(),
  settingsMoveModels: vi.fn(),
  spawnWorkerProbe: vi.fn(),
  jobsPauseAll: vi.fn(),
  appVersion: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: c, events: {} }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { AddJobDialog } from "@/components/AddJobDialog";
import { CommandPalette } from "@/components/CommandPalette";
import { FirstRunWizard } from "@/components/FirstRunWizard";
import { SettingsScreen } from "@/components/screens/SettingsScreen";
import { useSettings } from "@/lib/settings";
import { useTheme } from "@/lib/theme";

const TINY = { id: "whisper-tiny", name: "Whisper Tiny", weights_bytes: 77691713 };
const SETTINGS = {
  theme: "system",
  language: "en",
  first_run_done: false,
  models_dir: null,
  parallel_mode: "auto",
  max_parallel_jobs: 2,
  thread_cap_per_job: 8,
  gpu_preference: "auto",
  pause_on_battery_below: 20,
  offline_mode: false,
  retention_days: 0,
};

beforeEach(() => {
  vi.clearAllMocks();
  HTMLDialogElement.prototype.showModal = function showModal() {
    this.open = true;
  };
  HTMLDialogElement.prototype.close = function close() {
    this.open = false;
  };
  c.catalogList.mockResolvedValue([TINY]);
  c.catalogRecommend.mockResolvedValue({ status: "ok", data: "whisper-tiny" });
  c.catalogFitAll.mockResolvedValue({
    status: "ok",
    data: [{ model_id: "whisper-tiny", verdict: "good", rtf_est: 0.5 }],
  });
  c.hardwareGetProfile.mockResolvedValue({
    status: "ok",
    data: {
      memory: { total_bytes: 16 * 1024 ** 3 },
      cpu: { brand: "Test Chip" },
    },
  });
  c.jobsProbeDurations.mockResolvedValue([120, 240]);
  c.settingsSet.mockResolvedValue({ status: "ok", data: null });
  c.appVersion.mockResolvedValue({ name: "Open Audio Studio", version: "1.2.3" });
  useSettings.setState({ settings: { ...SETTINGS } });
});

describe("FirstRunWizard", () => {
  it("walks through privacy, the scan, and a one click download", async () => {
    c.modelsDownload.mockResolvedValue({ status: "ok", data: null });
    const onDone = vi.fn();
    const user = userEvent.setup();
    render(<FirstRunWizard onDone={onDone} />);

    expect(screen.getByText(/nothing is uploaded/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Get started" }));
    expect(await screen.findByText(/16\.0 GB|16 GB/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Next" }));
    await user.click(
      await screen.findByRole("button", { name: /^Download/ }),
    );
    expect(c.modelsDownload).toHaveBeenCalledWith("whisper-tiny");
    expect(onDone).toHaveBeenCalledWith("models");
  });

  it("carries on when the scan fails, and shows a download error", async () => {
    c.hardwareGetProfile.mockResolvedValue({ status: "error", error: "no" });
    c.modelsDownload.mockResolvedValue({
      status: "error",
      error: "Offline mode is on",
    });
    const user = userEvent.setup();
    render(<FirstRunWizard onDone={() => {}} />);
    await user.click(screen.getByRole("button", { name: "Get started" }));
    await screen.findByText(/could not read/);
    await user.click(screen.getByRole("button", { name: "Next" }));
    await user.click(await screen.findByRole("button", { name: /^Download/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Offline mode");
  });
});

describe("AddJobDialog", () => {
  const props = {
    paths: ["/a/one.wav", "/a/two.wav"],
    installed: [{ id: "whisper-tiny" }] as never,
    onClose: vi.fn(),
  };

  it("shows the fit badge and a live time estimate", async () => {
    render(<AddJobDialog {...props} />);
    expect(await screen.findByText("Good")).toBeInTheDocument();
    // (120 + 240) s × 0.5 RTF
    expect(await screen.findByText(/about 3 min/)).toBeInTheDocument();
  });

  it("queues the files with the chosen options", async () => {
    c.jobsCreate.mockResolvedValue({ status: "ok", data: [] });
    const onAdded = vi.fn();
    const user = userEvent.setup();
    render(<AddJobDialog {...props} onAdded={onAdded} />);
    await user.selectOptions(screen.getByLabelText("Task"), "translate");
    await user.selectOptions(screen.getByLabelText("Language"), "ur");
    await user.click(screen.getByRole("button", { name: "Add to queue" }));
    expect(c.jobsCreate).toHaveBeenCalledWith({
      paths: props.paths,
      model_id: "whisper-tiny",
      translate: true,
      language: "ur",
    });
    expect(onAdded).toHaveBeenCalled();
  });

  it("keeps the dialog open and explains a failure", async () => {
    c.jobsCreate.mockResolvedValue({
      status: "error",
      error: "File not found: /a/one.wav",
    });
    const user = userEvent.setup();
    render(<AddJobDialog {...props} />);
    await user.click(screen.getByRole("button", { name: "Add to queue" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("File not found");
  });
});

describe("SettingsScreen", () => {
  it("saves a change straight away", async () => {
    const user = userEvent.setup();
    render(<SettingsScreen />);
    await user.click(screen.getByRole("checkbox", { name: /offline mode/i }));
    expect(c.settingsSet).toHaveBeenCalledWith(
      expect.objectContaining({ offline_mode: true }),
    );
  });

  it("applies the theme and rolls back a failed save", async () => {
    const user = userEvent.setup();
    render(<SettingsScreen />);
    await user.selectOptions(screen.getByLabelText("Theme"), "dark");
    expect(useTheme.getState().theme).toBe("dark");

    c.settingsSet.mockResolvedValue({ status: "error", error: "disk full" });
    fireEvent.change(screen.getByLabelText(/threads per job/i), {
      target: { value: "4" },
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("disk full");
    expect(useSettings.getState().settings?.thread_cap_per_job).toBe(8);
  });

  it("ignores a number outside the allowed range", async () => {
    render(<SettingsScreen />);
    fireEvent.change(screen.getByLabelText(/threads per job/i), {
      target: { value: "0" },
    });
    expect(c.settingsSet).not.toHaveBeenCalled();
  });

  it("shows the version, license, and where to get help", async () => {
    render(<SettingsScreen />);
    expect(await screen.findByText("Version 1.2.3")).toBeInTheDocument();
    expect(screen.getByText(/MIT license/)).toBeInTheDocument();
    expect(screen.getByText(/no audio and no transcripts/)).toBeInTheDocument();
  });

  it("checks the worker", async () => {
    c.spawnWorkerProbe.mockResolvedValue({ status: "ok", data: "0.1.0" });
    const user = userEvent.setup();
    render(<SettingsScreen />);
    await user.click(screen.getByRole("button", { name: "Check worker" }));
    expect(await screen.findByText(/worker ready/i)).toBeInTheDocument();
  });
});

describe("CommandPalette", () => {
  function openPalette() {
    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", { key: "k", ctrlKey: true }),
      );
    });
  }

  it("filters commands and runs the chosen one from the keyboard", async () => {
    const onNavigate = vi.fn();
    const user = userEvent.setup();
    render(<CommandPalette onNavigate={onNavigate} />);
    openPalette();
    await user.type(screen.getByRole("textbox"), "machine{Enter}");
    expect(onNavigate).toHaveBeenCalledWith("machine");
  });

  it("acts on the queue", async () => {
    const user = userEvent.setup();
    render(<CommandPalette onNavigate={() => {}} />);
    openPalette();
    await user.click(screen.getByRole("button", { name: "Pause all jobs" }));
    expect(c.jobsPauseAll).toHaveBeenCalled();
  });

  it("says so when nothing matches", async () => {
    const user = userEvent.setup();
    render(<CommandPalette onNavigate={() => {}} />);
    openPalette();
    await user.type(screen.getByRole("textbox"), "zzz");
    expect(screen.getByText(/no matching/i)).toBeInTheDocument();
  });
});

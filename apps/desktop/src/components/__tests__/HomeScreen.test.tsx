import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Mock the generated IPC bindings so the component can be tested without the
// Tauri runtime.
const appVersion = vi.fn();
const modelsListInstalled = vi.fn();
vi.mock("@/bindings", () => ({
  commands: {
    appVersion: () => appVersion(),
    modelsListInstalled: () => modelsListInstalled(),
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { HomeScreen } from "@/components/screens/HomeScreen";
import { useJobs } from "@/lib/jobs";

describe("HomeScreen", () => {
  beforeEach(() => {
    appVersion.mockResolvedValue({ name: "Open Audio Studio", version: "0.1.0" });
    modelsListInstalled.mockResolvedValue({
      status: "ok",
      data: [{ id: "whisper-tiny" }],
    });
    useJobs.setState({ byId: {} });
  });

  it("shows the app name and version from the backend", async () => {
    render(<HomeScreen />);
    expect(await screen.findByText(/Version 0\.1\.0/)).toBeInTheDocument();
  });

  it("sends a user with no model to choose one", async () => {
    modelsListInstalled.mockResolvedValue({ status: "ok", data: [] });
    const onNeedModel = vi.fn();
    render(<HomeScreen onNeedModel={onNeedModel} />);
    await userEvent.click(
      await screen.findByRole("button", { name: /choose a model/i }),
    );
    expect(onNeedModel).toHaveBeenCalled();
  });

  it("lists recent finished jobs, newest first, and opens them", async () => {
    useJobs.setState({
      byId: {
        a: { id: "a", state: "completed", input_path: "/x/old.wav", finished_at_ms: 1 },
        b: { id: "b", state: "completed", input_path: "/x/new.wav", finished_at_ms: 2 },
        c: { id: "c", state: "running", input_path: "/x/busy.wav" },
      } as never,
    });
    const onOpenJob = vi.fn();
    render(<HomeScreen onOpenJob={onOpenJob} />);
    const rows = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(rows).toEqual(["new.wav", "old.wav"]);
    await userEvent.click(screen.getByRole("button", { name: "new.wav" }));
    expect(onOpenJob).toHaveBeenCalledWith("b");
  });
});

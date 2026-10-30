import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Mock the generated IPC bindings so the component can be tested without the
// Tauri runtime.
const appVersion = vi.fn();
const spawnWorkerProbe = vi.fn();
vi.mock("@/bindings", () => ({
  commands: {
    appVersion: () => appVersion(),
    spawnWorkerProbe: () => spawnWorkerProbe(),
  },
}));

import { HomeScreen } from "@/components/screens/HomeScreen";

describe("HomeScreen", () => {
  beforeEach(() => {
    appVersion.mockReset();
    spawnWorkerProbe.mockReset();
    appVersion.mockResolvedValue({ name: "Open Audio Studio", version: "0.1.0" });
  });

  it("shows the app name and version from the backend", async () => {
    render(<HomeScreen />);
    expect(
      await screen.findByText(/Version 0\.1\.0/),
    ).toBeInTheDocument();
  });

  it("reports a ready worker on a successful probe", async () => {
    spawnWorkerProbe.mockResolvedValue({ status: "ok", data: "0.1.0" });
    render(<HomeScreen />);
    await userEvent.click(screen.getByRole("button", { name: /check worker/i }));
    expect(await screen.findByText(/worker ready/i)).toBeInTheDocument();
  });

  it("shows the error when the probe fails", async () => {
    spawnWorkerProbe.mockResolvedValue({
      status: "error",
      error: "could not run the worker sidecar",
    });
    render(<HomeScreen />);
    await userEvent.click(screen.getByRole("button", { name: /check worker/i }));
    expect(
      await screen.findByText(/could not run the worker sidecar/i),
    ).toBeInTheDocument();
  });
});

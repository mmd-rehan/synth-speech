import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const jobsUpdateSegment = vi.fn();
const jobsSegments = vi.fn();
const jobsExport = vi.fn();
const save = vi.fn();

vi.mock("@/bindings", () => ({
  commands: {
    jobsSegments: (id: string) => jobsSegments(id),
    jobsAllowPlayback: () =>
      Promise.resolve({ status: "ok", data: "/clips/a.wav" }),
    jobsUpdateSegment: (id: string, idx: number, text: string) =>
      jobsUpdateSegment(id, idx, text),
    jobsExport: (id: string, format: string, path: string) =>
      jobsExport(id, format, path),
  },
  events: {},
}));
// jsdom has no layout to measure, so every row counts as visible here. The real
// list is exercised by the Playwright flow.
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 32,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({ index, start: index * 32 })),
    measureElement: () => {},
  }),
}));
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => `asset://${p}`,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: () => save() }));

import { TranscriptScreen } from "@/components/screens/TranscriptScreen";
import { useJobs } from "@/lib/jobs";

const LINES = [
  { idx: 0, start_ms: 0, end_ms: 1000, text: "hello wrld" },
  { idx: 1, start_ms: 65000, end_ms: 66000, text: "second line" },
];

describe("TranscriptScreen", () => {
  beforeEach(() => {
    jobsSegments.mockResolvedValue({ status: "ok", data: LINES });
    jobsUpdateSegment.mockResolvedValue({ status: "ok", data: null });
    jobsExport.mockResolvedValue({ status: "ok", data: null });
    useJobs.setState({
      byId: { a: { id: "a", input_path: "/clips/a.wav" } } as never,
      segments: {},
    });
  });

  it("shows lines with timestamps and filters by search", async () => {
    const user = userEvent.setup();
    render(<TranscriptScreen jobId="a" onBack={() => {}} />);
    expect(await screen.findByText("hello wrld")).toBeInTheDocument();
    expect(screen.getByText("1:05")).toBeInTheDocument();
    await user.type(screen.getByRole("searchbox"), "second");
    expect(screen.queryByText("hello wrld")).toBeNull();
    expect(screen.getByText("second line")).toBeInTheDocument();
    await user.clear(screen.getByRole("searchbox"));
    await user.type(screen.getByRole("searchbox"), "nothing like this");
    expect(screen.getByText(/no lines match/i)).toBeInTheDocument();
  });

  it("saves an edited line", async () => {
    const user = userEvent.setup();
    render(<TranscriptScreen jobId="a" onBack={() => {}} />);
    await user.click(await screen.findByText("hello wrld"));
    const input = screen.getByRole("textbox");
    await user.clear(input);
    await user.type(input, "hello world{Enter}");
    expect(jobsUpdateSegment).toHaveBeenCalledWith("a", 0, "hello world");
    expect(await screen.findByText("hello world")).toBeInTheDocument();
  });

  it("Escape abandons an edit", async () => {
    const user = userEvent.setup();
    render(<TranscriptScreen jobId="a" onBack={() => {}} />);
    await user.click(await screen.findByText("hello wrld"));
    await user.type(screen.getByRole("textbox"), "oops{Escape}");
    expect(jobsUpdateSegment).not.toHaveBeenCalled();
    expect(screen.getByText("hello wrld")).toBeInTheDocument();
  });

  it("exports to the chosen file", async () => {
    save.mockResolvedValue("/out/a.srt");
    const user = userEvent.setup();
    render(<TranscriptScreen jobId="a" onBack={() => {}} />);
    await screen.findByText("hello wrld");
    await user.click(screen.getByRole("button", { name: "Export SRT" }));
    expect(jobsExport).toHaveBeenCalledWith("a", "srt", "/out/a.srt");
  });

  it("does nothing when the save dialog is dismissed", async () => {
    save.mockResolvedValue(null);
    const user = userEvent.setup();
    render(<TranscriptScreen jobId="a" onBack={() => {}} />);
    await screen.findByText("hello wrld");
    await user.click(screen.getByRole("button", { name: "Export TXT" }));
    expect(jobsExport).not.toHaveBeenCalled();
  });
});

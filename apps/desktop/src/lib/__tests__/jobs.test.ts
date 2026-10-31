import { describe, expect, it } from "vitest";

import type { Job } from "@/bindings";
import {
  fileName,
  formatDuration,
  formatTimestamp,
  gaugeMessage,
  groupJobs,
} from "@/lib/jobs";

function job(id: string, state: Job["state"], rank: number, priority: Job["priority"] = "normal"): Job {
  return { id, state, rank, priority } as Job;
}

describe("groupJobs", () => {
  it("puts active work first and drops empty groups", () => {
    const groups = groupJobs([
      job("a", "completed", 1),
      job("b", "running", 2),
      job("c", "queued", 3),
    ]);
    expect(groups.map((g) => g.state)).toEqual(["running", "queued", "completed"]);
  });

  it("orders a group by priority then rank", () => {
    const [group] = groupJobs([
      job("late", "queued", 9),
      job("early", "queued", 1),
      job("urgent", "queued", 20, "high"),
      job("whenever", "queued", 0, "low"),
    ]);
    expect(group.jobs.map((j) => j.id)).toEqual([
      "urgent",
      "early",
      "late",
      "whenever",
    ]);
  });
});

describe("formatting", () => {
  it("takes the file name from any platform's path", () => {
    expect(fileName("/Users/a b/clip one.wav")).toBe("clip one.wav");
    expect(fileName("C:\\Users\\a b\\録音.mp3")).toBe("録音.mp3");
  });

  it.each([
    [null, "—"],
    [0.2, "1 s"],
    [42, "42 s"],
    [180, "3 min"],
    [3900, "1 h 5 min"],
    [7200, "2 h"],
  ])("formats %s seconds as %s", (secs, text) => {
    expect(formatDuration(secs)).toBe(text);
  });

  it("formats transcript timestamps", () => {
    expect(formatTimestamp(65_000)).toBe("1:05");
    expect(formatTimestamp(3_725_000)).toBe("1:02:05");
  });
});

describe("gaugeMessage", () => {
  it("speaks for every level", () => {
    expect(gaugeMessage("safe", 1, 4)).toBe("Your machine can run 1 job at once.");
    expect(gaugeMessage("tight", 3, 4)).toMatch(/other apps to slow down/);
    expect(gaugeMessage("too_many", 9, 4)).toMatch(/Max safe is 4/);
  });
});

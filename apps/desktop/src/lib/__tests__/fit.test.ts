import { describe, expect, it } from "vitest";

import { speedLabel, verdictLabel, verdictRank } from "@/lib/fit";

describe("verdictLabel", () => {
  it("labels each verdict", () => {
    expect(verdictLabel("excellent")).toBe("Excellent");
    expect(verdictLabel("not_recommended")).toBe("Not recommended");
  });
});

describe("verdictRank", () => {
  it("orders best highest", () => {
    expect(verdictRank("excellent")).toBeGreaterThan(verdictRank("good"));
    expect(verdictRank("good")).toBeGreaterThan(verdictRank("slow"));
    expect(verdictRank("slow")).toBeGreaterThan(verdictRank("risky"));
    expect(verdictRank("risky")).toBeGreaterThan(verdictRank("not_recommended"));
  });
});

describe("speedLabel", () => {
  it("formats fast, realtime, and slow", () => {
    expect(speedLabel(0.1)).toBe("~10× realtime");
    expect(speedLabel(1)).toBe("~realtime");
    expect(speedLabel(2)).toBe("~2.0× slower");
    expect(speedLabel(null)).toBe("—");
  });
});

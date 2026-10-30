import { describe, expect, it } from "vitest";

import { acceleratorLabel, formatBytes, formatPct } from "@/lib/format";
import { ramUsedPct } from "@/lib/usage";
import type { UsageSample } from "@/bindings";

describe("formatBytes", () => {
  it("formats across units", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(16 * 1024 ** 3)).toBe("16 GB");
  });
  it("handles missing values", () => {
    expect(formatBytes(null)).toBe("—");
    expect(formatBytes(undefined)).toBe("—");
  });
});

describe("formatPct", () => {
  it("rounds and handles missing", () => {
    expect(formatPct(42.6)).toBe("43%");
    expect(formatPct(null)).toBe("—");
  });
});

describe("acceleratorLabel", () => {
  it("maps known kinds and falls back", () => {
    expect(acceleratorLabel("metal_gpu")).toBe("Metal GPU");
    expect(acceleratorLabel("cpu_avx512")).toBe("CPU (AVX-512)");
    expect(acceleratorLabel("something_new")).toBe("something_new");
  });
});

describe("ramUsedPct", () => {
  it("is zero when there is no sample", () => {
    expect(ramUsedPct(null)).toBe(0);
  });
  it("computes used / (used + available)", () => {
    const sample = {
      ram_used_bytes: 3,
      ram_available_bytes: 1,
    } as UsageSample;
    expect(ramUsedPct(sample)).toBe(75);
  });
});

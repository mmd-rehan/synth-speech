/** Format a byte count as a human-readable string (binary units). */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes)) return "—";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB", "PB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unit]}`;
}

/** Format a 0–100 percentage. */
export function formatPct(pct: number | null | undefined): string {
  if (pct == null || !Number.isFinite(pct)) return "—";
  return `${Math.round(pct)}%`;
}

/** Human label for an accelerator's `kind` tag. */
export function acceleratorLabel(kind: string): string {
  switch (kind) {
    case "metal_gpu":
      return "Metal GPU";
    case "core_ml":
      return "Core ML";
    case "cuda":
      return "CUDA";
    case "direct_ml":
      return "DirectML";
    case "vulkan":
      return "Vulkan";
    case "cpu_avx2":
      return "CPU (AVX2)";
    case "cpu_avx512":
      return "CPU (AVX-512)";
    case "cpu_neon":
      return "CPU (NEON)";
    default:
      return kind;
  }
}

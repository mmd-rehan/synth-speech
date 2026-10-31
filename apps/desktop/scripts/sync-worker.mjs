// Builds the `oas-worker` crate and copies the binary into
// `src-tauri/binaries/` with the Rust target-triple suffix that Tauri expects
// for a sidecar (`externalBin`). Run automatically before `tauri dev`/`build`.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const desktopDir = resolve(here, "..");
const workspaceRoot = resolve(desktopDir, "..", "..");

/** Read the host target triple from rustc, or honour an explicit override. */
function targetTriple() {
  const override = process.env.OAS_TARGET_TRIPLE;
  if (override) return override;
  const info = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const match = info.match(/^host:\s*(.+)$/m);
  if (!match) throw new Error("could not determine host triple from `rustc -vV`");
  return match[1].trim();
}

/**
 * whisper.cpp defaults to the build machine's exact CPU, which would crash on
 * older ones, and MSVC gets no SIMD at all. Shipped x86-64 workers target AVX2
 * explicitly. ARM builds keep their defaults.
 */
function portableSimdFlags(triple) {
  if (!triple.startsWith("x86_64")) return {};
  return {
    GGML_NATIVE: "OFF",
    GGML_AVX: "ON",
    GGML_AVX2: "ON",
    GGML_FMA: "ON",
    GGML_F16C: "ON",
  };
}

/** Run time libraries the Windows worker needs, which a clean PC may not have. */
const WINDOWS_RUNTIME_DLLS = [
  "vcomp140.dll",
  "vcruntime140.dll",
  "vcruntime140_1.dll",
  "msvcp140.dll",
];

function main() {
  const triple = targetTriple();
  const isWindows = triple.includes("windows");
  const ext = isWindows ? ".exe" : "";

  // The `--target` flag is only needed when cross-compiling; when building for
  // the host, cargo omits the triple directory.
  const explicitTarget = process.env.OAS_TARGET_TRIPLE;
  const cargoArgs = ["build", "--release", "-p", "oas-worker"];
  if (explicitTarget) cargoArgs.push("--target", triple);
  // Apple Silicon workers use the GPU. Other platforms ship CPU workers.
  if (triple === "aarch64-apple-darwin") cargoArgs.push("--features", "metal");
  // ggml's own thread pool is costly on Windows (ADR-24).
  if (isWindows) cargoArgs.push("--features", "openmp");

  console.log(`[sync-worker] building oas-worker for ${triple}`);
  execFileSync("cargo", cargoArgs, {
    cwd: workspaceRoot,
    stdio: "inherit",
    env: { ...process.env, ...portableSimdFlags(triple) },
  });

  const builtDir = explicitTarget
    ? resolve(workspaceRoot, "target", triple, "release")
    : resolve(workspaceRoot, "target", "release");
  const src = resolve(builtDir, `oas-worker${ext}`);

  const destDir = resolve(desktopDir, "src-tauri", "binaries");
  mkdirSync(destDir, { recursive: true });
  const dest = resolve(destDir, `oas-worker-${triple}${ext}`);

  copyFileSync(src, dest);
  console.log(`[sync-worker] copied -> ${dest}`);

  if (isWindows && process.env.SystemRoot) {
    for (const dll of WINDOWS_RUNTIME_DLLS) {
      copyFileSync(join(process.env.SystemRoot, "System32", dll), resolve(destDir, dll));
    }
  }
}

main();

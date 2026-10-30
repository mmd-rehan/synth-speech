// Builds the `oas-worker` crate and copies the binary into
// `src-tauri/binaries/` with the Rust target-triple suffix that Tauri expects
// for a sidecar (`externalBin`). Run automatically before `tauri dev`/`build`.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
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

function main() {
  const triple = targetTriple();
  const isWindows = triple.includes("windows");
  const ext = isWindows ? ".exe" : "";

  // The `--target` flag is only needed when cross-compiling; when building for
  // the host, cargo omits the triple directory.
  const explicitTarget = process.env.OAS_TARGET_TRIPLE;
  const cargoArgs = ["build", "--release", "-p", "oas-worker"];
  if (explicitTarget) cargoArgs.push("--target", triple);

  console.log(`[sync-worker] building oas-worker for ${triple}`);
  execFileSync("cargo", cargoArgs, {
    cwd: workspaceRoot,
    stdio: "inherit",
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
}

main();

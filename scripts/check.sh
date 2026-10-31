#!/usr/bin/env bash
# Full quality gate: run before every commit and in CI.
# Formatting, linting (warnings denied), type-checking, and tests for both the
# Rust workspace and the TypeScript UI.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
desktop_dir="$repo_root/apps/desktop"
cd "$repo_root"

step() { printf '\n\033[1;34m==> %s\033[0m\n' "$1"; }

step "Rust: format check"
cargo fmt --all --check

# The Tauri crate's build requires the worker sidecar binary to exist, so build
# and stage it before any workspace-wide cargo command.
step "Build worker sidecar"
(cd "$desktop_dir" && pnpm worker:sync)

step "Rust: clippy (warnings denied)"
cargo clippy --workspace --all-targets -- -D warnings

step "Rust: tests"
cargo test --workspace

step "UI: install dependencies (frozen lockfile)"
(cd "$desktop_dir" && pnpm install --frozen-lockfile)

step "UI: lint (warnings denied)"
(cd "$desktop_dir" && pnpm exec eslint . --max-warnings 0)

step "UI: type check"
(cd "$desktop_dir" && pnpm typecheck)

step "UI: tests"
(cd "$desktop_dir" && pnpm test)

step "Regenerate IPC bindings"
cargo run --quiet -p oas-desktop --features export-bindings --bin export-bindings

step "Check generated bindings are up to date"
if ! git diff --quiet -- "$desktop_dir/src/bindings.ts"; then
  echo "error: apps/desktop/src/bindings.ts is out of date." >&2
  echo "       Run 'cargo run -p oas-desktop --features export-bindings --bin export-bindings' and commit the result." >&2
  exit 1
fi

printf '\n\033[1;32mAll checks passed.\033[0m\n'

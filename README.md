<div align="center">

# Open Audio Studio

**Run speech AI models locally, on your own machine.**

[![CI](https://github.com/mmd-rehan/synth-speech/actions/workflows/ci.yml/badge.svg)](https://github.com/mmd-rehan/synth-speech/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)

</div>

Open Audio Studio is a free, open-source desktop app that runs OpenAI Whisper and
other open speech models **fully on your computer**. It inspects your hardware,
tells you honestly how each model will perform *before* you download it, and runs
many transcription jobs in parallel without freezing or overheating your machine.

- **Local only.** Your audio never leaves the device. Network is used only to
  download models and check for updates — both can be turned off.
- **Honest about hardware.** Every model shows a predicted speed and memory
  verdict for *your* machine before you commit to a download.
- **Safe by default.** A resource governor refuses or warns before anything that
  could lock up your computer.
- **Cross-platform.** macOS and Windows are first-class; Linux is best-effort.

> **Status:** early development. The app is being built milestone by milestone and
> is not yet ready for general use.

## Platforms

| Platform | Support |
|----------|---------|
| macOS (Apple Silicon) | Primary |
| macOS (Intel) | Supported |
| Windows 10/11 (x64) | Primary |
| Windows (ARM64) | Planned |
| Linux | Best-effort |

## Development

Requirements:

- [Rust](https://rustup.rs) (stable; pinned via `rust-toolchain.toml`)
- [Node.js](https://nodejs.org) 20+ and [pnpm](https://pnpm.io) 9+
- Platform toolchain for [Tauri 2](https://v2.tauri.app/start/prerequisites/)
  (Xcode Command Line Tools on macOS; WebView2 + MSVC Build Tools on Windows)

```bash
# install JS dependencies
pnpm install

# run the app in development
pnpm tauri dev

# run all checks (format, lint, type-check, tests — Rust and UI)
./scripts/check.sh
```

Rust-only tests:

```bash
cargo test --workspace
```

## Architecture

A Tauri 2 shell with a Rust core and a React + TypeScript UI. Model inference runs
in **separate worker processes** so a native crash or out-of-memory kill can never
take down the UI or other jobs. The workspace is organised into small, layered
crates (`oas-core`, `oas-hardware`, `oas-fit`, `oas-catalog`, `oas-download`,
`oas-audio`, `oas-engine`, `oas-jobs`, `oas-storage`, `oas-export`,
`oas-protocol`).

## Contributing

Contributions are welcome once the project reaches its first public milestone.
Please open an issue to discuss substantial changes first.

## License

[MIT](./LICENSE). Model weights carry their own licenses, shown in the app before
download.

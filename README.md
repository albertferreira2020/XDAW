# XDAW

XDAW is an independent desktop DAW written in Rust. Its workflows are modeled on the public
documentation of Ableton Live 10 (Session/Arrangement clip launching, recording, mixing and
export), with its own name, look, instruments, presets and demo content. It contains no
proprietary code, libraries or assets, and makes no claim of sound equivalence to any commercial
product.

Status: early milestone on **macOS Apple Silicon** (only platform built and tested). The core is
portable; Windows and Linux are **not verified**. See [docs/FEATURE_MATRIX.md](docs/FEATURE_MATRIX.md)
for exactly what is implemented and verified.

## Build

Requirements: Rust 1.85+ (tested with 1.99.0) and Xcode Command Line Tools.

```bash
cargo build --release --locked
```

Run the tests (offline, no audio device needed):

```bash
cargo test --workspace --locked
```

Generate the demo session (all samples are synthesized by XDAW itself):

```bash
cargo run --release -p xdaw-engine --bin xdaw-make-demo -- "demo/XDAW Demo"
```

Start the app with the demo:

```bash
cargo run --release -p xdaw-ui --bin xdaw -- "demo/XDAW Demo/XDAW Demo.xdaw"
```

Package `dist/XDAW.app` and a zip:

```bash
./scripts/package-macos.sh
```

## Headless tools

- `xdaw-headless play <set.xdaw> [seconds] [buffer]` — plays through the default device, launching
  each scene, and prints callback timing (mean/percentiles/max vs. budget), deadline misses,
  CPU and peak memory.
- `xdaw-headless export <set.xdaw> <out.wav> [16|24|32]` — renders the Arrangement offline.
- `XDAW_SCRIPT="wait 2; scene 2; wait 4; shot /tmp/a.png; quit" xdaw <set.xdaw>` — scripted UI
  driver used for end-to-end checks (see `crates/ui/src/script.rs`).

## Documentation

- [User guide](docs/USER_GUIDE.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Project file format](docs/PROJECT_FORMAT.md)
- [Feature matrix and verification status](docs/FEATURE_MATRIX.md)
- [Dependencies and licenses](docs/DEPENDENCIES.md)
- [Research notes (Live 10 behaviour contracts)](docs/research/)
- [Progress log and next steps](docs/PROGRESS.md)

## License

MIT OR Apache-2.0.

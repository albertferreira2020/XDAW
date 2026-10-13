# Environment and investigation tooling (2026-10-13)

## Host

| Item | Value |
| --- | --- |
| Machine | MacBook Air, Apple M5, 16 GiB RAM |
| OS | macOS 26.6.2 (25G83), arm64 |
| Rust | rustc 1.99.0, cargo 1.99.0 (rustup) |
| C toolchain | Apple clang 21.0.0 (Command Line Tools; full Xcode not selected) |
| Node | v22.23.2 (Homebrew) |
| Java | `/usr/bin/java` stub present |
| Audio output | Built-in speakers, 2 ch, 48 kHz (default) |
| Audio input | Built-in microphone, 1 ch, 48 kHz (default); "Microsoft Teams Audio" virtual device also present |
| MIDI hardware | None detected at reconnaissance time (to be re-checked through midir at runtime) |

## REA (investigation dependency, not product code)

- Checkout: `~/Documents/rea`, commit `16f5ded44`, package `rea-agents@6.4.0`.
- The compiled runtime was missing; built locally with `npm ci && npm run build:cached` (no config writes).
- CLI verified: `node scripts/rea.mjs --help` lists the real catalog (e.g. `inspect-macho`,
  `inspect-plist`, `inspect-signature`, `decompile`, `function`, `trace`, `xrefs`,
  `observe-native-calls`, `observe-native-ui`, `capture-native-ui-scenario`).
- MCP guided workflows exist in `src/contracts/promptContracts.ts`: `investigate_feature`,
  `verify_reconstruction`, `audit_residual_unknowns` (plus `compare_application_versions`,
  `trace_crash`, `prepare_bounded_process_capture`). They orient an agent; they do not run a
  reconstruction.
- `rea doctor`: `healthy: false`. Hopper missing, `GHIDRA_INSTALL_DIR` unset, IDA MCP
  registration absent, REA skill/agent registration absent.
- `rea providers`: only `native-macos` utilities available (Mach-O/plist/signature inspection,
  native UI observation, LLDB call observation).
- Not done: `npx rea-agents setup`. It would register agent clients and can install Hopper; the
  project rules require showing planned changes and user approval. Pending user decision.

## Ableton Live 10 installation

- `/Applications`, `~/Applications` and Spotlight (`kMDItemCFBundleIdentifier == com.ableton.live*`,
  name "Ableton") contain **no Ableton Live installation**.
- Consequence: no version/edition/architecture/hash can be recorded and no binary or controlled
  runtime experiment against Live 10 is possible. Static analysis (Hopper/Ghidra/IDA) is also
  unavailable. **Blocked dependency: path to a Live 10 installation provided for this work.**
- Work continues from the official Live 10 Reference Manual (see `sources.md`).

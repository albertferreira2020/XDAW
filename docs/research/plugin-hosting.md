# Plugin hosting study (VST3 / Audio Units) — not implemented

## Requirements from the prompt

Discovery, parameters, latency (compensation), crash isolation and recovery, plus the licenses of
any SDKs used.

## Options

| Option | Platform | License | Notes |
| --- | --- | --- | --- |
| Steinberg VST3 SDK (C++) via bindings (e.g. `vst3` crates) | macOS/Windows/Linux | Historically GPLv3 or a Steinberg proprietary agreement; newer SDK releases are reported to use MIT | Unverified here: check the license of the exact SDK version before adoption. |
| Audio Units v2/v3 via AudioToolbox (Objective-C/Swift bridge) | macOS only | Apple platform APIs | AUv3 out-of-process by default on macOS, which isolates crashes. |
| CLAP (`clap-sys`) | all | MIT | Not requested, but simplest to host. |

## Proposed design

- `plugin-host` crate exposes a `HostedPlugin` boundary. It covers parameter descriptors, latency in
  samples, and process with events.
- Plugins run in a separate **host process**, one per plugin or per track. Audio and events are
  exchanged through shared-memory ring buffers, and a watchdog restarts a crashed host. The
  affected device outputs silence and is shown as "crashed" in the UI, so the main process never
  loads third-party code.
- The engine compensates the reported latency by delaying parallel paths: track output delay
  lines are sized to the maximum chain latency. That gives the sends/returns sums aligned inputs.
- Discovery runs in a scanner subprocess that writes a cache, and a crash during scanning
  blacklists the offending plugin.

## Status

Not implemented. Before implementing, decide on the SDK and license (VST3 version), and approve the
out-of-process architecture.

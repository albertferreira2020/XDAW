# XDAW progress log

Product name: **XDAW** (renamed from the working name "Ostinato" on user request, 2026-10-13).
Repository: `~/Documents/XDAW` (local git, `main`). REA lives separately in `~/Documents/rea`
and is an investigation dependency only.

## Decisions

| Decision | Reason |
| --- | --- |
| Rust 2024 workspace: `sequencer`, `dsp`, `midi`, `project`, `engine`, `plugin-host`, `ui` | Prompt architecture; dependencies flow inward (sequencer/dsp → project → engine → ui). |
| CPAL 0.18.2, midir 0.11.1, egui/eframe 0.36.2, symphonia 0.6.1, hound 3.5.1, rtrb 0.4.0, midly 0.5.3 | Current releases on crates.io (2026-10-13); APIs checked in the registry sources. All MIT/Apache-2.0. |
| One `Renderer` for realtime and export | Export equals playback by construction; tested for block-size independence. |
| UI owns the `Project`; audio thread gets compiled `Graph` snapshots + fine-grained commands | No locks in the callback; structural edits swap at block boundaries; runtime state (voices, launchers, smoothers) transferred by stable ids. |
| Replaced payloads return through a garbage ring | No deallocation on the audio thread. `SetClip` carries a `Box<Option<RtClip>>` that brings the old value back. |
| Musical clock anchored to integer samples per tempo segment | No accumulated drift (tested: 200 loops at 44.1 kHz/128 BPM within 1 sample). |
| Warp mode "Re-Pitch" first (pitch follows tempo) | Honest resampling; pitch-preserving stretching is a later, separately validated feature. |
| Own JSON project format `.xdaw`, versioned (`format`, `version`) | Atomic save (temp + fsync + rename), autosave + recovery, relative media paths, Collect. |

## Evidence so far

- `cargo test` (workspace): sequencer 16, dsp 21, midi 6, project 7, engine scheduler 15 — all passing.
- Quantized launch (own engine): onsets land in [0, 1) sample of the exact bar boundary at
  48 kHz/120, 44.1 kHz/128 and 48 kHz/97.3 BPM; tempo change while queued keeps the musical boundary.
- Real hardware (MacBook Air M5, built-in speakers, 48 kHz, buffer 256, demo session, 15 s,
  5 scene launches): 2878 callbacks, mean 0.100 ms, max 0.143 ms, budget 5.333 ms, 0 deadline
  misses, process CPU 2.0 %, peak RSS 35.7 MB. One backend stream error was counted in that run
  and not reproduced in a second run; its message is now logged (pending characterization).
- Offline export of the demo Arrangement: 33.5 s rendered in 0.32 s, 24-bit, peak 0.835, 0 clipped.

## Blocked / waiting on user

- Ableton Live 10 is not installed on this Mac; no native RE provider (Hopper/Ghidra/IDA) configured.
  Needed: path to a Live 10 installation provided for this work, and approval to run
  `npx rea-agents setup` (or a Ghidra 12.1.x path) if binary analysis is wanted.

## Next executable step

See the bottom of this file (updated at each milestone).

- [x] Stage 1: research contracts for launch/quantization (docs/research).
- [x] Stage 2 backend: device output, transport, clips, mixer, quantized launch, export.
- [ ] Stage 2 UI: egui app with transport, Session grid, mixer, export dialog.

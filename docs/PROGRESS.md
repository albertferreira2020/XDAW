# XDAW progress log

Product name: **XDAW** (renamed from the working name "Ostinato" on user request, 2026-10-13).
Repository: `~/Documents/XDAW` (local git, branch `main`, no remote yet). REA lives separately in
`~/Documents/rea` and is an investigation dependency only.

## Decisions

| Decision | Reason |
| --- | --- |
| Rust 2024 workspace: `sequencer`, `dsp`, `midi`, `project`, `engine`, `plugin-host`, `ui` | Prompt architecture; dependencies flow inward. |
| CPAL 0.18.2, midir 0.11.1, egui/eframe 0.36.2, symphonia 0.6.1, hound 3.5.1, rtrb 0.4.0, midly 0.5.3 | Current releases (2026-10-13); APIs checked in registry sources; licenses in `DEPENDENCIES.md`. |
| One `Renderer` for realtime and export | Export equals playback by construction (tested ≤ 5 % of peak across block sizes). |
| UI owns the `Project`; audio thread gets compiled `Graph` snapshots + fine-grained commands via a reconciler | No locks in the callback; structural edits swap at block boundaries with state transfer. |
| Replaced payloads return through a garbage ring | No deallocation on the audio thread. |
| Integer-sample-anchored musical clock | No drift (200 loops at 44.1 kHz/128 BPM within 1 sample). |
| Undo = automatic gesture-coalesced commits of reversible edits (track/transport/scenes/project) | Every UI mutation becomes undoable without per-widget plumbing. |
| Pitch-preserving stretch rendered offline (WSOLA) on the media worker | Keeps the callback cheap; Re-Pitch fallback while rendering. |
| Scripted UI driver with real pointer/key injection (`XDAW_SCRIPT`) | macOS denied screen capture to the terminal; the app captures its own screenshots. |

## Evidence

- `cargo test --workspace`: 87 tests passing (sequencer 19, dsp 23, midi 6, project 7, engine
  lib 1 + scheduler 19 + recording 8, ui 4).
- Quantized launch (own engine): onsets in [0, 1) sample of the exact boundary at 48 kHz/120,
  44.1 kHz/128, 48 kHz/97.3 BPM; tempo change while queued keeps the musical boundary.
- Real hardware, headless (MacBook Air M5, built-in speakers, 48 kHz, buffer 256, demo, 15 s):
  2878 callbacks, mean 0.100 ms, max 0.143 ms of 5.333 ms, 0 deadline misses, 2.0 % CPU,
  35.7 MB peak RSS.
- Scripted UI journey on the demo: launch scenes with Arrangement Record → Arrangement gained
  the performance; undo/redo; Save As to another folder; reopen; export audible (peak 0.835,
  0 clipped). Found and fixed: Save As kept media relative to the old folder.
- Click test found and fixed: fade-out tail of a replaced clip started at the block start.
- 30-minute UI + audio stress run: see "Stress run" below.

## Blocked / waiting on user

- Ableton Live 10 is not installed; no native RE provider (Hopper/Ghidra/IDA) configured.
  Needed: path to a Live 10 installation provided for this work, and approval for
  `npx rea-agents setup` (or a Ghidra 12.1.x path) if binary analysis is wanted.
- Optional: GitHub remote for the XDAW repository.
- Real MIDI controller and microphone recording checks need hardware/permission on the user's side.

## Milestones

- [x] Stage 1: Live 10 contracts for launch/quantization (`docs/research`).
- [x] Stage 2: device output, transport, clips, mixer, quantized launch, export (real audio verified).
- [x] Stage 3: Session ↔ Arrangement on one model, editing, undo/redo, performance recording.
- [x] Stage 4: MIDI clips, instruments, MIDI/audio recording (engine-verified; hardware unverified).
- [x] Stage 5: automation, effects, persistence, browser, shortcuts.
- [~] Stage 6: Re-Pitch + Stretch warping, Follow Actions, Capture MIDI, groups, Flatten done;
  warp markers, racks/macros, multi-clip editing, real Freeze, plugin hosting not done.
- [~] Stage 7: audits — behaviour (tests), audio quality (DSP tests), performance (30-min run),
  packaging (`scripts/package-macos.sh`). UX interaction coverage is partial.

## Next executable steps

1. Run the mouse-driven UI journey (`XDAW_SCRIPT` with `click`/`drag`/`key`) to verify piano-roll
   editing, Arrangement clip drag/trim/split, automation point editing, Flatten and Capture MIDI
   buttons; record results in `FEATURE_MATRIX.md`.
2. Warp markers (model field exists): piecewise-linear beat→source-time map used by Re-Pitch and
   Stretch rendering.
3. Instrument/effect racks with 8 macros mapped to device parameters.
4. Multi-clip MIDI editing (show several clips in the piano roll).
5. Plugin hosting study: VST3 SDK license and AU (AudioToolbox) adapter in an out-of-process host.
6. Windows/Linux builds in CI (unverified today).

## Stress run (2026-10-13) — incomplete, fix pending verification

- 48 kHz / 256, demo set, scripted scene launches, volume edits, view switches, Arrangement Record.
- First ~10 min (111,104 callbacks): mean 0.085 ms, p99 ≤ 0.2 ms, max 0.490 ms of 5.333 ms budget,
  0 deadline misses, 0 stream errors, 0 dropped commands/events. Process ran 2.3 h total:
  113 s user CPU, 137 MB peak RSS.
- The script stalled after ~10 min because eframe skips `ui()` while the window is hidden. That
  exposed a product bug: engine events, garbage collection, sync and autosave only ran in `ui()`.
  **Fixed** (commit "fix(ui): ...") by moving them to `App::logic()` and using wall-clock script
  timing. Screenshots (`shot`) still need a visible window.
- NEXT: rerun the 30-minute script (without `shot`) and record full statistics; then run the
  mouse-driven UI journey (step 1 above).

## Timeline navigation and drag & drop (2026-10-13)

- User feedback: timeline zoom, a horizontal scroll bar and sample drag & drop were missing.
- Root cause for drag & drop: egui's `Response::dnd_release_payload::<T>()` **clears** a payload
  of another type. Session slots asked for `SlotDrag` first, so Browser files dropped on a slot
  were discarded (the Arrangement had the reverse problem for Session clips). Fixed at the source
  with `browser::dnd_release`, used by every drop target.
- Added `timeline.rs` (shared `HView` zoom/scroll + scroll bar + ruler zoom) used by the
  Arrangement, piano roll and sample editor; `filedrop.rs` bridges Finder drops (winit 0.30 has
  no drop position, so the pointer is read from Core Graphics during the drag).
- Verified with the scripted UI (results in `FEATURE_MATRIX.md`); 94 tests pass. A real Finder
  drag by a person is still unverified.

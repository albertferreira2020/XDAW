# Feature: Session clip launch, quantization and Arrangement priority

- **Source:** Live 10 Reference Manual §7.1, §7.2, §7.4.2, §7.5, §13.2–13.6 (see `sources.md`).
- **Version:** Live 10 (manual text). No Live 10 binary available (see `00-environment-and-tools.md`).
- **Experiment input:** none against Live 10 (blocked). Experiment against our own scheduler:
  `crates/sequencer` unit tests and `crates/engine/tests/scheduler.rs` (sample-accurate render).

## Observed in documentation (confirmed reference)

1. Launch modes: Trigger (down starts, up ignored), Gate (down starts, up stops), Toggle (down
   starts, next down stops), Repeat (retriggers at clip quantization rate while held). §13.2
2. Clip quantization chooser: "None" disables; "Global" follows the Control Bar's Global
   Quantization; any value other than None also quantizes Follow-Action launches. §13.3
3. Legato: launched clip takes over the play position of the previously playing clip in the
   track; works with quantization off. §13.5
4. One clip per track at a time; scenes are rows; Scene Launch launches every clip in the row. §7.2
5. Clip Stop buttons can be removed per slot to pre-configure scene launch: removing the stop
   button of scene 3/track 4 prevents scene 3 from affecting track 4. §7.4.2
   ⇒ an empty slot *with* a stop button stops the track on scene launch.
6. Stopping Session clips does not stop the transport; pressing Stop twice returns to 1.1.1. §7.1
7. Session and Arrangement clips in a track are mutually exclusive. Launching a Session clip stops
   that track's Arrangement playback; clicking a Clip Stop button stops Arrangement playback
   (silence). Arrangement resumes only via Back to Arrangement. §7.5
8. Arrangement Record logs launched clips (as clips, not new audio), clip property changes,
   mixer/device automation and scene tempo/signature changes. §7.5
9. Follow Action time is measured from where play starts; Follow Actions bypass global
   quantization but not clip quantization. §13.6

## Inferences (marked in code as own proposals)

- I1. At a launch boundary the old clip stops and the new one starts on the same sample (no gap,
  no overlap). Reasoning: "only one clip at a time" plus quantized onset.
- I2. A launch request landing exactly on a boundary fires on that boundary (epsilon 1e-9 beat).
- I3. Note-value grids (1/2 … 1/32) restart at each bar line, so odd meters keep the downbeat.
- I4. When the transport is stopped, launching a clip starts the transport and the clip at the
  current song position without waiting for a grid boundary.
- I5. Back to Arrangement takes effect immediately (not quantized) and stops Session clips.
- I6. Transport stop does not re-enable Arrangement playback for released tracks.

## Unknowns

- U1. Exact behaviour of Back to Arrangement timing (I5) and transport stop (I6) in Live 10.
- U2. Whether note-value quantization is measured from bar line or song start in odd meters (I3).
- U3. Pre-boundary scheduling latency/look-ahead inside Live (not observable without the binary).
- U4. Behaviour when tempo changes between request and boundary (we recompute sample position from
  the beat target, so the boundary stays musical).

All unknowns require a Live 10 installation for controlled experiments
(e.g. REA `capture-native-ui-scenario` + audio capture) or static analysis with a provider.

## Consequence for implementation

- `xdaw_sequencer::Quantize::next_boundary`, `TrackLauncher` implement 1–7 and I1–I6.
- The engine processes launch transitions at exact sample offsets inside the audio block
  (sub-block splitting), never on UI timers.
- Scene launch: clip slot → launch; empty slot with stop button → stop; empty slot without stop
  button → no effect.

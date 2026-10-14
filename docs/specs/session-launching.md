# Session: clip launching, quantization and Follow Actions

Status: implemented (`crates/sequencer`, `crates/engine/src/render.rs`, `crates/ui/src/session.rs`).

## Clips and scenes

- S1. The Session is a grid: tracks are columns, scenes are rows, each cell is a clip slot.
- S2. A track plays at most one Session clip at a time.
- S3. Launching a scene launches every clip in its row. An empty slot that has a stop button stops
  its track; an empty slot without a stop button leaves the track unchanged.
- S4. A scene whose name contains `<n> BPM` sets the tempo, and `<x>/<y>` sets the time signature,
  when it is launched.
- S5. Capture Scene copies the clips that are currently playing into a new scene inserted below the
  selected one and launches it.
- S6. Stopping clips does not stop the transport. Stopping the transport twice returns the
  position to the song start.

## Launch modes

- S7. Trigger: press starts, release is ignored. Gate: press starts, release stops. Toggle: press
  starts, the next press stops. Repeat: retriggers at the clip's quantization while held.
- S8. Legato: the new clip starts at the play position of the clip it replaces in the same track.

## Quantization

- S9. Each clip has a quantization of None, Global, or an explicit grid (whole bars, or note
  values 1/2 … 1/32). The transport holds the Global value.
- S10. A launch or stop request takes effect at the next boundary of the resolved grid. At that
  boundary the previous clip stops and the new clip starts on the same sample (no gap, no overlap).
- S11. A request arriving exactly on a boundary (within 1e-9 beat) takes effect on that boundary.
- S12. *XDAW choice:* note-value grids restart at each bar line, so odd meters keep the downbeat.
- S13. With the transport stopped, a launch starts the transport and the clip at the current
  position without waiting for a boundary.
- S14. Launch transitions are applied at exact sample offsets inside the audio block, never on UI
  timers. If the tempo changes before the boundary, the boundary stays musical (beat-based).

## Follow Actions

- S15. A group is a run of adjacent filled slots in one track.
- S16. Each clip has a Follow Action time (measured from where the clip started playing) and two
  actions A and B with relative chances. Actions: No Action, Stop, Play Again, Previous, Next,
  First, Last, Any, Other.
- S17. Follow Actions ignore the Global quantization but respect an explicit clip quantization.

## Session versus Arrangement

- S18. Session and Arrangement playback in one track are exclusive: launching a Session clip or
  pressing a clip stop button releases the track from the Arrangement until Back to Arrangement.
- S19. *XDAW choice:* Back to Arrangement takes effect immediately and stops the Session clips;
  transport stop does not by itself re-enable the Arrangement for released tracks.

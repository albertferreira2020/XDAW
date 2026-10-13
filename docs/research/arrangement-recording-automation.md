# Features: Arrangement recording, recording into Session, automation override

- **Source:** Live 10 Reference Manual §7.5, §16.2–16.6, §19 (see `sources.md`).
- **Version:** Live 10 (manual text only; no Live 10 binary available).
- **Experiment input:** XDAW engine tests (`crates/engine/tests/*.rs`) and the scripted UI journey
  (`XDAW_SCRIPT`), not Live 10.

## Observed in documentation (confirmed reference)

1. With Arrangement Record on, Live logs launched clips, clip property changes, mixer/device
   automation and scene tempo/meter changes. Recording stops on a second press or transport stop.
   The result contains clips only, no new audio. §7.5
2. Session recording: arm tracks, Session Record records into the selected scene; pressing again
   goes into loop playback; Clip Record buttons appear in empty slots of armed tracks. §16.3
3. MIDI overdub in a looping Session clip: Session Record toggles between playback and overdub. §16.3.3
4. Count-in runs from negative bars up to 1.1.1, then recording starts. §16.6
5. Moving an automated control overrides its automation; Back to Arrangement re-enables it. §19

## Inferences (own proposals)

- I1. Recording into the Arrangement overwrites existing material in the recorded range.
- I2. The length of a new Session MIDI recording rounds up to whole bars on stop.
- I3. Live input timestamps are block-accurate, so the error is at most one buffer.
- I4. Audio recording supports one simultaneous take; audio overdub is not supported.

## Unknowns

- U1. Live 10's exact rounding rule for new clip length (I2).
- U2. Whether Live 10 places recorded MIDI with sub-block precision (it likely uses driver timestamps).
- U3. Interaction of punch-in/out with Session-to-Arrangement recording (not implemented).

## Consequence for implementation

- Arrangement logging lives in the UI layer (`ClipStarted`/`ClipStopped` events, which carry exact
  beats) and is built on `Track::clear_arrangement_range`.
- `SessionRecord` in the engine; `NoteRecorded` events carry song and clip-local beats.
- Automation override flags live per lane in `TrackRuntime`; Back to Arrangement clears them.

# Architecture

```
            ┌────────────── ui (egui/eframe) ──────────────┐
            │ owns Project + History, views, dialogs,      │
            │ Reconciler → commands, EngineEvent handling   │
            └───────┬───────────────────────▲──────────────┘
          commands  │ rtrb SPSC             │ events (rtrb), status (atomics),
          (boxed    ▼                       │ garbage (rtrb, dropped on UI thread)
          payloads) ┌──────────── engine ───┴──────────────┐
                    │ Renderer (audio thread or offline)    │
                    │  Graph snapshot: tracks/returns/master│
                    │  TrackLauncher per track (sequencer)  │
                    │  Devices (dsp), mixer, sends, click   │
                    └──────┬──────────────┬────────────────┘
               CPAL output │   CPAL input │ ring → monitoring / recorder ring → disk thread
               midir ──────┘ (ring)
```

Crates (dependencies point inward):

| Crate | Responsibility |
| --- | --- |
| `sequencer` | Musical clock (sample-anchored tempo segments), bars/beats/samples conversions, launch quantization, per-track launch state machine, clip-local looping. Pure and allocation-free. |
| `dsp` | Parameter descriptors, smoothing, filters, envelopes, oscillators, instruments (Poly, Sampler, Drum Kit), effects (Utility, EQ, Filter, Compressor, Echo Delay, Room Reverb), enum device dispatch. |
| `midi` | Message parsing, midir input, computer keyboard mapping, SMF import/export. |
| `project` | Serializable model, validation, reversible edits/history, atomic save, autosave/recovery, media resolution and collection. |
| `engine` | Graph compilation, `Renderer`, command/event protocol, reconciler, media decoding/cache, CPAL I/O, disk recorder, offline export, demo generator, headless tools. |
| `plugin-host` | Placeholder boundary for VST3/AU hosting (not implemented). |
| `ui` | The desktop application. |

## Realtime rules

The audio callback (`Renderer::process_interleaved`) never allocates, locks, performs I/O or
formats strings:

- Commands arrive through a bounded SPSC ring and are applied at the start of the callback.
- Structural changes arrive as a complete `Graph` built on the UI thread. On swap, runtime state
  (launch state, active notes, smoothers, device DSP state for devices with the same id/kind) is
  moved into the new graph; the old graph goes back through the garbage ring and is dropped by the
  UI thread.
- Clip replacement (`SetClip`) carries a `Box<Option<RtClip>>`; the box returns the old clip, so no
  allocation is needed on the audio thread.
- Event vectors are pre-allocated; pushes beyond capacity are dropped rather than reallocated.
- Rings that are full drop the message and count it in `SharedStatus` (visible in the UI).

## Time

The audio clock is authoritative. `Clock` maps an integer sample counter to beats through tempo
segments anchored on exact samples, so positions are recomputed (not accumulated) and loops do not
drift. Launch transitions, MIDI notes, loop wraps and the metronome are scheduled at sample
offsets inside the block (sub-block splitting). Live input from the computer keyboard/MIDI is
timestamped at block start (jitter ≤ one buffer).

## Playback and export share one path

`export::render_offline` builds the same `Graph` and runs the same `Renderer` with
`process_planar`, starting the transport at the export range and releasing all sources for the
tail. Differences between block sizes come only from per-block coefficient update phase in filters
and are bounded in tests (≤ 5 % of peak).

## Session vs Arrangement priority

Per track, `TrackLauncher.arrangement_active` decides the source. Launching a Session clip or a
Clip Stop button releases the track from the Arrangement until Back to Arrangement. Manual changes
to automated parameters during playback override the lane (`automation_overridden`), and Back to
Arrangement re-enables them. Arrangement Record is implemented in the UI from `ClipStarted`/
`ClipStopped` events: each launched clip becomes an Arrangement clip referencing the same content
(no new audio is created), overwriting what was underneath.

## Latency compensation

All built-in devices report zero latency (`DeviceKind::latency_samples`). Plugin delay
compensation is therefore not needed yet and is not implemented; it must be added together with
plugin hosting.

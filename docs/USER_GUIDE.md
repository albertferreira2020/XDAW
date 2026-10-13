# XDAW user guide

## Layout

- **Control bar** (top): tempo, Tap, time signature, Metronome (Met), count-in, song position,
  Play ▶, Stop ■, Arrangement Record (Rec, F9), Session Record (S.Rec), Back to Arrangement
  (↺ Arr, lit when any track is overriding the Arrangement or automation), Loop, global launch
  Quantize, computer keyboard (KB), KEY and MIDI map modes, CPU meter, Session/Arrangement switch.
- **Browser** (left): instruments, effects, presets, the current project folder and added folders.
  Drag items onto slots, track headers, the device area or the empty "drop to create track" column;
  double-click loads onto the selected track/slot.
- **Main area**: Session or Arrangement (Tab switches).
- **Detail panel** (bottom): Clip (properties, piano roll or audio regions) or Devices.
- **Status bar**: messages, MIDI inputs, audio device, sample rate and buffer.

## Session View

- Click a clip's triangle to launch it; launches wait for the global quantization (or the clip's
  own). Gate/Toggle/Repeat are set per clip (Clip panel or right-click → Launch Mode).
- Empty slots show a stop square (stop the track) or a record circle on armed tracks
  (record a new clip). Right-click a slot to add/remove its stop button: a scene launch stops
  tracks whose slot has a stop button and leaves tracks without one alone.
- Scene launchers are on the right; scene names like `96 BPM` or `7/8` change tempo/meter.
- Double-click an empty MIDI slot to create a 1-bar MIDI clip. Drag clips to move; hold Alt/Cmd to
  copy. Cmd+D duplicates, Delete removes, Enter launches the selected clip.
- Below each column: sends, pan, volume fader with meter (click the meter to clear clipping),
  activator (track number), Solo (Cmd-click for non-exclusive), Arm (R), and monitoring for audio
  tracks (In / Auto / Off).

## Arrangement View

- Click the ruler to locate; drag in the thin band above it to set the loop, double-click it to
  toggle looping.
- Drag clips to move, drag edges to trim, drag the top corners to set fades. Cmd+E splits
  selected clips at the insert marker; Cmd+C/V copy/paste at the marker; Cmd+D duplicates.
- Choose an automation parameter in a track header to show its envelope. Cmd-click or double-click
  to add a point, drag to move, double-click a point to delete.
- Session clips launched while Arrangement Record is on are written into the Arrangement. When any
  track plays Session material, ↺ Arr lights; click it to return to the Arrangement.

## Recording

- MIDI: select a MIDI track (it arms automatically), play with a MIDI keyboard or the computer
  keyboard (A W S E D F T G Y H U J K… ; Z/X octave, C/V velocity). Press S.Rec to record into the
  first empty slot from the selected scene, or to overdub the clip that is playing; press again to
  finish (the new clip length rounds up to whole bars).
- Audio: enable input in Audio/MIDI Settings, arm an audio track, press its empty slot's record
  circle or S.Rec. Takes are written to `Samples/Recorded/` next to the Set (or to the app support
  folder for unsaved Sets).

## Devices

Instruments go first on MIDI tracks (Poly, Sampler, Drum Rack); effects follow in order. Use ◀ ▶ to
reorder, "On" to bypass, "..." to save a preset, reset or delete. Drop audio files on the Sampler's
sample field or on Drum Rack pads. Knobs: drag vertically (Shift for fine), double-click to reset.

## Files

- File → Save / Save As (atomic), Collect All and Save (copies external media into `Samples/`).
- Unsaved work is autosaved every 30 seconds and offered for recovery after a crash.
- File → Export Audio renders the Arrangement or loop region to WAV (16/24-bit with optional
  dither, or 32-bit float) with a tail for effects; the dialog reports peak and clipped samples.
- File → Import Audio/MIDI File, Export Selected MIDI Clip.

## Shortcuts

Space play/stop · Shift+Space continue · Tab switch view · F9 Arrangement Record · Cmd+Z / Cmd+Shift+Z
undo/redo · Cmd+S save · Cmd+Shift+S save as · Cmd+O open · Cmd+N new · Cmd+E split · Cmd+D duplicate ·
Cmd+C / Cmd+V copy/paste · Cmd+A select all notes · Cmd+Shift+R export · Cmd+, settings ·
Delete remove · Enter launch selected clip · arrows move/transpose selected notes.

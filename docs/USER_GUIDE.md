# XDAW user guide

## Layout

- **Control bar** (top): tempo, Tap, time signature, Metronome (Met), count-in, song position,
  Play ▶, Stop ■, Arrangement Record (Rec, F9), Session Record (S.Rec), Back to Arrangement
  (↺ Arr, lit when any track is overriding the Arrangement or automation), Loop, global launch
  Quantize, computer keyboard (KB), KEY and MIDI map modes, CPU meter, Session/Arrangement switch.
- **Browser** (left): instruments, effects, presets, the current project folder and added folders.
  Drag items onto slots, track headers, the device area or the empty "drop to create track" column;
  files can also be dragged straight from Finder (several files fill consecutive slots). A sample
  dropped on a MIDI track loads into its Drum Kit (next free pad) or a Sampler;
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
  toggle looping. Drag the ruler up/down to zoom and sideways to scroll.
- Zoom: pinch or Cmd+wheel over the timeline, `+`/`−`, the Zoom slider, **Sel** (`Z`, zoom to the
  selected clips or the loop) and **All** (`Shift+Z`, whole song). `Z` is the octave key while the
  computer MIDI keyboard (KB) is on.
- Scroll: the scroll bar under the timeline (drag the thumb; drag its ends to zoom; click the
  trough to jump), Shift+wheel or a horizontal trackpad swipe. Scrolling by hand while playing turns
  Follow off. Track height: the Height slider or Alt+wheel.
- Drop samples or MIDI files from the Browser or from Finder onto a lane (placed at the pointer,
  several files one after another) or onto the area under the last track to create a new track.
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

A chain has three sections, always in this order: MIDI effects (Arpeggiator, Chord Stack, Scale
Lock, Note Duration, Pitch Randomizer, Velocity Shaper), one instrument (Poly, FM Synth, Morph
Synth, Sampler, Drum Kit), then audio effects. MIDI effects and instruments need a MIDI track.
Use ◀ ▶ to reorder within a section, "On" to bypass, "..." to save a preset, reset or delete.
Drop audio files on the Sampler's sample field or on Drum Kit pads. Knobs: drag vertically (Shift
for fine), double-click to reset.

- **Parametric EQ**: pick a band with 1–8, switch it On, choose its type; the curve shows the sum.
- **Sidechain**: Compressor, Bus Compressor and Gate have a Sidechain chooser; pick another track
  to duck or gate this one with it (a muted source still works).
- **Racks**: add an Instrument Rack (MIDI tracks) or Effect Rack, press **+ Chain**, select a chain
  and drop devices on "Drop into …". Each chain has an on switch (A), Solo, volume and, in
  Instrument Racks, a key zone, so one rack can split the keyboard. Right-click a chain to delete it.
  The track's macros can map any parameter inside a rack.
- **Limiter Lookahead**: smoother limiting at the cost of a few milliseconds of latency, which XDAW
  compensates on all other tracks automatically.

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

## Limiter and presets
Add **Limiter** from Browser > Audio Effects (put it last on the master). Gain drives the signal
into the ceiling; nothing leaves the device above Ceiling. Utility, EQ Three-Band and Limiter have
a **Presets** menu in the device header and factory entries under Browser > Presets.

## Recording into the Arrangement
Arm one or more tracks (R), optionally switch **Pin/Pout** (punch uses the loop brace), then press **Rec**.
Audio goes to disk while the song position is inside the punch window; MIDI is collected from played notes.
Every pass of a loop becomes its own take. Takes appear as lanes under the track (**T** button in the
track header); drag across a take lane, or click a clip on it, to make that part the comp. Recording
over existing material keeps it as an "Original" lane.

## Navigating and editing
- **Mixer** column (right of the lanes): activator, solo, arm, volume, pan, sends (Snd), meter.
- **Time** ruler (m:ss) and **locators**: double-click the strip under the bar numbers, `+ Locator`,
  drag to move, right-click to rename or delete, click to jump.
- Snap: Auto (adapts to zoom), Off, bar … 1/32 and triplets.
- Pick an automation lane in a track header, then use **Draw** (Select, Draw, Line, Erase) in the toolbar.
- Right-click a clip: Consolidate (Cmd+J), Reverse, Crossfade selected neighbours, Slice to New MIDI Track.
- Right-click a track header: Fold, Freeze/Unfreeze, Flatten Track. Groups can contain groups.
- Piano roll: Shift-click several Arrangement MIDI clips to edit them together (Multi-clip);
  **Lane** switches the bottom lane between Velocity and Chance.
- Sample editor: **Envelope** selects Clip Gain or Clip Pan and edits it with the same tools.

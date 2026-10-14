# Arrangement: navigation, recording, takes, automation and audio editing

Status: implemented (`crates/ui/src/arrangement.rs`, `timeline.rs`, `arrec.rs`, `audio_ops.rs`,
`crates/project/src/arrange.rs`, `crates/engine/src/render.rs`).

## Navigation

- A1. Pinch or Cmd+wheel zooms around the pointer. Horizontal wheel or Shift+wheel scrolls.
- A2. The scroll bar: drag the thumb to scroll, drag its ends to change the visible range, click
  the trough to jump there, wheel over it to scroll.
- A3. On the beat ruler, vertical drag zooms (down zooms in) and horizontal drag scrolls.
- A4. *XDAW choice:* the grid is "Auto" (smallest power-of-two beat division at least 12 px wide)
  or a fixed value, including triplet values.
- A5. A time ruler in minutes:seconds sits below the beat ruler. Named locators can be added,
  dragged, renamed, deleted and clicked to jump.

## Recording

- A6. Arrangement Record logs launched Session clips, clip property changes, mixer and device
  automation, and scene tempo/signature changes. A second press or transport stop ends it.
- A7. Armed tracks record new material into the Arrangement. With punch in/out enabled the loop
  region is the recording window; input outside it is not recorded.
- A8. With loop on, each pass becomes a separate take. Takes are kept as take lanes under the
  track; the main lane shows the comp. Dragging over or clicking a take puts that range in the comp.
- A9. Recording over existing material replaces it in the recorded range; the replaced material is
  kept as a take named "Original".
- A10. *XDAW choice:* a new Session MIDI recording is rounded up to whole bars when it stops. Live
  input timing is block-accurate (error at most one audio buffer).
- A11. A count-in runs from negative bars up to the start position, then recording starts.

## Automation

- A12. Moving an automated control overrides its automation until Back to Arrangement.
- A13. Tools: Draw writes steps on the grid, Line writes a straight ramp, Erase removes points in
  a range.
- A14. Audio clips have their own gain and pan envelopes, edited in the clip editor.

## Tracks

- A15. Group tracks can contain other group tracks (no cycles). Groups can be folded.
- A16. Freeze renders a track with its devices (plus a 3 s tail) and plays the render instead of
  the live chain; edits are disabled while frozen. Flatten replaces the track with the render.

## Audio editing

- A17. Reverse writes a reversed copy of the clip's audio and points the clip at it.
- A18. Consolidate (Cmd+J) joins the selected clips of a track into one clip.
- A19. Crossfade between two overlapping neighbour clips is centred on the overlap.
- A20. Slice to New MIDI Track cuts the clip at grid lines or detected transients, loads the
  slices on a 16-pad Drum Kit and writes a MIDI clip that replays them in order.

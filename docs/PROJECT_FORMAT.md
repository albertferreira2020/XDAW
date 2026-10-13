# Project format (`.xdaw`, version 1)

A Set is a UTF-8 JSON document. The two first fields identify it:

```json
{ "format": "xdaw-project", "version": 1, ... }
```

Readers reject other `format` values and versions newer than they support (with a message), and
reject malformed JSON without touching the open Set.

## Top level

| Field | Meaning |
| --- | --- |
| `name` | Set name |
| `transport` | `bpm`, `time_signature {numerator, denominator}`, `global_quantize`, `metronome`, `count_in_bars`, `loop_enabled`, `loop_start`, `loop_length` (beats) |
| `tracks` | Audio/MIDI tracks |
| `returns` | Return tracks |
| `master` | Master track |
| `scenes` | `{id, name, color}`; every track has one slot per scene |
| `midi_mappings`, `key_mappings` | `{source, target}` mappings |
| `next_id` | Next free stable id |

## Track

`id`, `name`, `kind` (`Audio`, `Midi`, `Return`, `Master`), `color`, `mixer` (`volume_db`, `pan`,
`mute`, `solo`, `arm`, `monitor`, `sends_db[]`, `input_channel`), `devices[]`, `slots[]`
(`{clip, has_stop_button}`), `arrangement[]`, `automation[]`.

Devices: `{id, kind, params[], bypass, sample, pads[], preset_name}`. Parameter values are stored
in native units in the order of the device's descriptor list; presets store `(param_id, value)`
pairs so they survive reordering.

Clips: `{id, name, color, content, region {start, loop_start, end, looping}, launch_mode,
quantize, legato, gain_db, transpose}` where `content` is
`{"Audio": {media, warp, source_bpm, markers[]}}` or `{"Midi": {notes: [{pitch, start, length,
velocity}]}}`. All positions are quarter-note beats.

Arrangement clips: `{id, start, length, offset, fade_in, fade_out, clip}`. `offset` is the clip-local
elapsed position at `start` (non-zero after splits/trims).

Automation: `{target, points: [{beat, value}]}`; `target` is `Volume`, `Pan`, `{"Send": i}` or
`{"Device": {device, param}}`. Values are native units; interpolation is linear and the first/last
values hold outside the points.

## Media

`media.path` is relative to the `.xdaw` file's folder when the file lies inside it, otherwise
absolute. **Collect All and Save** copies external files into `<folder>/Samples/` and rewrites the
references. Missing media is reported on open; affected clips stay in the Set and play silence.

## Safety

- Save writes a temporary file in the same folder, fsyncs it and renames it over the target.
- Autosave every 30 s while there are unsaved changes, to `<set>.xdaw.autosave` (or
  `~/Library/Application Support/XDAW/untitled.xdaw.autosave` for unsaved Sets).
  On open, an autosave newer than the Set is offered for recovery.

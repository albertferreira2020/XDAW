# Roadmap

Compared against the public Live material (Session and Arrangement views, devices, recording,
warping) and our own `FEATURE_MATRIX.md`. Ordered by how much a user notices the gap.

## Done in the second milestone (see FEATURE_MATRIX.md for how each was verified)
1. Arrangement recording with punch in/out and take lanes (audio + MIDI).
2. Multi-clip MIDI editing and a per-note Chance lane (Velocity lane already existed).
3. Track folding, nested groups, Freeze / Flatten.
4. Mixer column beside the Arrangement.
5. Time ruler, locators, adaptive and triplet grid.
6. Automation drawing tools and audio clip envelopes (gain, pan).
7. Reverse, Consolidate, Crossfade, Slice to New MIDI Track (fades were already draggable).

Known gaps inside those items:
- Clip envelopes exist for audio clips only (no MIDI controller envelopes, no pitch envelope).
- Takes are one lane per pass; there is no packing of non-overlapping takes into one lane.
- MIDI recording timing is block-accurate (about 5 ms at 256 frames), not sample-accurate.
- Slicing ignores warp markers and caps at the Drum Kit's 16 pads.
- Frozen audio follows tempo like Re-Pitch; it is not re-rendered when the tempo changes.
- Reverse writes a new file and clears warp markers.

## Still to do
### Devices
Done (spec `docs/specs/devices.md`): Parametric EQ, Bus Compressor, Saturator, Motion Filter,
Chorus, Phaser, Gate, Multiband Compressor, FM Synth, Morph Synth; six MIDI effects; sidechain for
Compressor/Bus Compressor/Gate; Limiter lookahead with automatic delay compensation.

9. Racks: instrument, drum and effect racks with parallel chains, key/velocity zones and macros.
10. Remaining built-in devices: Looper, spectrum analyser, slicing sampler.
11. Gain-reduction meters on the dynamics devices (values exist in the engine, not shown yet).
12. Compensation of device latency on return tracks.
13. VST3 / Audio Unit hosting (needs a licence decision on the VST3 SDK).

## Session and performance
14. Clip launch via MIDI/key mapping UI (MIDI map mode) and controller scripts.
15. Return/Send routing UI, audio/MIDI input and output routing menus per track.
16. Tempo automation and tempo follower, Link-style sync.
17. Scene tempo/time-signature, and Session-to-Arrangement capture polish.

## Project, library and platform
18. Browser: tags/collections, preview (audition) of samples, search across folders, hot-swap.
19. Export: stems, normalize, MP3/OGG encoding; render of a selection.
20. Undo history list, project templates, and Collect All UI.
21. Windows and Linux builds (core is portable but unverified), installer/signing/notarization.
22. Accessibility and UI scale, theme options, keyboard shortcut customization.

## Needs input from the owner
- GitHub/CI for Windows and Linux runners; real hardware MIDI/microphone verification.

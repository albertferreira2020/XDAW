# Roadmap: what is still missing versus Live

Compared against the public Live material (Session and Arrangement views, devices, recording,
warping) and our own `FEATURE_MATRIX.md`. Ordered by how much a user notices the gap.

## Arrangement and editing
1. Arrangement audio/MIDI recording with punch in/out and takes (comping lanes).
2. Multi-clip MIDI editing, and per-note velocity/probability lanes in the piano roll.
3. Track folding and nested groups; track freeze/flatten.
4. Mixer strip column beside the Arrangement (volume, pan, sends, meters per track).
5. Time ruler in seconds/bars switch, locators with names, and a global grid/triplet mode.
6. Clip envelopes (clip-level automation) and automation drawing tools (pencil, curve, snap).
7. Audio editing: consolidate, reverse, fades UI, crossfades, slice to new MIDI track.
8. Warp UI: warp-marker editing in the sample editor, more modes (beats, tones, texture, complex).

## Devices
9. Racks: Instrument, Drum and Effect Racks with chains, zones and macro automation.
10. More built-in devices: EQ Eight, Glue-style compressor, Saturator, Auto Filter, Chorus,
    Phaser, Gate, Multiband, Looper, Spectrum analyser, Simpler-style slicing sampler,
    wavetable and FM synths.
11. MIDI effects: Arpeggiator, Chord, Scale, Note Length, Random, Velocity.
12. Sidechain routing on compressor/limiter, lookahead mode for the Limiter.
13. VST3 / Audio Unit hosting with plugin delay compensation.

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

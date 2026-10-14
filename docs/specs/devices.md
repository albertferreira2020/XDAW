# Built-in devices

Status: D1–D5 implemented (see `docs/FEATURE_MATRIX.md`); D6 is planned. Names and parameter sets are XDAW's own.

## D1. Chain rules

- D1.1 A track's chain is: MIDI effects (any number) → at most one instrument → audio effects.
  MIDI effects and instruments exist only on MIDI tracks. Adding a device inserts it at the end of
  its section; moving a device never crosses a section boundary.
- D1.2 Every device has a bypass toggle; a bypassed MIDI effect passes events unchanged and a
  bypassed audio effect passes audio unchanged.
- D1.3 Devices never allocate, lock or do I/O while processing. Each device's output for a given
  input and parameter set is deterministic (random devices use a seeded generator).
- D1.4 Parameters are smoothed where a jump would click (gains, frequencies, times).

## D2. Audio effects

| Device | Parameters | Behaviour |
| --- | --- | --- |
| **Parametric EQ** | 8 bands × (On, Type, Freq, Gain, Q); Output | Types: Low Cut 12 dB/oct, Low Cut 24 dB/oct, Low Shelf, Bell, Notch, High Shelf, High Cut 12, High Cut 24. Bands are in series. The UI draws the summed magnitude curve. Bands 1 and 8 start as Low Cut / High Cut (off); 2–7 as Bell (off). |
| **Bus Compressor** | Threshold, Ratio (2, 4, 10), Attack (0.1, 0.3, 1, 3, 10, 30 ms), Release (0.1, 0.2, 0.4, 0.6, 0.8, 1.2 s, Auto), Makeup, Mix, Soft Clip | Stereo-linked RMS-ish detector (1 ms smoothing), hard knee. Auto release: release time follows how long the signal stayed above threshold (fast for transients, slow for sustained material) between 0.1 and 1.2 s. Mix is parallel (dry/compressed). Soft Clip applies `tanh` scaled to never exceed 0 dBFS. |
| **Saturator** | Drive, Curve (Soft, Hard, Warm, Fold), Bias, Output, Mix | Soft: `tanh`. Hard: clip at ±1. Warm: `tanh` shifted by Bias (even harmonics), re-centred on zero and scaled so its peak stays within ±1. Fold: sine wavefolder. A 10 Hz DC blocker follows the curve. Output compensates nothing automatically. |
| **Motion Filter** | Type (LP, HP, BP), Cutoff, Resonance, LFO Amount, LFO Rate (Hz), LFO Sync (Off, 4/1, 2/1, 1/1, 1/2, 1/4, 1/8, 1/16 note values), LFO Shape (Sine, Triangle, Square, Saw, Random), Stereo Phase, Env Amount, Env Attack, Env Release, Mix | Cutoff is modulated in octaves: `cutoff · 2^(lfo·amount·4 + env·envamount·6)`. A synced LFO's phase follows the song position while the transport plays. Random holds a new value each cycle. |
| **Chorus** | Rate, Depth (ms), Delay (ms), Feedback, Width, Mix | Two modulated delay lines per side, LFOs 90° apart, linear interpolation. |
| **Phaser** | Stages (4, 8, 12), Rate, Depth, Center, Feedback, Spread, Mix | First-order all-pass chain swept around Center over ±Depth octaves; right channel LFO offset by Spread. |
| **Gate** | Threshold, Return (hysteresis dB), Attack, Hold, Release, Floor, Sidechain | Opens when the detector exceeds Threshold, closes when it falls below Threshold − Return after Hold. Closed gain is Floor dB. |
| **Multiband Compressor** | Low/High crossover; per band (Low, Mid, High): Threshold, Ratio, Gain; Attack, Release; Output | Linkwitz–Riley 4th-order crossovers (bands sum flat when unprocessed). Each band uses the compressor curve of the existing Compressor with a 6 dB knee. |

## D3. Dynamics additions

- D3.1 Sidechain: Compressor, Bus Compressor and Gate have a **Sidechain** source chooser: Self or
  any other audio or MIDI track (group tracks cannot be sources). A muted source still drives the
  detector (the tap is before the fader). The detector then listens to that track's post-device,
  pre-fader signal from the **same audio block**; tracks are processed so a source is rendered
  before the devices that listen to it. A source that would create a cycle is ignored (Self).
- D3.2 Limiter Lookahead: Off, 1.5 ms, 3 ms, 6 ms. With lookahead, gain reduction starts before the
  peak (linear ramp over the lookahead time) and the device reports that many samples of latency,
  which the mixer compensates as for any device latency.
- D3.3 Delay compensation: every audio/MIDI track is delayed (after its fader) so that its path to
  the master — its own devices plus those of the groups it feeds — matches the slowest path. Export
  removes the total latency so files line up with the timeline. *Limitation:* latency of devices on
  return tracks is not compensated.

## D4. MIDI effects

They receive the note events of a block and output transformed events. Note-offs always match the
note-ons they produced (no stuck notes), including on bypass, parameter changes and transport stop.

| Device | Parameters | Behaviour |
| --- | --- | --- |
| **Arpeggiator** | Pattern (Up, Down, Up-Down, Down-Up, As Played, Random), Rate (1/4, 1/8, 1/8T, 1/16, 1/16T, 1/32), Gate (5–100 %), Octaves (1–4, +12 semitones each), Velocity (Played / Fixed), Fixed Velocity, Hold | Steps are on the song grid while the transport plays (the first step waits for the next grid line) and on a free clock starting at the first held note otherwise. Each step plays one held note (plus octaves) for Gate % of the step. Hold keeps the last chord until a new chord starts. |
| **Chord Stack** | Shift 1–6 (−24…+24 semitones, 0 = off), Velocity scale per shift | Each incoming note also plays up to six transposed notes. Duplicates are merged. |
| **Scale Lock** | Root (C…B), Scale (Major, Minor, Dorian, Phrygian, Lydian, Mixolydian, Locrian, Harmonic Minor, Pentatonic Major, Pentatonic Minor, Blues, Chromatic), Direction (Nearest, Up, Down) | Notes outside the scale move to the nearest scale degree (ties go up unless Direction says otherwise). |
| **Note Duration** | Mode (Time, Synced), Length (ms), Length Sync (1/32 … 1/1), Release Trigger | Replaces each note's duration. With Release Trigger, notes start on the incoming note-off instead of the note-on. |
| **Pitch Randomizer** | Chance (%), Range (semitones), Choices (1–24), Sign (Bipolar, Up, Down) | Each note is, with Chance, transposed by a random multiple of Range/Choices. Seeded per device. |
| **Velocity Shaper** | Drive (curve), Out Low, Out High, Random | Maps velocity `v/127` through `x^(2^(−2·Drive))`, scales into [Out Low, Out High], then adds ± Random. |

## D5. Instruments

| Device | Parameters | Behaviour |
| --- | --- | --- |
| **FM Synth** | 4 operators × (Ratio, Level, Attack, Decay, Sustain, Release); Algorithm (8 fixed routings); Feedback; Filter Cutoff, Resonance; Volume; Voices | Sine operators; phase modulation; operator 4 (top of every modulation chain) has self-feedback. Polyphonic with voice stealing as Poly. |
| **Morph Synth** | Table (Basic, Vocal, Digital, Organ), Position, Pos Env (amount of the amp envelope added to Position), Detune, Unison (1–4), Filter, ADSR, Velocity, Volume | Each table holds 8 single-cycle frames generated by XDAW code from additive formulas. Position crossfades between adjacent frames. Band-limited by mip-mapping per octave (up to 256 harmonics). Unison voices spread over ±Detune and alternate left/right. |

## D6. Later

Racks with chains and macro-mapped chains, Looper, Spectrum analyser, slicing sampler, plugin
hosting.

# Feature matrix (Live 10 workflows → XDAW)

Status legend:

- **Verified**: implemented and checked by an automated test, the scripted UI driver, or a run on
  real hardware (evidence column).
- **Implemented, unverified**: code exists and is reachable from the UI, but no check has run.
- **Partial**: a documented subset exists.
- **Not implemented**.
- **Blocked**: needs something unavailable here.

"Reference" says where the expected behaviour comes from: **L10** = Live 10 Reference Manual
(see `docs/research/sources.md`), **Own** = XDAW's own proposal where Live 10 behaviour is unknown.

Platform: macOS 26.6.2, Apple M5, built-in output at 48 kHz. Windows/Linux: **not verified**.

## Transport and clock

| Feature | Reference | Status | Evidence / notes |
| --- | --- | --- | --- |
| Play / Stop / Stop twice → 1.1.1 / Continue (Shift+Space) | L10 §7.1 | Verified | scheduler tests; scripted UI |
| BPM 20–999, tap tempo | L10 §7.2 | Verified (BPM), unverified (tap) | `tempo_change_reschedules_boundary_musically` |
| Time signature (n 1–99, d 1/2/4/8/16) | L10 §7.2 | Verified | `time_signature_rejects_values_outside_documented_range` |
| Bars.beats.sixteenths display, negative count-in bars | L10 §16.6 | Verified (display) | `bbs_display_is_one_based_and_counts_in_negative_bars` |
| Count-in before recording | L10 §16.6 | Implemented, unverified | `PlayWithCountIn` |
| Metronome (downbeat accent, follows denominator) | L10 §16 | Implemented, unverified | not audibly checked |
| Arrangement loop | L10 §6 | Verified | `arrangement_loop_wraps_on_sample` |
| No drift over long loops | Own | Verified | 200 loops at 44.1 kHz/128 BPM within 1 sample |
| Device selection, 44.1/48 kHz, buffer 64–2048 | prompt | Implemented; 48 kHz/256 verified | headless + UI runs on built-in output |
| CPU / callback / deadline-miss display | prompt | Verified | callback histogram in `SharedStatus` |

## Session View

| Feature | Reference | Status | Evidence / notes |
| --- | --- | --- | --- |
| Tracks × scenes grid, one clip per track | L10 §7.2 | Verified | scheduler tests, screenshots |
| Clip states stopped/queued/playing/recording | L10 §7.1 | Verified (queued/playing), recording engine-verified | status atomics + screenshots |
| Global and per-clip launch quantization (None…8 Bars, 1/2…1/32) | L10 §13.3 | Verified | onset in [0,1) sample at 48k/120, 44.1k/128, 48k/97.3 |
| Launch modes Trigger/Gate/Toggle/Repeat | L10 §13.2 | Verified | `gate_stops_on_release_and_toggle_stops_on_second_press`, `repeat_relaunches_every_grid_step_until_released` |
| Legato | L10 §13.5 | Partial: launcher unit-verified | `legato_keeps_origin` |
| Clip replacement on the same sample, 2 ms crossfade | Own (I1) | Verified | `replacing_clip_switches_on_same_sample_without_gap`, `launch_transition_has_no_click` |
| Scene launch, stop buttons, add/remove stop button | L10 §7.4.2 | Verified | `scene_launch_respects_stop_buttons` |
| Stop All Clips (quantized) | L10 §7.5 | Implemented, unverified | |
| Select next scene on launch | L10 §7.2 | Implemented, unverified | |
| Scene name tempo/time signature ("96 BPM", "7/8") | L10 §7.2 | Verified (parser) | `scene_names_parse_like_live_10` |
| Capture and Insert Scene | L10 §7.4.3 | Implemented, unverified | |
| Duplicate/rename/color/move/copy clips, drag & drop | L10 §7.4 | Partial: drops verified (scripted UI); rename/color/move unverified | Browser → slot (`bass.wav` on Drums slot 1); Finder drop of 2 files → consecutive slots, scene added; drop zone → new track; undo |
| Samples dragged from Finder (OS file drop) onto slots, lanes, drop zones, Sampler/Drum Rack | Own (Live 10 §5 behaviour) | Verified (simulated OS drop through the same raw-input path); real Finder drag unverified | `filedrop.rs`; script `osdrop`; macOS pointer read via Core Graphics during the drag |
| Audio file dropped on a MIDI track loads an instrument | L10 §5 | Verified (scripted UI) | drop on Beat → "Loaded on Drum Rack pad 9" |
| Follow Actions (No Action, Stop, Play Again, Previous, Next, First, Last, Any, Other; chances A/B; group = adjacent clips) | L10 §13.6 | Verified (engine); UI controls unverified | `follow_actions_chain_clips_at_exact_times_and_stop`, sequencer follow tests; Previous wrap at group top is an inference |

## Arrangement View

| Feature | Reference | Status | Evidence / notes |
| --- | --- | --- | --- |
| Timeline, ruler, playhead, follow, snap | L10 §6 | Verified (render) | screenshot `s3.png` |
| Horizontal zoom: pinch, Cmd+wheel, +/−, slider, ruler vertical drag, Sel (Z), All (Shift+Z) | L10 §6.2 | Verified (scripted UI) | ppb 18 → 29.7 (Cmd+wheel) → 59.4 (pinch) → 47.5 (−) → fit; ruler drag 16.6 → 27.7; `timeline` unit tests |
| Horizontal scroll bar (drag thumb, drag ends to zoom, click trough), Shift/horizontal wheel | Own | Verified (scripted UI) | thumb drag scroll 9.8 → 18.0; wheel scrolls; manual scroll while playing turns Follow off |
| Track height (slider, Alt+wheel) | L10 §6 | Verified (scripted UI) | 56 → 63.1 |
| Clip names stay visible when a clip starts left of the view | L10 §6 | Verified (render) | screenshot |
| Piano roll and sample editor zoom/scroll bars | L10 §9/§10 | Verified (scripted UI) | piano + key; waveform ppb 40 → 205.7 and scroll; screenshot |
| Waveforms and MIDI note previews | L10 §6 | Verified (render) | screenshot |
| Move, trim, split, duplicate, copy/paste, delete clips | L10 §6 | Implemented, unverified (UI interactions) | `clear_arrangement_range` model test |
| Fades (in/out handles) | L10 §6 | Implemented; playback math covered by edge declick | |
| Session ↔ Arrangement exclusivity, Back to Arrangement | L10 §7.5 | Verified | `session_launch_overrides_arrangement_until_back_to_arrangement`, `clip_stop_button_silences_arrangement_track` |
| Record Session performance into Arrangement | L10 §7.5 | Verified (scripted UI) | journey: clip counts `[2,1,1,1,2,1,1,1]` → `[5,4,3,2,5,3,3,5]` |
| Punch in/out, arrangement audio recording | L10 §16 | Not implemented | audio recording goes to Session slots |

## MIDI and recording

| Feature | Reference | Status | Evidence / notes |
| --- | --- | --- | --- |
| Piano roll: add/delete/move/resize, multi-select, velocity lane, quantize, transpose, loop duplication | L10 §10 | Implemented, unverified (UI interactions); renders verified | screenshot `s2.png` |
| Note-off/sustain/all-notes-off, no stuck notes on stop/relaunch/arm-off | prompt | Verified | `stopping_transport_leaves_no_stuck_notes`, `clip_stop_and_relaunch_send_note_offs`, synth sustain test |
| Sample-accurate MIDI clip notes across loops | Own | Verified | `midi_note_onsets_are_sample_accurate_across_loops` |
| Physical MIDI input (midir) | prompt | Implemented, unverified | no MIDI device attached during testing |
| Computer MIDI keyboard (A–K rows, Z/X octave, C/V velocity) | L10 §27 | Implemented, unverified | |
| Session MIDI recording and overdub | L10 §16.3 | Verified (engine); UI clip creation unverified | `midi_session_record_captures_notes_with_beats` (timing error ≤ one block) |
| Audio recording from input, monitoring In/Auto/Off, written by disk thread | L10 §16 | Verified with simulated input; real microphone unverified | `audio_session_record_streams_input_to_wav` |
| Input clipping indicator | prompt | Partial | input peak tracked; no dedicated UI indicator yet |
| MIDI mapping / key mapping (learn mode) | L10 §27 | Implemented, unverified | |
| Capture MIDI (retrospective) | L10 §16.8 | Verified (capture logic); UI button unverified | `crates/ui/src/capture.rs` tests. History: last 2048 notes / 10 min per track; phrase = notes without a gap > 2 bars; keeps song positions if played with transport running, else current tempo from the first note; no tempo detection |

## Mixer, instruments and effects

| Feature | Reference | Status | Evidence / notes |
| --- | --- | --- | --- |
| Volume, pan (constant power), mute (activator), solo (in place), arm, meters, clip indicator | L10 §15 | Verified (vol/pan/mute/solo/sends); meters visual only | `sends_reach_returns_and_mute_solo_gate`, `solo_silences_other_tracks_but_not_returns`, pan law test |
| Sends (post-fader) and return tracks | L10 §15 | Verified | same |
| Group tracks (one level; group bus with devices/mixer/sends; group slots launch members; solo through group) | L10 §15.3 | Verified (engine); UI unverified | `group_tracks_sum_members_through_their_mixer`; nesting and folding are not implemented |
| Feedback-free routing | prompt | Verified by construction | returns cannot send; `Project::validate` |
| Latency compensation | prompt | Not needed yet | all built-in devices are zero-latency |
| Poly synth (2 PolyBLEP osc, SVF, 2 ADSR, 16 voices) | Own | Verified | voice stealing, sustain, release tests |
| Sampler (Hermite interpolation, ADSR, root key) | Own | Verified | `sampler_plays_at_root_rate_and_octave_up` |
| Drum Rack (16 pads, C1–D#2) | Own | Verified | `drum_pad_maps_note_to_pad` |
| Utility, EQ Three-Band, Filter, Compressor, Echo Delay (synced), Room Reverb | Own | Verified (signal tests) | silence, gain, filter response, compressor curve, delay timing, reverb decay, NaN recovery |
| Limiter (gain, ceiling, release; zero latency, brickwall) | Own | Verified (signal + export tests) | output never above ceiling, quiet audio untouched, release recovers, master export peak held at -1 dB |
| Factory presets for Utility, EQ Three-Band, Limiter | Own | Verified (data/engine tests); UI menu not exercised by script | ids/ranges valid, applied via device `Presets` menu and Browser > Presets (drag or double-click) |
| Device order, bypass, presets (JSON, id-keyed) | prompt | Implemented, unverified (UI) | |
| Parameter smoothing | prompt | Verified | `smoothing_reaches_target_and_bounds_step` |
| Racks with macros | L10 | Partial (macros only) | `macros_drive_mapped_parameters_in_normalised_space` |

## Automation and warping

| Feature | Reference | Status | Evidence / notes |
| --- | --- | --- | --- |
| Envelopes for volume/pan/sends/device params, linear interpolation | L10 §19 | Verified (playback) | `automation_drives_volume_and_manual_change_overrides` |
| Manual change overrides automation; Back to Arrangement re-enables | L10 §19 | Verified | same |
| Envelope editing (add/move/delete points) | L10 §19 | Implemented, unverified | |
| Automation recording during Arrangement Record | L10 §19.1 | Implemented, unverified | |
| Warp "Re-Pitch" (tempo follows by resampling; pitch changes) | L10 §9 | Verified | loop drift test |
| Warp "Off" | L10 §9 | Implemented, unverified | |
| Warp markers (piecewise beat → source time; Re-Pitch and Stretch) | L10 §9 | Verified (engine); marker editing UI unverified | `warp_markers_remap_source_time_in_repitch_and_stretch`, sequencer warp tests; no automatic transient/tempo detection |
| Pitch-preserving time stretching (warp mode "Stretch") | L10 §9 | Verified (DSP + engine) | WSOLA, 2048-sample Hann frames, ±512 search, offline on the media worker; `stretch_changes_length_but_not_pitch` (pitch ±1 %, exact length), `stretch_clip_uses_pitch_preserving_rendering_when_available`. Limits: transient smearing/doubling at large ratios, ratio 0.25–4, Re-Pitch fallback while rendering after tempo changes, memory per tempo |

## Files and projects

| Feature | Reference | Status | Evidence / notes |
| --- | --- | --- | --- |
| Import WAV | prompt | Verified | demo media, recording test |
| Import AIFF, FLAC | prompt | Verified (decoder) | `decodes_wav_aiff_and_flac_identically` (fixtures made with macOS afconvert) |
| MIDI file import/export | prompt | Verified | `round_trip_notes_and_tempo` |
| WAV export 16/24-bit PCM, 32-bit float, 44.1/48 kHz, range, tail, TPDF dither, clipping count | prompt | Verified (16/24-bit, 32f) | headless export, journey export, `pcm16_export_round_trips_with_dither_bound`, `export_matches_realtime_render_of_the_same_state` |
| Versioned format, atomic save, reopen | prompt | Verified | `round_trip_preserves_everything`, journey |
| Invalid/newer file rejection | prompt | Verified | `invalid_and_future_files_are_rejected_with_reasons` |
| Autosave and crash recovery | prompt | Verified (model) ; UI dialog unverified | `autosave_newer_than_project_is_offered_for_recovery` |
| Relative media paths, Save As rebasing, Collect All and Save, missing media | prompt | Verified | `missing_media_reported_and_collect_copies_into_project`, journey (bug found and fixed) |
| Undo/redo (reversible edits, gesture-coalesced) | prompt | Verified | history test, journey |
| `.als` interoperability | prompt | Not investigated | no example files provided |

## Advanced

| Feature | Status |
| --- | --- |
| Flatten a track's Arrangement to a new audio track (post-device render) | Implemented, unverified (UI) |
| Macro controls (8 per track, each mapped to any device parameters with ranges) | Partial: verified (model); UI unverified; no rack chains, no macro automation |
| Instrument/effect rack chains, multi-clip MIDI editing, reversible Freeze | Not implemented |
| VST3 / Audio Unit hosting | Not implemented (`plugin-host` placeholder) |
| Max for Live, Push | Not implemented; require their own compatibility projects |

## Reverse engineering of Live 10

| Item | Status |
| --- | --- |
| REA CLI available, workflows present | Verified (REA 6.4.0) |
| Live 10 installation, version/edition/hash | **Blocked**: not installed on this Mac |
| Static analysis provider (Hopper/Ghidra/IDA) | **Blocked**: none configured; `rea setup` needs user approval |

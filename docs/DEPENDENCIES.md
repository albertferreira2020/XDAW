# Dependencies

Pinned by `Cargo.lock`; versions checked on crates.io on 2026-10-13. APIs were verified against
the sources in the local cargo registry.

| Crate | Version | License | Use |
| --- | --- | --- | --- |
| cpal | 0.18.2 | Apache-2.0 | Audio device I/O (CoreAudio on macOS) |
| midir | 0.11.1 | MIT | MIDI input |
| midly | 0.5.3 | Unlicense | Standard MIDI File read/write |
| eframe / egui | 0.36.2 | MIT OR Apache-2.0 | Native UI |
| rfd | 0.17.2 | MIT | Native file dialogs |
| symphonia (wav, aiff, flac, pcm) | 0.6.1 | MPL-2.0 | Audio decoding |
| hound | 3.5.1 | Apache-2.0 | WAV writing (export, recording) |
| rtrb | 0.4.0 | MIT OR Apache-2.0 | Lock-free SPSC rings to/from the audio thread |
| crossbeam-channel | 0.5.17 | MIT OR Apache-2.0 | Worker channels (media loader, recorder) |
| serde / serde_json | 1.0.229 / 1.0.154 | MIT OR Apache-2.0 | Project and preset files |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | Error types |

Transitive dependencies (466 packages) are permissive (MIT, Apache-2.0, BSD, Zlib, ISC, Unicode,
BSL-1.0, Unlicense) except: **MPL-2.0** (symphonia family; file-level copyleft — used unmodified,
source available from crates.io) and a few dual-licensed crates where XDAW uses the permissive
option (`MIT OR Apache-2.0 OR LGPL-2.1-or-later`, `Apache-2.0 OR GPL-2.0-only`). egui's default
fonts ship under OFL-1.1 / Ubuntu Font License.

Regenerate the summary with:

```bash
cargo metadata --format-version 1 | python3 -c "import json,sys,collections; d=json.load(sys.stdin); print(collections.Counter(p.get('license') for p in d['packages']))"
```

No proprietary SDKs are used. VST3/AU hosting (not implemented) would require evaluating the
Steinberg VST3 SDK license and Apple's AudioToolbox APIs separately.

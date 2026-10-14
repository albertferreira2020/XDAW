# Clean-room policy and provenance record

This file has two parts:

1. **Provenance.** A factual record of what third-party material was and was not used to build
   XDAW up to commit `d188377`, the last commit before this policy.
2. **Policy.** The rules every contribution must follow from now on.

It is an engineering record, not legal advice. Before commercial release or if a claim is
received, have it reviewed by an intellectual-property lawyer in the relevant jurisdictions.

## 1. Provenance record (2026-10-13, up to commit `d188377`)

### How it was built

XDAW was written by Albert de Almeida Ferreira with an AI coding assistant (Claude, Anthropic).
The git history (`git log`) is the timeline; every commit is in the public repository.

**Honest limit:** this phase was **not** a formal two-team clean room. The same assistant session
read the public documentation, wrote the behaviour notes (`docs/research/`) and wrote the code.
The assistant also has general background knowledge of commercial DAWs from its training. The
safeguards below reduce the risk; they do not substitute for team separation.

### What was used

| Material | How it was used | Evidence |
| --- | --- | --- |
| Live 10 Reference Manual, public web pages (Internet Archive snapshots, 2019) and the public product page `ableton.com/en/live/what-is-live/` | Read to learn **functional behaviour** (what a feature does), summarised in XDAW's own words | `docs/research/sources.md` (URLs, SHA-256 of each page). Manual text is not committed |
| Open-source crates | Linked as dependencies under their licenses | `docs/DEPENDENCIES.md`, `Cargo.lock` |
| Audio in `demo/` and `crates/engine/tests/fixtures/` | Synthesized by XDAW's own code (`crates/engine/src/demo.rs`); `.aif`/`.flac` fixtures converted from the generated WAV with macOS `afconvert` | source code, `recording.rs` comment |
| Screenshot in `docs/images/` | A capture of XDAW itself | file |

### What was not used

| Material | Status | Evidence |
| --- | --- | --- |
| Ableton Live binaries, installers, libraries or plugins | **Never present** on the development machine; nothing was disassembled, decompiled, traced or debugged | `docs/research/00-environment-and-tools.md` (search of `/Applications`, `~/Applications` and Spotlight) |
| Ableton source code, SDKs, headers or private APIs | Never accessed | — |
| Ableton project/preset/rack formats (`.als`, `.alc`, `.adg`, `.adv`, `.alp`) | Not read, written or reverse engineered | `FEATURE_MATRIX.md` (".als interoperability: Not investigated") |
| Ableton samples, packs, presets, instruments, impulse responses | None used. XDAW presets are its own values (`crates/dsp/src/presets.rs`) | — |
| Ableton graphics, icons, fonts, colour themes, screenshots | None used. XDAW's palette is its own (`crates/ui/src/theme.rs`: graphite with teal accent); fonts are egui defaults (OFL/UFL) | — |
| Manual text, figures or screenshots | Not copied into the repository | `sources.md` stores only hashes |
| Proprietary licence or activation mechanisms | Not inspected or altered (explicit project rule) | `docs/PROGRESS.md` |

### What XDAW shares with the reference product

Only **functional ideas** and generic industry terminology: a clip grid with scenes, a linear
timeline, launch quantization, follow actions, take lanes, freeze/flatten, automation, warping.
These ideas also exist in other DAWs. XDAW's code, data model, file format (`.xdaw`, JSON),
DSP algorithms, presets, sounds, palette, icons and name are its own.

### Corrective changes made with this policy

- Code comments no longer cite the third-party manual. They cite XDAW's own specifications in
  `docs/specs/`, which are written as XDAW requirements in XDAW's own words.
- The device formerly named "Drum Rack" (a vendor-specific product name) is now **Drum Kit**.
  Old project files still load (`#[serde(alias = "DrumRack")]`).
- README no longer presents XDAW as a clone of another product. `NOTICE` carries a
  trademark/non-affiliation statement. `LICENSE-MIT` and `LICENSE-APACHE` were added.
- `crates/project/tests/clean_room.rs` fails the build if the code or specs mention the
  third-party vendor or product, its manual, or its file formats (see §2.5).

The git history was **not** rewritten: earlier commits remain as an honest record of how the
project was built.

## 2. Policy for all future work

### 2.1 Roles

- **Research ("dirty") role.** Reads public documentation of third-party products, and runs
  controlled experiments on a legally licensed copy only if the user explicitly authorizes it.
  Writes findings in `docs/research/` as functional behaviour only: inputs, outputs, ordering,
  limits. No quoted text, no screenshots, no internal implementation details.
- **Spec role.** Turns research notes into XDAW requirements in `docs/specs/`, in XDAW's own
  terms, marking XDAW's own decisions. A spec must not reference a third-party product.
- **Implementation ("clean") role.** Reads **only** `docs/specs/`, XDAW code and open-source
  dependencies. It never reads third-party manuals, products, screenshots or `docs/research/`
  while implementing.

When people are available, research and implementation are done by different people or
different AI sessions, and the hand-off is the spec file in a reviewed commit. When one person
or session must play both roles, record that in the commit message (`Clean-Room: single-role`)
so the limitation stays visible.

### 2.2 Never allowed

- Copying, translating or closely paraphrasing third-party code, documentation text, UI layout
  measurements, graphics, icons, fonts, colour themes, sounds, presets or demo content.
- Disassembling, decompiling, debugging or tracing third-party binaries. If interoperability ever
  requires it, stop and get a legal opinion for the jurisdiction first; record the decision here.
- Reading or writing proprietary file formats without a documented legal basis.
- Inspecting, circumventing or altering licence, activation or copy-protection mechanisms.
- Using third-party product or feature names that are trademarks or brand-specific
  (for example "Live", "Push", "Drum Rack", "Simpler", "Operator", "Max for Live") as XDAW names.
  Use generic terms (Session, Arrangement, Sampler, Drum Kit, Clip, Scene, Follow Action, Warp).
- Marketing XDAW as a "clone", "replacement" or "compatible version" of a named product.

### 2.3 Allowed

- Implementing functional behaviour described in public documentation, written as XDAW's own spec.
- Generic industry terms and conventions shared across DAWs (MIDI, piano roll, BPM, bars/beats,
  quantize, send/return, freeze, comping).
- Open-source dependencies with licences listed in `docs/DEPENDENCIES.md`.
- Naming another product in `docs/research/`, `docs/CLEAN_ROOM.md` or `NOTICE` for provenance
  only.

### 2.4 For each feature

1. Research note (if any) in `docs/research/`, with sources and hashes in `sources.md`.
2. Spec in `docs/specs/`, reviewed before implementation depends on it.
3. Implementation citing the spec (`spec: docs/specs/<file>.md`).
4. Commit trailer: `Spec: docs/specs/<file>.md` and, if applicable, `Clean-Room: single-role`.

### 2.5 Automated check

`cargo test -p xdaw-project --test clean_room` scans `crates/`, `docs/specs/`, `scripts/`
and `README.md` for third-party vendor, product, manual and file-format references. Add new terms
to its list when a new reference product is researched.

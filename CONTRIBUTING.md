# Contributing to XDAW

Thank you for helping. XDAW is developed under a clean-room policy; read
[docs/CLEAN_ROOM.md](docs/CLEAN_ROOM.md) before your first change.

## Rules

- Implement against `docs/specs/` and XDAW's code only. Do not consult third-party DAW code,
  binaries, decompiled output, screenshots or manuals while implementing.
- Never contribute third-party code, text, graphics, icons, fonts, themes, sounds, presets or
  file-format readers/writers. Do not use vendor-specific product names for XDAW features.
- Do not touch licence, activation or copy-protection mechanisms of any software.
- New dependencies must have a licence compatible with MIT OR Apache-2.0 and be listed in
  `docs/DEPENDENCIES.md`.
- Behaviour research goes to `docs/research/` (sources plus SHA-256, functional facts only, in
  your own words). Specs go to `docs/specs/`. They are reviewed before code depends on them.

## Commits

Sign off every commit (`git commit -s`). This certifies the
[Developer Certificate of Origin](https://developercertificate.org/): you wrote the change or
have the right to submit it under the project licence. Add trailers when they apply:

```
Spec: docs/specs/<file>.md
Clean-Room: single-role
```

Use `Clean-Room: single-role` when the same person or AI session both researched the reference
behaviour and wrote the code.

## Checks

```bash
cargo test --workspace
```

`crates/project/tests/clean_room.rs` fails if code, specs or README reference third-party
products, their manuals or their file formats.

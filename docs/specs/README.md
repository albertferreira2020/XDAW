# XDAW functional specifications

These files are the only behaviour reference for XDAW's implementation. Code comments cite them
(`spec: docs/specs/<file>.md`), never third-party manuals, products or screenshots.

Rules for these files (see [../CLEAN_ROOM.md](../CLEAN_ROOM.md)):

- They describe **what** XDAW does (inputs, outputs, ordering, limits), in XDAW's own words and
  terms. They never quote or paraphrase third-party documentation text, and never describe how
  another product is implemented internally.
- They contain no third-party code, data, images, presets, samples, file formats or UI layouts.
- Every change is reviewed by someone other than its author before implementation depends on it.
- Values marked *XDAW choice* are decisions of this project, not observations of another product.

# OLUT surface constructor experiment O

**STATUS: RESEARCH, 2026-10-05.** Default-off child of N (`9312854d`). This probe does not
implement output LUT processing or working display initialization.

N's validated recovered Windows 580.88 journal advances past the first ILUT and TMO constructors.
The next static lead is the per-head output LUT descriptor from public `POSTCOMP_HEAD_HDR_CAPB`.
The OLUT buffer count `__SIZE_1` is 1, so the absent-SFCLOAD buffer-count guard passes. The candidate mismatch
is populated pointer slots versus the zero-pointer requirement with SFCLOAD absent, analogous to TMO.

`KF3_DISPLAY_OLUT_CONSTRUCTOR_PROBE=1`, read once while constructing the display plane:

- Forces the M+N ILUT+TMO constructor page, then sets only the compiler-derived
  `POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD_TRUE` field for each advertised head.
- Forces `Engine::new_constructor_probe`. All display DMA and cursor-PIO methods remain refused before execution or completion;
  this flag cannot select the normal engine.
- Requires a generated one-bit field, nonzero in-range TRUE enum, bounded array count,
  aligned offset and full-word page containment. Missing/invalid definitions fail realization.

The flag accepts only the string `1` and is host-controlled.

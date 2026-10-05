# Display ILUT constructor experiment

**STATUS: RESEARCH, 2026-10-05.** Default off, based on the TMO-only experiment
`b431aeaf`. This cannot provide a working display or demonstrate input-LUT processing.

**Follow-up, 2026-10-05:** M passes the first ILUT constructor and fails at the following TMO
constructor in the recovered journal. [Experiment N](TMO_SURFACE.md) adds a separate default-off
TMO surface-loading discriminator while preserving the same immutable all-method refusal.

Windows580.88's pinned per-window constructor rejects a two-buffer descriptor
when its ILUT surface-loading flag is clear. The caller derives that flag from
public `PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_SFCLOAD` and unconditionally requests two
buffers. Kayfabe's ordinary CAPB remains zero. Exact public-source definitions,
pinned driver hash/RVAs, other guards and operational limitations are in the
[reviewed source/caller analysis](https://github.com/reindertpelsma/kayfabe/blob/667b3ae9/tools/windows-boundary-compare/CONSTRUCTOR.md).
This is a source-derived hypothesis, not a native capability table copied into the product.

`KF3_DISPLAY_ILUT_CONSTRUCTOR_PROBE=1` at display-plane construction:

- Selects the existing TMO constructor page, then sets only each advertised
  window's compiler-derived `ILUT_SFCLOAD_TRUE` field. DIRECT, size/count exponents,
  precisions, and all other words remain unchanged from the TMO-only probe.
- Forces `Engine::new_constructor_probe` even when the TMO environment flag is
  absent. All DMA and cursor-PIO display methods remain refused before decoding,
  enqueueing, assembly/armed mutation, acquire handling or completion. This cannot
  select the ordinary engine with the diagnostic capabilities.
- Validates the source field/TRUE value, array/count and full-word page bounds.
  Missing definitions fail display realization; no zero-offset or captured fallback.

The flag is host-only, read once at construction, and accepts only the string `1`.
Both flags absent select the original caps and engine. TMO alone selects the old
TMO-only probe. ILUT alone or both flags select TMO+ILUT with the same blanket
method refusal. There is no runtime promotion or method/control allowlist change.
Runners must clear inherited flags and record explicitly requested flags.

This deliberately advertises an unimplemented capability solely to distinguish
a constructor guard, with all display-method execution disabled. It departs from
normal operational capability semantics and is not a production workaround.
Source `SET_CONTEXT_DMA_ILUT`, `SET_OFFSET_ILUT`, and `SET_ILUT_CONTROL` describe
real LUT binding/processing obligations. Those are not implemented by this patch.
Native values or successful initialization would not justify enabling normal
method execution. Existing display-context creation/import and optional console
copies remain; the refusal claim concerns channel methods, not every GPU action.

Compatibility uses the existing exact generated display tables:580.65.06 and
580.159.04, all C573/C673/C773/CA73 family rows. Chips without display retain their
existing no-display behavior. No host-driver, guest-driver or GPU-die gate/table
is added. This is source coverage across eight cells, not hardware qualification.

Validation: all94 `kf-disp` library tests pass, including full-page equality
except the derived ILUT field in all eight cells, unchanged ordinary pages,
missing/invalid definitions, and the inherited all-method refusal/lifetime tests.
`cargo check -p kf-qemu --lib` also passes. Hardware is still pending; the full
hardware/isolation merge bar is not claimed.

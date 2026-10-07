# TMO surface constructor experiment N

**STATUS: RESEARCH, 2026-10-05.** Default-off child of M (`d2c7ca1b`). This probe does not
implement tone mapping, LUT processing or working display initialization.

M's [validated recovered Windows 580.88 journal](https://github.com/reindertpelsma/kayfabe/blob/f7ad0585/tools/windows-debug-capture/watchdog-m/evidence-m/README.md) advances past the first ILUT constructor and fails at the
second TMO constructor. The [pinned dataflow audit](https://github.com/reindertpelsma/kayfabe/blob/d36f41ec/tools/windows-boundary-compare/M_DATAFLOW.md)
documents the exact driver hash, product publication path and reviewed RVAs. Unlike ILUT's
two-buffer descriptor, TMO requests one buffer. With TMO surface loading absent, the constructor
requires empty pointer slots even though its caller fills the TMO backing-pointer array. No live
pointer or exact inner guard was recovered; this is a source-derived discriminator, not a proven
fix or a reason to copy native capability bits.

`KF3_DISPLAY_TMO_SURFACE_CONSTRUCTOR_PROBE=1`, read once while constructing the display plane:

- Forces the M ILUT+TMO constructor page, then sets only the compiler-derived
  `PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD_TRUE` field for each advertised window. All DIRECT,
  LOGSZ/LOGNR, precision and other page fields remain unchanged from M.
- Forces `Engine::new_constructor_probe` even when neither older constructor flag is set.
  All display DMA and cursor-PIO methods remain refused before execution or completion;
  this flag cannot select the normal engine.
- Requires a generated one-bit field, nonzero in-range TRUE enum, bounded array count,
  aligned offset and full-word page containment. Missing/invalid definitions fail realization.

The flag accepts only the string `1` and is host-controlled. With it absent, ordinary, TMO-only
and M selections remain unchanged. A runner must clear inherited constructor flags and record
explicit selections; the experiment runner is maintained separately by the hardware owner.
No scheduling/control acceptance, host RM verbs, input pointers or new GPU work are introduced.
Existing display-context creation/import and optional console copies are unchanged; the blanket
refusal applies to display-channel methods, not every pre-existing GPU operation.

This intentionally advertises an operational capability solely for a constructor experiment
while refusing its methods. It deviates from normal production capability semantics and is not
eligible for default enablement. A successful constructor would still require a separate design
and qualification before supporting real TMO LUT binding, data processing, updates and lifetimes.

Compatibility remains exactly the existing display cells: guest source tables 580.65.06 and
580.159.04, each with C573/C673/C773/CA73 family rows. The mask, TRUE enum and offsets come from
those generated tables; no die-specific or native captured constants are introduced. Unknown
driver/class layouts refuse. Displayless-chip behavior is unchanged. Source coverage over eight
cells is not hardware qualification across those families or either driver axis.

Validation covers the entire 4096-byte page: relative to M, only the generated TMO surface-load
fields differ in all eight cells, and ordinary/M pages remain unchanged. Missing fields/enums,
out-of-page/misaligned arrays, insufficient counts, malformed field widths and invalid TRUE
values refuse. The existing immutable all-method-refusal tests remain required.

Validation result: all96 `kf-disp` library tests pass and `cargo check -p kf-qemu --lib`
passes. Hardware testing is pending; no complete hardware or hostile-isolation merge bar is claimed.

# Bounded pre-composition tone mapping

**STATUS: RESEARCH, 2026-10-06 — direct Linux zero-intensity fixture qualified at `74cd590c`; broader work incomplete.**
Owner-directed continuation on `codex/sdr-lut-20261006`, within the existing
opt-in `KF3_DISPLAY_SDR_COLOR=1`; default remains off. No Windows success,
master promotion, full HDR or physical post-LUT parity claim.

The corrected strict Linux run B failed because our capability page declared
TMO absent. The guest DRM property list was queried live, but OGKM constructs
that list from our virtual capabilities. Its `SetHDRLayerCaps()` maps
`tmoPresent` to `supportsICtCp`; `nv_drm_plane_install_properties()` gates the
plane TMO property on that support and extended DRM properties.

The owner suggested checking whether the fallback itself is shared with
Windows before attributing its failure to missing TMO. The identified fallback,
`nvNeedsTmoLut()`, is in NVKMS. The local OGKM `nvidia-modeset.Kbuild:24` says its
core is shared across Unix platforms, and `src/nvidia-modeset/Makefile:214`
identifies a Unix driver. That does not establish Windows uses this fallback.
Conversely, absence of that evidence does not prove TMO caused Code 43. Shared
hardware-class contracts remain useful; Windows behavior needs a direct retest.

**Owner scope clarification:** unsupported table formats and chroma-correction
controls must gain real processing support; leaving those refusals is not the
requested endpoint. The implementation below is an intermediate testable
checkpoint, not completion of that broader work. No success no-op substitutes
for missing processing.

## Implementation under test

The opt-in page declares TMO present, one surface-loaded 1024-interval table.
Bindings, offsets, fields and register addresses are compiler-derived from OGKM
for every family. Turing through Ada use owning-channel context DMAs and
256-byte offsets; GB20x uses its physical vidmem address vocabulary.

The worker plans CSC00 -> CSC0 LUT -> CSC01 -> TMO -> CSC10 -> CSC1 LUT -> CSC11
before blend. Signed matrices and the two indexed inline tables are held in
bounded assembly/armed banks, with promotion only at UPDATE. Entries and
segments retain their indexed values rather than just the last register write;
channel free/reallocation discards the banks. The CSC0 table uses 33 logarithmic
segments; CSC1 uses 64 linear segments. Initialization and summed sample counts
are checked before upload, with a fixed 1025-entry ceiling.

The GPU performs the matrices and interpolation. TMO applies to intensity
(the middle component in OGKM's Ct/I/Cp ordering), not independent RGB gamma.
Variable per-zone sample counts are decoded and bounded on the GPU.
Only OGKM's `TMO_LUT_SETTINGS_NO_CORRECTION` controls are currently supported. Other chroma
policies refuse. TMO is a 64-segment linear UNORM16 table with 65..1025 sample
entries including the endpoint, plus four header entries. The register-authored
extent bounds both the DMA span and GPU snapshot copy; allocations retain the
fixed maximum size. Headers whose sample count exceeds/mismatches that declared extent or unequal intensity
channels are rejected on the GPU against the same immutable snapshot used for
processing. Existing input-table FP16 validation remains in force.

There are 65 fixed LUT snapshot slots (32 ILUT, 32 TMO, one OLUT) and 32 fixed
register-program upload slots, with the existing 4K frame ceiling. Ownership,
incarnation and UPDATE invalidate tone snapshots independently of the input
LUT. The existing real GPU completion barrier holds publication, notifiers and
GET advancement; no CPU pixel transform or forged completion is introduced.

## Current evidence and remaining verification

Product `74cd590cec952e1b206d54532345ba0d481d0cd4` passes the strict Linux TMO
fixture: a retained zero curve, accepted real atomic commits, nonzero TMO armed
at capture, black transformed output and byte-identical restoration. GPU SDR
and TMO fixtures and 9/9 gates (11/11 USER births) pass.
[Evidence and limits](../../traces/tmo_stage_20261006/README.md). This result does
not complete broader table-format/chroma controls or establish Windows behavior.

## Verification procedure

GPU-free planner/engine tests and synthetic GPU fixtures precede the strict
`linux_color_audit.py --sdr-color --require-tmo` run. The fixture constants are
reproducibly derived from pinned OGKM source by `derive_tmo_fixture.py`; no
per-die capture is introduced. That synthetic test does not establish native
post-LUT parity or exact hardware rounding. Logarithmic segment indexing is
inferred from the source table abscissae; independent native validation remains
required before broader colour claims.

A Linux pass must show accepted real KMS submission, a retained curve, a nonzero
armed TMO binding at capture, changed zero-intensity output and byte-identical
restoration. A compositor callback alone cannot pass. After real processing is
qualified, retest Windows; additional initialization walls remain possible.

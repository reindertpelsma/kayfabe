# Bounded pre-composition tone mapping

**STATUS: RESEARCH, 2026-10-07 — direct Linux zero-intensity fixture qualified at `74cd590c`;
segmented ILUT/OLUT, OLUT normalization and 610's no-correction SAT_MODE implemented
GPU-free on `claude/colour-chroma-20261007`, hardware UNRUN; general chroma correction
still refused (no source semantics).**
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

**Correction, 2026-10-07 (folded above the text it corrects):** the refusals of
segmented ILUT/OLUT, nonunity OLUT normalization and SAT_MODE 3 described below
(*Implementation under test*, *2026-10-06 extension evidence*, *DIRECT8 checkpoint*)
are lifted by the GPU-free implementation in the 2026-10-07 section at the end.
Its GPU fixture has **not run**. Arbitrary chroma-correction programs still refuse.

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

## 2026-10-06 extension evidence

At product `8dde9b51d`, the real-GPU oracle passes nonuniform segments, compact
65-sample tone tables, malicious extent mismatch and existing SDR/TMO fixtures.
[Evidence](../../traces/tmo_stage_20261006/README.md) includes the Windows run13
result at `74cd590c`: Code43 persists with 8,330 methods but no UPDATE/scanout;
the first changed saved assertion moves from `0x169d836` to `0x1a103e6`.
Chroma policies beyond OGKM's no-correction program remain unresolved: the
public class fields provide bit widths, while this OGKM version only programs
the no-correction constant. No transfer equation for SAT_MODE and the weighted
zones has been established. The later DIRECT8 checkpoint below removes that format refusal. Segmented
ILUT/OLUT remains implementation work; these limits do not close the owner goal.

## DIRECT8 implementation checkpoint, 2026-10-06

The next source extension decodes DIRECT8 with 257 samples (256 plus endpoint)
and a 261-entry/2088-byte total extent. DIRECT10 retains 1025 samples. GPU
lookup and validation receive the authored count; input UNORM8 selects its byte
index in DIRECT8 rather than shifting to ten bits. GPU fixtures use a compact
input with distinct index255 values and a compact output ramp. Segmented
ILUT/OLUT and chroma-correction policies remain open. Nouveau's
[window implementation](https://github.com/torvalds/linux/blob/master/drivers/gpu/drm/nouveau/dispnv50/wndwc57e.c)
independently uses DIRECT8 for 256 entries and adds four header entries and an
interpolation endpoint. At product `00220cc6`, all eight SDR GPU fixtures pass on the borrowed RTX4070,
including both compact-index tests ([log](../../traces/tmo_stage_20261006/direct8-00220-color-sdr.log)).
This is a synthetic GPU result; no native DIRECT8 display parity claim.

## Segmented tables, OLUT normalization and chroma controls, 2026-10-07

**GPU-free checkpoint on `claude/colour-chroma-20261007` (from `a6f84d0d`). The GPU
fixture `crates/kf-cuda/examples/color_vss.rs` is written but UNRUN; the borrowed host
was reserved. Nothing below is a hardware claim.** Same opt-in `KF3_DISPLAY_SDR_COLOR=1`;
the capability page is unchanged (default off).

### Source inventory (OGKM 580.159.04 `b81d58e`, 610.43.02 `57130a2`)

| control | source semantics | guest-reachable via | before | now |
|---|---|---|---|---|
| ILUT `MODE_SEGMENTED` | FP16, VSS **linear 64** segments, ≤1025 entries (`nvkms-evo3.c:6055`); `SIZE = 4 + 507 + 1` for PQ EOTF (`:4534`, `:4712`) | DRM plane `NV_PLANE_DEGAMMA_TF=PQ` (`nvidia-drm-crtc.c:582`), attached whenever the hard-coded Turing+ VSS ILUT cap holds (`:2666`); NVKMS HDR flips | refused (and the refusal stopped the display) | GPU lookup |
| OLUT `MODE_SEGMENTED` | UNORM16, VSS **logarithmic 33** segments (`:6085`); `SIZE = 4 + 336 + 1` (`:5303`, `:5482`) | DRM CRTC `NV_CRTC_REGAMMA_TF=PQ` (`nvidia-drm-crtc.c:901`); NVKMS HDR heads | refused | GPU lookup |
| `HEAD_SET_OLUT_FP_NORM_SCALE` | UNORM32, `0xffffffff` = 1.0; DRM writes `0xffffffff / regamma_divisor` (`nvidia-drm-crtc.c:2542`); NVKMS `/125` for its [0,125] PQ path (`nvkms-evo3.c:5391`, comment `:4515`) | DRM `NV_CRTC_REGAMMA_DIVISOR`; NVKMS HDR | nonunity refused | GPU multiply |
| TMO `SAT_MODE` | `TMO_LUT_SETTINGS_NO_CORRECTION` = 2 at 580/595.84 (`nvkms-evo3.c:540`), **3 at 610.43.02** (`:630`); both labelled "No color correction", same zones/weights | NVKMS TMO; DRM `NV_PLANE_TMO_LUT` | 3 refused | accepted, Ct/Cp untouched |
| TMO zones/weights, `SAT_MODE` 0/1 | field widths only (`clc57e.h`); no driver programs another value; no transfer equation in OGKM, nouveau or 610 | raw method writes only | refused | **still refused** |
| `*_MIRROR` (ILUT/OLUT/CSC LUTs) | every driver writes `_DISABLE` (`nvkms-evo3.c:611`, nouveau `headc57d.c`) | raw writes only | refused | still refused |
| `NVCA7E_SET_CSC0LUT_FP_NORM_SCALE` | GB20x field only; OGKM 580 never writes it | raw writes only | **not decoded** (gap) | gap recorded; GB20x scanout absent |
| capability `*_LOGNR`/`DIRECT` | NVKMS never reads them (Linux VSS caps are hard-coded, `:6052`) | — | DIRECT10, LOGNR 0 | unchanged |

### What is implemented

- `kf-disp/src/color.rs:199` decodes `MODE_SEGMENTED` per stage: entries = `SIZE − 4`
  in `[segments + 1, 1025]` (ILUT 64, OLUT 33), `Lut::segmented`; mirror/reserved bits
  and the undefined mode 3 refuse. `:501` passes the OLUT normalization through (any
  UNORM32, ≤ 1.0 by construction); a bypassed OLUT keeps no scale, as before (nouveau's
  clear path leaves the method unwritten). `:398` accepts SAT_MODE 2 or 3 with exactly
  the NO_CORRECTION zone/weight words.
- `cuda/display/kf_color.cu:102` `vss_lookup` reads the 3-bit interval counts from the
  **same immutable snapshot** it samples; every index is bounded by the authored extent
  even for a rejected header (`:117`–`:129`). `:199` `kf_vss_validate` rejects a header
  whose intervals + 1 exceed the extent (status 8) and FP16 input entries that are
  negative, nonfinite or > 128.0 (status 16); publication, notifiers and GET stay held
  behind the existing GPU verdict. `:286` applies the normalization to the OLUT input
  in every OLUT mode. No CPU transform, no CPU table read, no new allocation: tables
  use the existing fixed 1025-entry slots.
- `kf-cuda/src/display.rs:790` bounds segmented extents per slot and launches the
  validator; `kf-qemu/src/display.rs:925`/`:3486` carry mode and normalization; a mode
  change is a new token, never the old snapshot reinterpreted.

### Inferred domains, and their independent oracle

No source states VSS abscissae. The kernel uses ILUT [0,1] in 64 equal segments and
OLUT segment 0 = [0,2^-32], segment k = [2^(k−33), 2^(k−32)]. The GPU-free test
`crates/kf-cuda/tests/vss_domain_oracle.rs` checks this against analytic SMPTE ST 2084
using OGKM's own PQ tables: OETF within 6 of 65535 codes (the alternative [0,128]
domain in 125-units misses by >100), EOTF within 0.2 % in 125-units (128-units misses
by >2 %). This also shows that `/125` normalization plus a [0,1] log domain is the only
consistent reading of NVKMS's HDR path. Two inferences remain unverified on hardware:
segmented ILUT input uses DIRECT10's UNORM10 position `(c << 2) / 1024` (so a 16-per-
segment table equals DIRECT10 exactly), not `c / 255`; and FP16 ILUT values up to 128
are accepted for segmented tables (NVKMS's PQ EOTF reaches 125.06) while DIRECT tables
keep the [0,1] SDR bound. The existing CSC0 inline-table domain ([0,2^−25]…[64,128])
fits the same table only if 128 (not 125) is 10 000 nits — an open question, unchanged.

### Verification status

GPU-free: `cargo test -p kf-disp -p kf-cuda -p kf-qemu` pass, rustfmt clean, Clippy
existing-debt 415 / new 0, PTX regenerated by `make_color_ptx.sh` (clang 21.1.8) with
the ABI test updated. **Needs hardware, in order:** `color_vss <BDF>` (NVKMS and DRM PQ
round trips within one code of ST 2084, uniform-VSS ≡ DIRECT10, DIRECT-OLUT
normalization, hostile header/value refusals), the existing `color_sdr`/`color_tmo`
fixtures for regressions, 9/9 gates, then a strict Linux run with
`NV_PLANE_DEGAMMA_TF`/`NV_CRTC_REGAMMA_TF=PQ` and `NV_CRTC_REGAMMA_DIVISOR` (no
harness exists yet). **Needs an owner decision:** (1) whether NVIDIA's "No color
correction" label suffices to treat 610's SAT_MODE 3 like 2 — the change suggests one of
them may not be pure pass-through in hardware; (2) arbitrary chroma correction has no
source transfer function: keep the refusal, or fund a hardware-derived oracle (e.g.
display CRC fitting on an owned bare-metal head) before any implementation; (3)
whether to declare segmented capability bits (`LOGNR`) for Windows, whose meaning is
undefined in source.

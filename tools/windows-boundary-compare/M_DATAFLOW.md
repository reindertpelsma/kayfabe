# M capability publication and the second TMO constructor

**STATUS: RESEARCH, 2026-10-05.** Exact product source `d2c7ca1bd5c6f5be677ecd4a1c341ff4de8f6948`
and pinned Windows 580.88 static analysis. No hardware access, product edit or capability change
was performed for this audit. A constructor-only next experiment is proposed, not production support.

## The ILUT declaration reaches the authored page

The M8 command record names product/artifact revision `d2c7ca1b`, QEMU executable SHA256
`3088510b1fd8c5dd8985bbaf53ff01e05196b08256af618cbff80aae9ace670c`, and explicitly enables
`KF3_DISPLAY_ILUT_CONSTRUCTOR_PROBE=1`. Its log confirms ILUT surface loading `true`, the Ada
display row, and guest ABI `580.65.06`. Windows identifies itself as retail `580.88`; the explicit
guest ABI selection remains `580.65.06`. The source path is:

1. [`Device::bring_up`](https://github.com/reindertpelsma/kayfabe/blob/d2c7ca1b/crates/kf-qemu/src/device.rs#L706)
   selects the family row from the queried architecture/implementation. Ada uses display class
   C770, capability class C773, four heads and eight windows. The saved M8 RPC log contains
   successful C770 allocation; it does not prove a separate C773 RPC allocation occurred.
2. [`DisplayPlane::build`](https://github.com/reindertpelsma/kayfabe/blob/d2c7ca1b/crates/kf-qemu/src/display.rs#L1328)
   uses the selected **guest** table version, resolves the generated class/register tables, and
   chooses `ilut_constructor_probe_page` before the TMO-only or ordinary author. ILUT forces the
   same immutable all-display-method refusal engine even when the separate TMO flag is unset.
3. [`ilut_constructor_probe_page`](https://github.com/reindertpelsma/kayfabe/blob/d2c7ca1b/crates/kf-disp/src/caps.rs#L92)
   first authors the ordinary page plus TMO presence. It resolves C773
   `PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_SFCLOAD_TRUE` and writes that field for every advertised window.
   The compiler table defines CAPB at page offset `0x784+i*32`, field 14:14, TRUE=1. The resulting
   CAPB word is authored `0x4000`; LOGSZ, LOGNR and DIRECT remain zero. These are code/source facts,
   not values copied from a native GPU capture.
4. [`static_words`](https://github.com/reindertpelsma/kayfabe/blob/d2c7ca1b/crates/kf-qemu/src/display.rs#L1494)
   adds the generated `NV_PDISP_FE_SW` base. After all BAR pieces attach,
   [`seal_shadow`](https://github.com/reindertpelsma/kayfabe/blob/d2c7ca1b/crates/kf-qemu/src/device.rs#L1136)
   stores those words into the corresponding QEMU-backed shadow before the device runs.
   [`kf3_bar0_build`](https://github.com/reindertpelsma/kayfabe/blob/d2c7ca1b/qemu/hw/misc/kf3/kf3.c#L310)
   uses ROM-device RAM for this region: reads use its backing directly; writes trap.

No later CPU-side caps clearing was found along the inspected seal/publication paths. M8's
saved display-write counters remain zero. This is still not a captured Windows load or a live
capability-page snapshot: after Code43, BAR decoding is unavailable. The recovered journal's
advance past the first constructor provides stronger behavioral corroboration than its unchanged
365-RPC count. The separate dump analysis owns the raw-dump validation and exact assertion sequence.

## The second descriptor has a different constraint

The pinned driver SHA256 is
`31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c`.
Only reviewed RVAs are used. Extend/reproduce the fixed disassembly with
[`inspect-startdevice-j.py`](../windows-debug-capture/inspect-startdevice-j.py); every range is
at most 256 bytes and the binary hash is checked before disassembly.

M's recovered caller assertion is `16e97c6`, following the second constructor call `16e972c`;
L's was `16e97f4`, following the first call `16e9675`. The success-byte test at `16e9689` must
pass to reach the second call. The TMO predicate at `16e96a7` must also be true. The second
descriptor is the first window's TMO descriptor at frame-relative `rbp+0xd00` (stride `0xb0`).
This proves progress to a different constructor call, not the exact failing inner guard.

| TMO input | Static source/dataflow |
|---|---|
| +0 entry count | `1 << CAPD.TMO_LOGSZ`, from bits3:0 at `16e8f3f–16e8f57`. Zero exponent gives one. |
| +4 count | `1 << CAPD.TMO_LOGNR`, from bits6:4 at `16e8f51–16e8f74`. Zero exponent gives one. |
| +8 bit0 | CAPD.TMO_DIRECT bit9, extracted at `16e8f68`. This is not the pointer-shape discriminator. |
| +8 bit1 | CAPD.TMO_SFCLOAD bit8, extracted at `16e8f6d–16e8f97`. M leaves it zero. |
| +0x10 stride | The same `16f0690` alignment helper: `align_up((entries+5)*8,256)`. One entry gives256, above the minimum48. |
| +0x14 instances | Same owner getter at virtual slot+258 as ILUT; `1fbb0` reads owner+19d4. Both descriptors call it while being filled. |
| +0x18 buffers | **One**, initialized at `16e85ab`. Unlike ILUT, the TMO fill does not overwrite it with two. |
| +0x28 pointer array | Caller `16e96e0–16e96f7` writes owner backing+offset for each instance before calling the constructor. |

The public source field is
[`NVC773_PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/clc773.h):
CAPD has offset `0x78c+i*32`, the field is 8:8, TRUE=1. The existing compiler-generated table
already contains the field and TRUE value for C573/C673/C773/CA73 in both admitted display
driver tables. No observed native bit is needed to derive it.

With SFCLOAD=0 and buffer-count1, the count guard **passes**. At `16f04e0`, however, SFCLOAD=0
branches to `16f0536`; this requires each inspected pointer slot to be zero. A nonzero slot
branches at `16f0545` to the pointer-error path `16f05f2`. With SFCLOAD=1, slots inside the declared
buffer/instance range must instead be nonzero (`16f04ef–16f050f`), while unused slots still must
be zero. The caller's populated pointer array therefore presents a concrete shape mismatch if its
successful backing allocation supplied the expected nonzero pointer. No live pointer value was
recovered, so this remains a precisely located hypothesis rather than proof of the inner failure.

The initial guards also require nonzero +0/+4, sufficient stride, and 1..8 instances. Both ILUT
and TMO fill +0x14 through the same owner getter. A direct initialization path at `169cbcf–169cbd5`
copies parent+`0xefc` into owner+`0x19d4`; it is independent of CAPB/CAPD. Passing the first ILUT
constructor reduces this shared-count concern, but no live count or proof against every possible
mutation is claimed. CPU allocation after the pointer checks remains another possible failure.

## Minimal next discriminator

An isolated child of M can set **only** the generated CAPD `TMO_SFCLOAD_TRUE` for advertised
windows. Proposed flag: `KF3_DISPLAY_TMO_SURFACE_CONSTRUCTOR_PROBE=1`. It must force M's ILUT+TMO
declarations and the immutable all-display-method refusal engine even when used alone. DIRECT,
size/count fields, resource policies, addresses and control acceptance stay unchanged.

This deliberately advertises a construction-only property while refusing all operational display
methods. Public TMO methods bind a LUT and program real processing state; they are not implemented
by setting a caps bit. A positive constructor result would narrow the initialization issue, not
justify production tone-mapping support, copied native capability pages, or accepting GPU work.

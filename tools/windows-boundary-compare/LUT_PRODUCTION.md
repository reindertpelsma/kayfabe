# After the LUT constructor diagnostics

**STATUS: RESEARCH, 2026-10-05 — DRAFT, independent review pending at owner-requested stop.** Read-only source audit of product
`9312854d0f67e2db9c6c5dffc7c467955970b816` (N, default-off, all display methods
refused). This proposes work; it implements nothing and does not qualify N for
production. Windows constructor progress establishes neither its eventual LUT
programming nor correct rendered pixels. See [the M dataflow audit](M_DATAFLOW.md)
for the earlier constructor evidence.

## Conclusion

A source-defined disabled TMO binding can be implemented honestly. It does not
justify advertising active TMO/SFCLOAD support. An unconditional ILUT bypass is
also insufficient: public NVKMS programs a real identity FP16 ILUT for ordinary
non-FP16 composition, even without a client-provided LUT. The useful next
production increment is explicit color-state validation, followed by real bounded
LUT processing on Kayfabe's GPU composition path. Keep the constructor probes
separate until their advertised operations exist.

The presently advertised CSC/LUT blocks deserve the same audit. The current
engine stores generic aligned, in-range method words; the composition plan and
kernel have no ILUT/TMO/CSC state. Thus a successful ordinary display test does not
prove arbitrary color transformations work. This is an existing semantic gap,
not permission to extend silent acceptance.

## Source anchors and compatibility scope

Read locally, without executing guest drivers:

- OGKM 580.65.06 source commit
  [`307159f2623d3bf45feb9177bd2da52ffbc5ddf9`](https://github.com/NVIDIA/open-gpu-kernel-modules/tree/307159f2623d3bf45feb9177bd2da52ffbc5ddf9).
- OGKM 580.159.04 source commit
  [`b81d58ee0224d1d290bef1c080592b619e184042`](https://github.com/NVIDIA/open-gpu-kernel-modules/tree/b81d58ee0224d1d290bef1c080592b619e184042).
- The complete `nvkms-evo3.c` and `nvkms-evo4.c` diffs between these commits only
  add the unrelated `supportsYCbCr422OverHDMIFRL` HAL initializer field. The LUT
  functions discussed here are unchanged. This is source equivalence for these
  two rows, not a claim across all NVIDIA releases or Windows implementations.

The product's `kf-disp::class::for_version` admits those two display tables.
Its family rows select Turing C573/C57E, Ampere C673/C67E, Ada C773/C67E, and
GB20x CA73/CA7E (caps/window). Ada does **not** use an invented C77E window.
Displayless datacenter families remain displayless. Host-driver selection must
remain independent of guest-class vocabulary; no RTX 4070 fact belongs in a
virtual capability author.

| Obligation | Public source at 580.65.06 | Consequence |
|---|---|---|
| Disabled TMO | `nvkms-evo3.c:917–922`, `EvoSetTmoLutSurfaceAddressC5:852–866` | Null surface emits handle 0 and offset 0; do not fetch LUT data in that state. |
| Active TMO | `nvkms-evo3.c:925–978`, `InitializeTmoLut:762–817` | Control, LUT binding, and intensity-zone/weight state affect output; not mere buffer registration. |
| Default input LUT | `nvkms-evo3.c:4539–4565,4672–4723` | Non-FP16, non-bypass composition gets 1024 FP16 identity entries plus a repeated interpolation endpoint and the source-defined header; controls select DIRECT10, interpolation off, mirror off. |
| Actual ILUT bypass | `nvkms-evo3.c:4672–4677,4718–4721` | FP16 input or `bypassComposition` selects a null binding. These guards cannot be assumed for Windows. |
| Client input LUT | `nvkms-evo3.c:4348–4371` | A client-specified surface/offset/size is used; it need not be the default identity table. |
| Capability effects | `nvkms-evo3.c:6047–6072` | TMO presence exposes ICtCp and a 64-segment, 1025-entry UNORM16 LUT; the Turing+ HAL exposes FP16 ILUT support. Presence is not a bypass-only promise. |
| Other color state | `nvkms-evo3.c:4630–4639,4655–4669,1045–1069,5352–5407` | FMT/CSC and output LUT state also matter. A default identity ILUT alone is not proof the entire composed pipeline is identity. |

These names describe Linux source paths, not recovered Windows method values.
The published headers name fields and methods but do not by themselves specify
all fixed-function numerical rounding, saturation and interpolation behavior.
Missing arithmetic semantics require further source/reverse-engineering evidence
before full TMO emulation can be claimed.

## Method and address vocabulary that must be generated

| Family window class | ILUT binding | TMO binding |
|---|---|---|
| C57E / C67E | `SET_CONTEXT_DMA_ILUT` + `SET_OFFSET_ILUT` | `SET_CONTEXT_DMA_TMO_LUT` + `SET_OFFSET_TMO_LUT` |
| CA7E | `SET_SURFACE_ADDRESS_HI_ILUT` + `SET_SURFACE_ADDRESS_LO_ILUT` | `SET_SURFACE_ADDRESS_HI_TMO_LUT` + `SET_SURFACE_ADDRESS_LO_TMO_LUT` |

`nvkms-evo3.c:4568–4581` emits the ILUT offset directly;
`:852–866` emits TMO offset shifted right eight. Do not share an assumed offset
unit between the methods. Blackwell's HAL uses the C9-style address writers
(`nvkms-evo4.c:1593–1648`, reused by `nvEvoCA`): the common helper at `:87–106`
splits the byte address into high32 and low32 shifted right four, with target and
enable fields encoded separately. CA7E's own public header defines those fields.
It supports several address targets; none is a host pointer or host physical PFN.

Extend `tools/derive_display_classes.sh` through its existing compiler evaluation
path for `SET_ILUT_CONTROL`, both old context-DMA bindings, `SET_TMO_*`, and the
relevant FMT/CSC/OLUT controls. Existing `SET_OFFSET`/`SET_SURFACE_ADDRESS_`
prefixes already derive some addresses, but not the full operational vocabulary.
Validate enum values, masks, sizes and disabled encodings per admitted table;
missing vocabulary refuses that feature. Do not implement by hand-copying the
numeric offsets in a research note.

## Bounded implementation proposal

1. **Decode state before accepting an UPDATE.** Add typed assembly and armed
   ILUT/TMO/color state. Class-derived mode, interpolate, mirror, sizes, target,
   enable, and TMO zone/control fields need explicit meanings and reserved-field
   checks. Inactive storage may be staged across methods, but the complete active
   state must validate before latching, interlocked completion, or notifier
   success. An unsupported active transform halts/refuses the relevant channel;
   it must not fall through to generic bank storage and successful UPDATE.
2. **Resolve only owned, bounded guest storage.** Older classes use the existing
   instance-memory `(client, handle, channel)` lookup and the context DMA's limit.
   Check linear layout, the method-specific offset units, checked byte extent
   (including required header/endpoint), and the entire span against the guest
   store. CA7E needs its own typed address-target decode. A minimal first active
   implementation may explicitly support vidmem only; guest system memory needs
   live, revocable RAM authority and an audited upload path. IOVA needs the actual
   supported guest-IOMMU translation or refusal, never treating it as a physical
   address. No new privileged host RM mapping or raw guest-to-host forwarding.
3. **Make LUT lifetime real.** A per-window resolution is tied to the channel
   incarnation, accepted binding, and armed generation. Free/reuse, reset, failed
   update and family changes cannot resurrect an old binding. Resolve at latch;
   preserve an owned bounded GPU snapshot or a justified retained store view until
   the last composition using it completes. If validation relies on table
   contents, processing must use that same snapshot: validating identity and then
   reading mutable guest bytes is a TOCTOU bug. Existing `LatchedDmas` behavior
   preserves framebuffer scanout across unbind; do not automatically assume its
   lifetime rule also specifies LUT load semantics.
4. **Execute the promised transformation.** Begin with a source-defined direct
   FP16 ILUT subset and documented disabled TMO, adding accurate FMT/CSC/OLUT
   behavior as required by the complete accepted pipeline. Active segmented TMO
   additionally requires its segment/header arithmetic, intensity weighting,
   saturation, interpolation and ordering. An identity optimization is legitimate
   only if the entire accepted pipeline is proven equivalent, including
   quantization/rounding; a familiar size or default-looking control word is not
   sufficient. Caps must state the implemented limits, not native captured values.
5. **Complete what was actually processed.** GPU pixel/LUT work belongs in the
   authored display CUDA path, using bounded fixed-budget storage. Pure disabled
   state updates may complete after emulation. Any composition/load queued to the
   host GPU requires its actual completion before dependent success/release.
   Do not reuse the current console-copy failure fallback as proof an unsupported
   transform succeeded: `ScanState::finish` also releases barriers after refusal
   or abandonment, and `scanout.rs` explicitly preserves flip completion when
   console rendering refuses. Such output fallback is different from implementing
   a newly promised color operation.

The v3 distinction is explicit: [V3_DISPLAY §4.8](../../docs/design/V3_DISPLAY.md#48-constraint-audit)
allows CPU parsing and control-state emulation, while scanout work runs on the GPU.
[OWNER_RULINGS §A](../../docs/OWNER_RULINGS.md#a-standing-principles) forbids a CPU
data plane. A CPU pixel-transform fallback would be a deviation, even though the
guest display channel is emulated. The existing `compose_reference` CPU routine
is a test oracle for the GPU kernel, not a product execution route.

## What to validate before promotion

GPU-free checks should cover all eight guest-table/family cells, missing generated
definitions, disabled bindings, interlocked atomic failure, malformed sizes and
address overflow, wrong-client handles, free/reuse/reset, and mutable-LUT snapshot
behavior. GPU tests should compare the implemented numerical subset against a
source-derived oracle using adversarial tables, endpoint values and rounding
cases, then confirm load/composition failure never produces success for skipped
work. Repeated matched Windows/Linux runs must show the actual methods and
states used; constructors and native capability pages cannot provide them.

The minimal no-transform policy is honest but can refuse otherwise valid Linux
composition and cannot yet be said to satisfy Windows. Real direct-ILUT support
is a useful bounded first implementation, but TMO presence must remain a declared
experimental mismatch until its exposed transformations exist or a source-proven
driver path operates without advertising them. No production permission, feature
enablement, or hardware qualification is implied by this note.

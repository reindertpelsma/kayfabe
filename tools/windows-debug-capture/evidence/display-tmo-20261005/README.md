# Windows display-construction failure: public TMO capability mapping

**STATUS: RESEARCH, 2026-10-05.** Static source audit for the Windows J investigation;
no product change or hardware result is claimed here.

## Finding and provenance

The Windows 580.88 helper under investigation reads capability byte offset
`0x780 + window * 0x20`, then tests bit 20. This exactly matches the public
`NVC773_PRECOMP_WIN_PIPE_HDR_CAPA(window).TMO_PRESENT`. The earlier description
as a per-head capability was incorrect: this is a **per-window precomposition**
capability. The Windows stack/call-site causality belongs to the separate NVCD
analysis; this note proves the public field correspondence and its obligations.

Public source is NVIDIA OGKM **580.65.06**, commit
`307159f2623d3bf45feb9177bd2da52ffbc5ddf9`, from
<https://github.com/NVIDIA/open-gpu-kernel-modules>. Product observations below
are pinned to Kayfabe **`fdc991c9d3fd51dab005beda2487365661a3db25`** (probe J).

| Public source | Meaning |
| --- | --- |
| `src/common/sdk/nvidia/inc/class/clc773.h:503–530` | Array byte offset `0x780 + i*32`; TMO-present field bit 20, TRUE=1, FALSE=0. |
| Same header, lines 589–602 | Companion CAPD at `0x78c + i*32`: TMO_LOGSZ, TMO_LOGNR, TMO_SFCLOAD and TMO_DIRECT. Their reset values do not prove a usable TMO configuration. |
| `src/nvidia-modeset/src/nvkms-evo3.c:5868–5900` | Linux reads the same per-window field into `tmoPresent`; asserts that TMO requires the CSC0 matrices, CSC10/11 and CSC LUT capabilities. |
| Same file, lines 6049–6072 | TMO presence also exposes ICtCp and a 64-segment, 1025-entry, VSS-required UNORM16 tone-mapping LUT to callers. It is a functional promise, not an allocation hint. |
| `src/nvidia-modeset/src/nvkms-evo.c:9670–9722` | Automatic tone mapping depends on a non-NULL surface, HDR metadata/output state and source/target luminance. A client override can directly request its LUT. |

The method is not Windows-only: public Linux NVKMS consumes the same capability
and programs TMO. Windows's unconditional constructor allocation when the
capability is absent is a separate observation, not a Linux behavioral claim.

## Disabled and enabled contracts

`ConfigureTmoLut` in `nvkms-evo3.c:869–983` distinguishes the states:

* **Disabled:** lines 917–921 call `SetTmoLutSurfaceAddress(NULL, 0)` and return.
  For the C5 path (`EvoSetTmoLutSurfaceAddressC5`, lines 852–866), this writes
  `SET_CONTEXT_DMA_TMO_LUT` handle zero and `SET_OFFSET_TMO_LUT` origin zero.
  This is a source-backed bypass; it does not read a LUT or apply a transform.
* **Enabled:** lines 924–982 set `SET_TMO_CONTROL` with size/interpolation/saturation,
  bind the LUT surface and offset, and program the low/medium/high intensity
  zone and value methods. Earlier code initializes LUT content when required.
  Accepting an active binding without applying its requested transform would
  silently produce incorrect display output.
* **Later address ABI:** `nvkms-evo4.c:1593–1618` uses
  `SET_SURFACE_ADDRESS_HI_TMO_LUT` and `SET_SURFACE_ADDRESS_LO_TMO_LUT` with an
  explicit ENABLE field. C97E/CA7E therefore need their own generated vocabulary;
  a C57E context-DMA guard cannot be assumed to cover them.

The public C57E/C67E headers place TMO control/intensity methods at
`0x500`, `0x508` through `0x51c`, context DMA at `0x528`, and offset at `0x52c`.
C97E/CA7E use address HI/LO at `0x690`/`0x694`, with ENABLE bit 0.
These are manually reviewed source references, **not a hand-maintained product
register table**. Product values must continue to come from the compiler.

A future bypass implementation must distinguish disabled state from an active
identity-looking LUT: the latter still names guest memory and needs its real
validation/lifetime/interpretation. This audit does not prove arbitrary LUTs
are identity or authorize discarding their contents.

## What J implemented and why a cap-only patch is insufficient

At J, `crates/kf-disp/src/caps.rs:21–23,148–162` deliberately advertises no TMO;
it sets CSC/LUT presence but leaves TMO_PRESENT clear. The cap word therefore
explains why the inspected Windows predicate returns false.

`crates/kf-disp/src/engine.rs:745–790` checks method alignment and bounds, then
stores every in-range non-UPDATE method in assembly. `complete` at lines
1082 onward arms the words and issues requested completions. There is no TMO
semantic rejection in that revision. Merely setting TMO_PRESENT could therefore
silently complete work the display has not implemented.

The next **proposed diagnostic**, not a production compatibility fix, is to
advertise the compiler-derived TMO presence bit only under a default-off
construction experiment while refusing **all display methods** before assembly
mutation or UPDATE/completion. This can test the constructor hypothesis without
claiming tone mapping, HDR, active scanout, or Windows readiness. It is an
intentional capability over-advertisement for that bounded experiment and must
remain labeled as such. There are no host calls or LUT dereferences in this
proposal. Production behavior must either implement the advertised contract or
find a source-supported way to avoid advertising it.

## Generated availability and compatibility limits

`generated-availability.json` is a reproducible census of the existing
**compiler-derived** display TSVs, not a parser of raw C. Reproduce from a local
Kayfabe clone containing the pinned product revision:

```sh
python3 tools/windows-debug-capture/inspect-display-tmo.py --repo /path/to/kayfabe \
  > tools/windows-debug-capture/evidence/display-tmo-20261005/generated-availability.json
```

Both committed source-version tables, **580.65.06 and 580.159.04**, contain the
array offset, TMO_PRESENT field and TRUE=1 for C573/C673/C773/CA73. C373 has no
such generated field. Missing entries mean unavailable to the current generated
vocabulary, not evidence that hardware or a public header lacks the feature.

The C57E/C67E tables currently include `SET_OFFSET_TMO_LUT` but omit
`SET_CONTEXT_DMA_TMO_LUT` and `SET_TMO_CONTROL`. CA7E address methods and ENABLE
fields are present. C97E is not a currently generated method class in these
tables. A later selective semantic guard would need the generator expanded and
the relevant driver/class cells checked; a construction-only blanket refusal
does not need to infer any of those missing method values.

This establishes a source mapping across the listed class definitions, not
Windows runtime compatibility across drivers or dies. It supplies no captured
per-die capability table. The diagnostic must fail closed when its field is
unavailable; headless families cannot acquire display support from this note.

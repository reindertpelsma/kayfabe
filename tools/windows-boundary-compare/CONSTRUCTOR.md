# Source-defined capability constraints in the Windows display constructor

**STATUS: RESEARCH, 2026-10-05.** Offline public-source and pinned retail-code analysis.
No product changes, capability enablement, GPU jobs or captured per-die tables. This narrows
experiment L's failure; it does not implement input-LUT or tone-mapping behavior.

## Concrete discriminator: ILUT surface loading and buffer count

Windows 580.88 constructs the first per-window descriptor from
`NVC773_PRECOMP_WIN_PIPE_HDR_CAPB(i)`, byte offset `0x784 + i*32`. Public OGKM 580.65.06
commit `307159f2623d3bf45feb9177bd2da52ffbc5ddf9`,
[`class/clc773.h:547–562`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/clc773.h#L547),
defines the following fields. They already exist in the compiler-generated
[class table](../../crates/kf-disp/data/classes-580.65.06.tsv).

| Source field | Bits | Pinned Windows constructor input |
|---|---|---|
| `PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_LOGSZ` | 9:6 | input+0 = `1 << field`; required nonzero |
| `PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_LOGNR` | 12:10 | input+4 = `1 << field`; required nonzero |
| `PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_SFCLOAD` | 14 | input+8 bit1; selects permitted buffer-count shape |
| `PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_DIRECT` | 15 | input+8 bit0; **not** tested by the failing buffer-count guard |

The caller unconditionally writes **two** to input+`0x18`. Constructor `0x16f03c0`
requires input+`0x18 == 1` when input+8 bit1 is clear; with bit1 set it accepts counts up to
two at this guard. Therefore Kayfabe's zero CAPB guarantees failure at that guard if execution
reaches it: surface loading is absent, yet the caller requests a two-buffer descriptor.
This statement follows from the pinned caller and constructor, not an assumed native value.
The assertion in L still does not expose live locals or exclude an earlier guard failing first.

The initial size checks do **not** require changing LOGSZ/LOGNR just to discriminate this guard:
zero exponents give counts one, both nonzero. The caller's stride helper computes
`align_up((entry_count+5)*8,256)`, so entry_count one yields 256, exceeding the constructor's
minimum `entry_count*8+0x28` (48). Input+`0x14` must separately be 1..8; it comes from
owner+`0x19d4`, not these capability bits, and no live value is claimed here. Later buffer-pointer
checks and CPU allocation can still fail. Passing a particular guard is not full initialization.

A minimal isolated discriminator is **only** the compiler-derived `ILUT_SFCLOAD_TRUE` field for
advertised windows, retaining L's all-display-method refusal and all other caps unchanged. This is
an intentional construction-only declaration, not a production promise of surface loading. It
must not be used without the refusal guard, and cannot justify default capability changes.
DIRECT and LUT sizes should remain unchanged in this one-variable experiment. This note does
not request or perform the experiment.

## Reproducible pinned-code evidence

Trusted `nvlddmkm.sys` SHA256:
`31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c`.
Only RVAs appear here. The extended [bounded inspection script](../windows-debug-capture/inspect-startdevice-j.py)
checks this hash and disassembles fixed ranges, each at most 256 bytes, without executing the driver.

```sh
python3 tools/windows-debug-capture/inspect-startdevice-j.py \
  --driver /trusted/580.88/Display.Driver/nvlddmkm.sys --disassemble
```

| RVA | Observed static behavior |
|---|---|
| `16e8ddb–16e8e11` | Starts offset cursor at `0x78c`, tests source window-exists bit at page+4. |
| `16e8e11–16e8e68` | Reads cursor-8 (`0x784+i*32`), expands LOGSZ and LOGNR into input+0/+4. |
| `16e8e72–16e8eaa` | Maps DIRECT and SFCLOAD into input+8 bits0 and1. |
| `16e8eab–16e8edc` | Gets input+14, computes stride, and unconditionally writes buffer-count2. |
| `1fbb0–1fbb7` | Getter reads owner+19d4. All five static vtables containing initializer16e7d90 at+a80 use this getter at+258; no live count inferred. |
| `16e9675–16e9692` | Calls constructor16f03c0, tests success byte and branches to L's assertion. |
| `16f0481–16f04bd` | Checks nonzero counts, minimum stride and 1..8 instance count. |
| `16f04bd–16f0536` | With SFCLOAD clear, requires buffer-count1; count2 reaches failure16f051f. With SFCLOAD set, count<=2 proceeds to pointer checks. |
| `16e97ef–16e981d` | L's journal return address16e97f4 is the caller's generic constructor-failed assertion. |

The private journal/caller association and preceding TMO allocation progress remain documented in
[the J/L causal evidence](../windows-debug-capture/evidence/2026-10-05-startdevice-j.md).

## Other source-named fields worth comparing

These are candidate constraints, **not inferred required native values**:

- `PRECOMP_WIN_PIPE_HDR_CAPD_TMO_LOGSZ` (3:0), `TMO_LOGNR` (6:4), `TMO_SFCLOAD` (8),
  and `TMO_DIRECT` (9) feed the analogous TMO descriptor at caller16e8f12–16e8fd2.
  This later descriptor has separate handling; the ILUT result alone does not diagnose it.
- `PRECOMP_WIN_PIPE_HDR_CAPC_CSC0LUT_*` and `CAPE_CSC1LUT_*` name the two color-transform
  LUT stages' size/count/load capabilities. Their meanings are relevant to feature promises;
  this audit has not tied them to the current first constructor failure.
- CAPA `FULL_WIDTH`, `UNIT_WIDTH`, `ALPHA_WIDTH`; CAPB `FMT_PRECISION`; and CAPC/CAPE
  `CSC*_PRECISION` name format/processing widths. They should be retained in annotated
  comparisons, but no direct dataflow into this constructor's initial guards was established.
- `SYS_CAPB_WINDOW*_EXISTS` determines which window descriptors the caller builds. Do not
  confuse these per-window fields with head counts. Separately, public NVKMS debug code
  asserts postcomposition `UNIT_WIDTH == 16` (`nvkms-evo3.c:5858–5863`); that is not proof
  of the Windows precomposition constructor's width requirement.

## Surface loading is an operational promise

Public `clc57e.h:461–476` defines `SET_ILUT_CONTROL` (interpolation, mirror, segmented/direct
mode and size), `SET_CONTEXT_DMA_ILUT`, and `SET_OFFSET_ILUT`. NVKMS
`nvkms-evo3.c:4565–4581` binds a LUT surface descriptor or null bypass; at4707–4714 it programs
LUT mode/size, and prepares real LUT data beforehand. `DIRECT` is a capability name: do not
assume it is identical to the independently named `SET_ILUT_CONTROL_MODE_DIRECT10` enum.
The capability/operational relation needs its own full source audit before production admission.

The current Kayfabe display engine's ordinary path stores aligned method words in its assembly
bank (`engine.rs` around exec), then decodes a selected scanout vocabulary. There is no dedicated
ILUT/TMO decoding or LUT data processing in `kf-disp/src` at this revision. Generic method storage
is not implementation of these features. An authored virtual ILUT capability would require a
bounded guest-owned LUT binding, validated source/layout/size, actual advertised color-transform
behavior, and correct update/lifetime semantics. It cannot simply be copied from native hardware
or declared to make a constructor pass. L's blanket method refusal keeps the present investigation
from claiming those operations work; the proposed one-bit discriminator must preserve that limit.

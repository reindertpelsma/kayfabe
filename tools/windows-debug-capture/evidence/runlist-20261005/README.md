# Class 0xb297: software-runlist evidence

**STATUS: RESEARCH, 2026-10-05.** Static metadata and public-source comparison;
no product behavior or hardware compatibility is certified by this directory.

`0xb297` is strongly identified as the RM **RunlistApi** resource. Its public
allocation-parameter counterpart is `NV_SWRUNLIST_ALLOCATION_PARAMS`. This is
not an ordinary 3D object inferred from the `97` suffix. The external class
number-to-name link is an inference from independent metadata and source
evidence, **not a published OGKM `#define`**.

An important limitation for the allocation-only diagnostic: **`maxTSGs=0`
selects native default capacity; it does not promise an empty or inert resource.**
Native successful allocation can create two hardware runlist buffers and process
the requested QoS mask. Returning success while retaining only guest metadata
is an intentionally incomplete diagnostic, not implementation of that contract.
It must not imply supported submission, preemption, completion or notification.

## What establishes the identity

1. Authenticated proprietary Linux **535.309.01 and 610.43.02** both contain an
   `RS_RESOURCE_DESC` row with external class `0xb297`, internal class
   `0xf4b771`, required 12-byte parameters, multiple instances permitted, and
   parent class `0x4b01b3` (`Subdevice`). ELF relocations connect the row to a
   matching NVOC class-info descriptor and its parent list. This is structured
   metadata, not a free-floating occurrence in firmware bytes.
2. Every one of the **299 distinct NVOC class names** in the public OGKM
   580.65.06 generated headers matches the first 24 bits of MD5(name). Names are
   discovered from compiler-preprocessor output and numeric values are compiled;
   raw C definitions are not parsed to derive this census.
   `MD5("RunlistApi") = f4b771218c13a4ed2ce4b8106ff25038`. A 24-bit hash is not
   intrinsically unique; the allocation layout, parent and later behavior are
   independent corroboration. The candidate name is explicitly an inference.
3. Public OGKM 580.65.06 `nvos.h` defines
   `NV_SWRUNLIST_ALLOCATION_PARAMS`: `engineId`, `maxTSGs`,
   `qosIntrEnableMask`, all `NvU32`. The actual header was compiled by
   [runlist-layout.c](../../runlist-layout.c): size 12, offsets 0/4/8.
   Its comments describe software runlists, double buffering, TSG capacity and
   the hardware-dependent entry format. [Public declaration](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/nvos.h#L2914)
4. Windows 580.88 analysis independently reports the same metadata at
   `nvlddmkm.sys` RVA `0x1135a40`, with flags `0xa153`, and allocates two handles
   subsequently used by control `0x20801111`. Windows findings belong to the
   separate pinned-driver/NVCD investigation; this tool independently parses
   the Linux installers, not Windows private dump contents.

## Compatibility evidence and its limits

| Artifact | 0xb297 descriptor | Allocation layout | 0x20801111 | Evidence scope |
|---|---|---|---|---|
| Linux proprietary 535.309.01 | Present, internal `0xf4b771`, Subdevice parent | Required 12 bytes; flags `0x103` | Present, 40 bytes; flags `0x2200` | Static registration and handler inspection |
| Linux open 535.309.01 object in same installer | Not found by structured scan | No matching descriptor | Not found | Bounded negative evidence |
| Windows proprietary 580.88 | Same internal class and parent, flags `0xa153` | Required 12 bytes | Caller uses 40 bytes | Separate retail-driver investigation, RTX 4070 capture |
| Linux proprietary 610.43.02 | Present, same internal class and parent | Required 12 bytes; flags `0xa153` | Present, 40 bytes; flags `0x40` | Static registration and handler inspection |
| Linux open 610.43.02 object in same installer | Not found by structured scan | No matching descriptor | Not found | `0x20801110` is present separately |
| Public OGKM 580.65.06 | No published mapping found | Compiled SW-runlist type is 12 bytes | No public definition found | Source layout; not external-class registration |

These Linux versions are **installer/driver-file versions**, not an asserted
GSP protocol version. Windows 580.88's association with GSP 580.65.06 comes from
the separate trace evidence. Nothing here proves the class is supported on
every die, callable by every Linux configuration, or used during Linux boot.
It does establish that the registration and implementation are **not confined
to the inspected Windows binary**. No GPU was used in this investigation.

The name/layout evidence does not justify adding a captured per-die table or
bypassing the v3 family/host-driver/guest-driver axes. Deriving a production
class entry from retail metadata rather than OGKM is a departure from the
normal class-generation source and must remain explicit.

## Engine namespace and constructor

In proprietary Linux 610.43.02, ELF relocations resolve:

| Link | Symbol / section-relative offset |
|---|---|
| Resource row | `.data+0xc030` |
| NVOC class info | `_nv003112rm`, `.rodata+0x56e7520` |
| Dynamic creation wrapper | `_nv006236rm`, `.text+0xbe4e40` |
| Generated object creation | `_nv006614rm`, `.text+0xbe4bf0` |
| Resource constructor | `_nv053753rm`, `.text+0x4be3a0` |
| Engine conversion called on parameter offset 0 | `_nv029646rm`, `.text+0x556f80` |

The engine conversion's 83 table entries and its zero/out-of-range branches
match the compiled public `gpuGetRmEngineType_IMPL` for inputs **0..84**, all
85 equal. Therefore this constructor takes **NV2080_ENGINE_TYPE**, then converts
to the internal RM_ENGINE_TYPE domain. It is not a raw runlist index.
[engine-domain-610.43.02.json](engine-domain-610.43.02.json) records the comparison.
The public function explains why these domains differ for compatibility.
[Public conversion](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/86856f779971146f807a7ae2a864b4aee147b553/src/nvidia/src/kernel/gpu/gpu_engine_type.c#L34)

The native allocation path forwards `maxTSGs` through
`_nv053958rm` → `_nv015950rm` → `_nv038917rm` → `_nv038838rm`.
The last function calculates buffer size:

- `.text+0x48e91a` checks requested capacity against a hardware-derived limit.
- `.text+0x48e923` tests the requested value for zero.
- `.text+0x48e932` substitutes the derived limit when the request is zero.
- The result participates in the size calculation at `0x48e9ae`/`0x48e9b4`.
- `_nv038917rm` then loops across two descriptors and allocates their storage.

This is static control-flow evidence for 610.43.02, not a measured capacity or
a portable sizing formula. No size from this binary should become a per-die
constant. The constructor also has GPU-property-dependent early return paths;
those properties were not fully mapped here.

The public `NV_SWRUNLIST_QOS_INTR_NONE=0` and named QoS bits establish the mask's
documented domain. The retail constructor reads offset 8 and passes it to a
FIFO HAL callback after buffer allocation. The callback's complete effect is
not established here; no success/completion may be inferred for unsupported
nonzero masks.

## Scheduling control and privilege boundary

The [bounded scheduling follow-up](scheduling-control.md) resolves additional
535 behavior: +4 selects a group ordinal during runlist construction, +24/25/26
affect rebuild/staging/submission paths, and +32 enters a potentially event-bearing
asynchronous path. **Zero count does not bypass scheduling or prove a no-op.**
An earlier memory-allocation refusal also means the subsequent Windows control
failure is not yet established as an independent initialization root cause.

Linux 535 and 610 `0x20801111` handlers have matching significant accesses:

| Parameter byte offset | Observed use; names are descriptive unless stated |
|---|---|
| 0, u32 | Resource handle, looked up in the caller's RM client, cast to internal `0xf4b771`; its parent must match the calling Subdevice |
| 4, u32 | In 535, compared to the group ordinal while building the hardware runlist; the current entry position is saved on a match. Exact public name unknown |
| 8, u32 | Looked up as public NVOC `Memory` class `0x4789f2` in the caller's resource hierarchy |
| 12, u32 | Used downstream as a bound on 12-byte records in the owned memory |
| 16, u16 | Offset added to the owned Memory object's CPU mapping to locate an index sequence |
| 20, u32 | Downstream index count; the sequence contains 16-bit indices |
| 24/25/26, bytes | Scheduling flags passed to scheduler operations; complete meanings not established |
| 32, u64 | Conditionally stored in a pending per-engine record by an asynchronous helper; potentially event/pointer-bearing, complete type not established |

The 610 handler is `_nv055811rm` at `.text+0x4b1cf0`; the 535 counterpart is
`_nv046624rm` at `.text+0x43ce10`. In 610, resource lookup/type/parent validation
occurs around `0x4b1dd9..0x4b1e46`; Memory lookup/type checking around
`0x4b1e70..0x4b1ea6`; the scheduler call is at `0x4b1ee2`.
Downstream code examines selected 12-byte records, resolves referenced channel
resources and invokes scheduler virtual methods. This is a real scheduling
operation, not merely handle bookkeeping. The read pointer is obtained from an
owned Memory object here; this does **not** establish that the complete control
has no pointer-bearing or other unsafe fields.

Allocation and scheduling have different privilege classifications. The
allocation descriptor sets `RS_FLAGS_ALLOC_NON_PRIVILEGED` in both inspected
Linux versions. That proves **registration policy**, not successful runtime
allocation from an unprivileged process. The `0x20801111` method flags do not set
`RMCTRL_FLAGS_NON_PRIVILEGED` or `RMCTRL_FLAGS_PRIVILEGED`; public `control.h`
defines the default as **kernel-client-only**. No forwarding allowlist addition
is justified by the allocation flag. [Public control policy](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/inc/kernel/rmapi/control.h#L172)

The neighboring **`0x20801110` is publicly named** in OGKM 610.43.02:
`NV2080_CTRL_CMD_FIFO_CONFIG_CTXSW_TIMEOUT`, taking `NvU32 timeout` and
`NvBool bEnable` (8-byte metadata size). Its timeout unit is a PTIMER
microsecond tick of 1024 nanoseconds. This source fact does not identify
`0x20801111`. [Public timeout control](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/86856f779971146f807a7ae2a864b4aee147b553/src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080fifo.h#L484)

## Reproduction and provenance

[inspect-linux-runlist.py](../../inspect-linux-runlist.py) first verifies the
entire installer SHA256 against the reviewed pins, then uses system `zstd` and
Python's streaming tar reader. It reads only the two kernel object members and
parses them as ELF data. **It never executes an installer or extracted object.**
The saved JSON contains hashes and derived metadata, not binary sections.
[runlist-metadata-layout.c](../../runlist-metadata-layout.c) compiles actual
`sizeof`/`offsetof` from the corresponding pinned public 535/610 headers before
parsing each version's resource and method records. These are x86-64 layouts
with `NV_PRINTF_STRINGS_ALLOWED=0`, matching the inspected retail records.
The class-ID storage is measured using compiler-initialized bytes because
610's public `NVOC_CLASS_INFO.classId` is a bit-field.
The parser uses those compiled offsets, then cross-checks class-info and parent
relocations. Only the public print probes are compiled and executed.

```bash
python3 tools/windows-debug-capture/inspect-linux-runlist.py \
  --installer /path/NVIDIA-Linux-x86_64-535.309.01.run \
  --installer /path/NVIDIA-Linux-x86_64-610.43.02.run \
  --ogkm-root /path/ogkm-580.65.06 \
  --ogkm-git /path/open-gpu-kernel-modules > linux-metadata.json

cc -I/path/ogkm-580.65.06/src/common/sdk/nvidia/inc \
  tools/windows-debug-capture/runlist-layout.c -o /tmp/kf-runlist-layout
/tmp/kf-runlist-layout
```

Official checksum files, fetched over HTTPS, exactly matched the existing
local installer files:

| Driver | Installer SHA256 | Official checksum |
|---|---|---|
| 535.309.01 | `288b4902ea79b017b49a9226a84f7eaa3e6b49ca0146ae50277c2fe19dd39803` | [NVIDIA](https://download.nvidia.com/XFree86/Linux-x86_64/535.309.01/NVIDIA-Linux-x86_64-535.309.01.run.sha256sum) |
| 610.43.02 | `3034a054bb4cdf7752ff8dc272564cb105513804bff53538945901b16ca77463` | [NVIDIA](https://download.nvidia.com/XFree86/Linux-x86_64/610.43.02/NVIDIA-Linux-x86_64-610.43.02.run.sha256sum) |

The engine comparison additionally uses a data-only extraction of
`kernel/nvidia/nv-kernel.o_binary` from the 610 installer (SHA256
`af6dbeebe6b7d4d5ea63fa954796dc78de69cfaa754922a3425f62dab108b859`).
[compare-runlist-engine.py](../../compare-runlist-engine.py) verifies that hash,
compiles the function and enum from public OGKM tag 610.43.02, and compares its
output with the reviewed retail lookup table. Its `nvtypes.h` support include
was the public 580.65.06 SDK; the enum, mapping function and NV2080 engine header
all came from 610.43.02. The proprietary function is never executed.

```bash
python3 tools/windows-debug-capture/compare-runlist-engine.py \
  --proprietary-object /path/nv-kernel.o_binary \
  --ogkm-git /path/open-gpu-kernel-modules \
  --sdk-include /path/ogkm-580.65.06/src/common/sdk/nvidia/inc
```

Public-source checkouts: OGKM 580.65.06
`307159f2623d3bf45feb9177bd2da52ffbc5ddf9`; 535.309.01
`15b1a21dfe4eab54a463a0ea3086823ae35fe02e`; 610.43.02
`86856f779971146f807a7ae2a864b4aee147b553`.

Additional bounded negative searches: NVIDIA/open-gpu-doc
`9fdf5c4062007929d9f4e6cbad9c9771fe61b880` (current contents and all 88 commits
of the classes directory), envytools
`f102b82381f3f11cee113d16374c87091db039d9`, nouveau-wiki
`9a051d7e7e1de252d4d8efa6f610920a7e1d0c94`, gVisor's nvgpu/nvproxy source
`bc2e4d8a69cb7703793876690c8e670b98e13699`. No relevant `b297`/`NVB297`/
`clb297` definition was found. Search absence is not a claim about all sources.

Validation: both complete installer scans succeeded; parent/class-info and
control-owner relocations cross-checked; public layout compiled; all 85 engine
comparison inputs matched; Python compilation and `git diff --check` passed.
Raw driver binaries, disassembly, private Windows dumps and payloads are not
committed here.

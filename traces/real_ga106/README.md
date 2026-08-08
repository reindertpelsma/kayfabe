# `traces/real_ga106/` — what a **real GA106** answers, asked directly

★★★ These are not captures of this port, and they are not the C oracle. They are the
**physical-control reply plane of a genuine NVIDIA GA106**, read out of the driver that owns
it, on the day it was asked.

## Provenance

| | |
|---|---|
| part | NVIDIA GeForce RTX 3060 (GA106), `GPU-e28d7776-e4f9-704b-d392-d46f187343f8` |
| host | vast.ai instance `46494693`, kernel `6.8.0-59-generic` |
| driver | **NVIDIA open kernel modules 580.159.04** — the same version the Mode-2 guest runs |
| source | `research_clones/ogkm-580.159.04`, git tag `b81d58e` (`version.mk: 580.159.04`) |
| date | 2026-08-01 |
| method | that exact source rebuilt with `NV_PRINTF(LEVEL_ERROR, …)` probes, `insmod`ed in place of the stock DKMS module, and `RmInitAdapter` driven by `nvidia-smi` opening `/dev/nvidia0` |

The box was returned to its stock module afterwards.

### ⊘⊘⊘ THE METHOD ROW ABOVE IS ALSO THIS DIRECTORY'S BLIND SPOT — and it is shared

★★★ *"`RmInitAdapter` driven by `nvidia-smi`"* is **the same method** that produced the C
artifact's captured control table (`mode2_initctrl_ga106.h`) and `cap1_coldboot_hermetic`.
All three oracles this project owns were made by the same harness, so **none of them can
witness anything only CUDA asks for**, and three of them agreeing that a control is *"never
requested"* is not corroboration — it is one defect counted three times
(`a_table_does_not_decide_behaviour`: *"a correction from the same source is not an
independent check"*).

`[measured 2026-08-08]` That is exactly how `execution_plane_increments.md` §14.22 ruled
`NV2081_BINAPI` a *phantom*. §14.26 refuted the ruling; §14.27 measured what it should have
said. ⇒ **From `cuInit` onward, the instrument is a new capture and never a lookup here.**

The three files dated **2026-08-08** below were taken with a CUDA process, expressly to
close that hole, and their provenance row differs:

| | |
|---|---|
| host | vast bench `vh`, RTX 3060 (GA106) `10de:2504`, host driver **580.159.04 Open**, **stock** module |
| date | 2026-08-08 |
| method | ⊘ **no driver rebuild and no probes.** `scripts/rpctrace/cuinit_probe.c` drives `libcuda.so.1` directly (`dlopen`/`dlsym`, no toolkit) under `scripts/rpctrace/cuda_ioctl_trace.c`, an `LD_PRELOAD` interposer on `ioctl(2)`; plus `kayfabe-rm-ladder --binapi-ctrl` at rev `6c9e3d2bb` |

### `cuinit_ioctl_trace_real_ga106.txt`

The **whole** of `cuInit(0)` on a real GA106: 9 `NV_ESC_RM_ALLOC`s and ~60
`NV_ESC_RM_CONTROL`s, each with its params buffer **before and after** the call, bracketed by
`MARK` lines. This is the first capture this project has ever held of the region past
`nvidia-smi`.

★ Two facts in it are worth naming here because they bound what a port must serve:
`0x2080012f` (`GPU_QUERY_ECC_STATUS`) returns **`0x56` on real hardware** and `cuInit` still
succeeds — a refusal mid-`cuInit` is survivable; and `0x20800102`
(`NV2080_CTRL_CMD_GPU_GET_INFO_V2`) is **input-dependent**, so no fixed-body table row can
answer it correctly for an arbitrary request.

⚠ **In / out being identical is not evidence that nothing was written.** libcuda hands RM
zeroed buffers, so an all-zero pair is ambiguous by construction. That ambiguity is what the
next file exists to remove.

### `rmladder_r20_binapi_real_ga106.txt`

`0x20810108` on an `NV2081_BINAPI` allocated under the Subdevice, with the 992-byte buffer
**seeded to `0xCD`** — the one thing an interposer must never do. Two runs, byte-identical.

⇒ RM returns `NV_OK` and writes **6 bytes of 992**: offset `0` (1 byte), offsets `132..135`
(4 bytes), offset `984` (1 byte), all zero. **986 bytes are never touched.**
⊘ *"The reply is 992 zeros"* — the reading the interposed trace alone would have licensed —
is a **986-byte over-claim**. Same family as `c_oracle_empty_rows_are_wrong`, arrived at from
the opposite direction: there an empty capture was decoded as zeros, here a zero capture
would have been decoded as a written body.

★ The alloc is issued the way libcuda measurably issues it: `paramsSize=0`, params **NULL**.
RM's own RPC to GSP then carries `paramsSize=4`, because `RS_OPTIONAL(NV2081_ALLOC_PARAMETERS)`
declares that size for the registered class. The client-side ioctl and the guest-side wire we
must answer are **not the same number**.

### `cuinit_fault_injection_matrix.txt`

⚠ **Not a capture of hardware** — the same interposer in its opt-in injection mode, forcing
one status to `0x56` and nothing else, so that *"libcuda asks X"* can be separated from
*"libcuda needs X"*. §14.26 refused to close that gap by assertion; this closes it by
experiment, on the real part:

| refused (`0x56`) | `cuInit(0)` |
|---|---|
| nothing | `0` |
| control `0x20810108` | **`0`** — ⊘ **not load-bearing** |
| alloc `NV2081_BINAPI` (`0x2081`) | **`100`** |
| control `0x20800102` `GPU_GET_INFO_V2` | **`100`** |

## The files

### `fmb_real_ga106.txt`

The targeted measurement (RTX 3060, open 580.159.04, 2026-08-01), from two probes placed
independently:

- `kceGetFaultMethodBufferSize_IMPL` — the deserialised `NvU32` *after* the RPC returns;
- `kchangrpAllocFaultMethodBuffers_GV100` and `kfifoCalcTotalSizeOfFaultMethodBuffers_GV100`
  — the value as each consumer actually uses it.

```
0x20802a08 -> status=0x0 params.size=20480 (0x5000) sizeof=4
kchangrpAllocFaultMethodBuffers bufSizeInBytes=20480 (0x5000) runQueues=2
calcTotal perGroup=20480 maxChannelGroups=4096 runQueues=2
```

### `rpc_transcript_real_ga106.txt`

Every `ROUTE_TO_PHYSICAL` control the CPU-side RM sent to GSP during one cold
`RmInitAdapter`, logged at `rpcRmApiControl_GSP`'s reply (`ogkm-580: rpc.c:11064`) — **88
calls, 55 distinct commands**, each with its `paramsSize`, the GSP's own status, and the
first eight reply bytes.

⚠ **`head=` is the first 8 bytes only.** It is enough to settle a `NvU32` and to falsify an
"empty" row; it is *not* a reply body. A control needing more must be re-measured on an
RTX 3060 (open 580.159.04, as on 2026-08-01), and the recipe above is how — see the next
file, which is that re-measure.

### `rpc_bodies_real_ga106.txt`

★★★ **Whole reply bodies**, taken 2026-08-01 on the same part with the same recipe, with the
probe widened from an 8-byte head to a chunked full-body dump over an allowlist: **the eleven
`dlen = 0` rows of the C oracle's table, plus `0x20800a2a`** (3712 bytes, whose head was all
the transcript above could see). 612 lines, 12 distinct commands.

Every block is bracketed by a `BEGIN` carrying `psize` and an `END` carrying it again, and
every line is `+<offset> <up to 16 bytes>`, so a dropped line is detectable rather than
readable as a short reply — `crates/kayfabe-abi/tests/real_ga106_bodies.rs` asserts exactly
that before it compares anything.

★ Repeated blocks for one command are repeated **calls**, kept verbatim. `0x20800a6c`
answers `0x31` three times during adapter init and `0x11` afterwards in the same run — it
**echoes its `[IN]` `flags`**, and those are the two words `kbusVerifyBar2_GM107`'s call
sites pass. A deduplication would have hidden it, and it is what refuted
`kayfabe_abi::l2evict`'s claim that a real GSP answers this control with four zeros.

⚠ `0x20800a4c` (`INTERNAL_GPU_GET_SMC_MODE`) is **not** an `RmInitAdapter` control. It is
reached only when a client asks `NV2080_CTRL_GPU_INFO_INDEX_GPU_SMC_MODE`
(`ogkm-580: subdevice_ctrl_gpu_kernel.c:232-266`), so the run was widened with
`nvidia-smi -q` to reach it. It is the eleventh row and it was the only one the previous
transcript never saw.

## ★★★ Why this exists: the C oracle is wrong about a whole class of rows

The C research artifact's `src/qemu/mode2_initctrl_ga106.h` is labelled *"real, captured from
host"*, and for rows carrying a body it **is** — `0x20800a61` and `0x20800a80` match this
transcript byte for byte. But every row it recorded with `dlen = 0` is contradicted:

★★★ **All eleven were then re-measured, not a sample** (2026-08-01,
`rpc_bodies_real_ga106.txt`):

| control | C's row | real GA106 | verdict |
|---|---|---|---|
| `0x20802a06` | `psize 4, dlen 0` | `10 00 00 00` | **contradicted** |
| `0x2080017e` | `psize 8, dlen 0` | `00 00 00 02 00 00 00 00` | **contradicted** |
| `0x20800af3` | `psize 2, dlen 0` | `01 01` | **contradicted** |
| `0x20802a08` | `psize 4, dlen 0` | `00 50 00 00` | **contradicted** |
| `0xa06f0103` | `psize 3, dlen 0` | `01 00 00` | **contradicted** |
| `0xa06f0104` | `psize 4, dlen 0` | `0b 00 00 00` | **contradicted** |
| `0x20800a4b` | `psize 4, dlen 0` | `00 00 01 04` | **contradicted** |
| `0x20800aac` | `psize 4, dlen 0` | `00 00 01 00` | **contradicted** |
| `0x20800a6c` | `psize 4, dlen 0` | `31 00 00 00` **and** `11 00 00 00` | **contradicted**, and an `[IN]` **echo** |
| `0x20800a4c` | `psize 4, dlen 0` | `00 00 00 00` | ⚠ coincides |
| `0x20800a70` | `psize 0, dlen 0` | `<empty>` | ⚠ coincides |

**Nine of eleven are wrong**, and the two that are not are the interesting ones.
`0x20800a70` has `psize = 0` — there is no body it could have failed to capture, and that is
checkable *from the row*. `0x20800a4c` genuinely answers zero because SMC is disabled on this
part — and **nothing about its row says so**; in the capture it is byte-identical to the nine
that are wrong.

⇒ *"the oracle answered it with an empty body"* is **not** evidence that a real GSP answers
zero. It is a capture artefact, and a triage row that cites one is citing nothing.
`0x20802a08`'s row did exactly that for four rungs. The rule is now a named refusal in
`kayfabe_abi::oracle`, keyed on `psize > 0 && dlen == 0` rather than on a list of ids, and
`crates/kayfabe-device/tests/sweep_triage.rs` fails any triage row that cites one of these
rows without naming what hardware said.

## ★★ And the other direction: a FULL row was corroborated, all 3712 bytes

`0x20800a2a` (`INTERNAL_STATIC_KGR_GET_INFO`) carries `dlen == psize == 3712` in the C's
table. Asked of the real part it answered **byte-identically**, twice in one run. That is the
class result stated the other way round and it is why this file does not demote the artifact:
rows the C actually captured are right; rows it recorded empty say nothing. See
`kayfabe_abi::grinfo`.

⊘ **Untouched: the TRUNCATED rows** — `0 < dlen < psize`, of which `0x20800a22` (16 376 of
34 592) is the largest. A third class, not measured here. The recipe now works for whole
bodies of any size, so the measurement is available; it has not been taken.

⊘ This does **not** demote the C generally: it remains the only implementation a real driver
has accepted end to end, and its non-empty bodies are corroborated here. It demotes one
specific move — reading an empty `ctl_` array as *"hardware says zero"*.

## ⊘ What these traces do NOT establish

- ⊘ **They are one part, one driver version, one boot.** Nothing here is a claim about AD10x,
  GH100, or a different driver.
- ⊘ **They are not a replay.** There is no ordering guarantee strong enough to diff against —
  they are an *answer sheet*, consulted per control, not a sequence.
- ⊘ **They cover the physical-control plane only.** Register accesses, the message queue, DMA
  and interrupts are all invisible to this probe.
- ⊘ **`gspst=0x56` means this part's own GSP refuses that control** — which is a fact worth
  having (`0x20800a87`, `0x20800b05`), but says nothing about whether *our* refusal is
  survivable in *our* guest.

## ★★★ §14.28's two additions, and they are of DIFFERENT KINDS

⚠ **`GPU-d0913685-1ec0-805a-e319-43a901a0e1ff` is a SECOND, DIFFERENT physical GA106.** The
provenance table above names `GPU-e28d7776`; every file added on 2026-08-08 was read from
the other part. That distinction is not bookkeeping — it is the whole finding of
`rmladder_r21_gpuinfo_sweep_real_ga106.txt`.

| file | boundary | what it is |
|---|---|---|
| `rmladder_r21_gpuinfo_sweep_real_ga106.txt` | ioctl, host | all 70 `GPU_GET_INFO_V2` indices, **one call each** (`getGpuInfos` breaks its loop on the first error, so a 70-index request measures only the first failure), tail seeded `0xCD` |
| `cuinit_ioctl_trace_guest_gt1_e6ed6bc.txt` | ioctl, **INSIDE THE GUEST** | ⊘ **not a real GA106 at all** — the same interposer run against *this port*, so it is the differential partner of `cuinit_ioctl_trace_real_ga106.txt` |

★★★ **`0x23` and `0x24` differ between the two parts** (`0x19ece058`/`0xb91e2532` here,
`0x4324d4e9`/`0x8708a4a8` there) and are stable across runs on each. They are **per-chip
identity values**; ⊘ no chip-family table may state them, and `kayfabe_abi::gpuinfo` refuses
them by name.

⊘ **And the R21 sweep is an oracle for FEWER rows than it prints.** The guest kernel resolves
32 of the 70 indices itself and forwards only the `default:` arm, so a row here is a claim
about GSP-RM **only** for an index the kernel forwards. For the other 32 it is the *host
kernel's* answer and says nothing about what a GSP would return.

## ★★★ §14.29–§14.31's additions, and the one that CLEARS a refusal

All read from `GPU-d0913685-1ec0-805a-e319-43a901a0e1ff` — the **second** physical GA106,
not the `GPU-e28d7776` of the provenance table above.

| file | boundary | what it is |
|---|---|---|
| `gpuinfo_bisect_guest_gis1_e6ed6bc.txt`, `cuinit_bisect_guest_w1429_49b182a.txt` | ioctl, **inside the guest** | §14.29's `NVSWEEP_GPUINFO` bisect, before and after |
| `rmladder_r22_businfo_{sweep,loaded}_real_ga106.txt` | ioctl, host | §14.30: all 52 `BUS_GET_INFO_V2` indices one call each, plus `0x2d` ×16 with the PCIe link **idle** and **under load** — the pair is the measurement, neither run alone is |
| ★ `rmladder_r23_atomics_real_ga106.txt` | ioctl, host | §14.31: `BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS`, eight arms over `{capType} × {tail seed}` |
| `cuinit_trace_guest_gt143{0,1}_*.txt` | ioctl, **inside the guest** | ⊘ not a real GA106 — this port under the same interposer, the differential partner of `cuinit_ioctl_trace_real_ga106.txt` |

### ⊘⊘ `rmladder_r23_atomics_real_ga106.txt` is the file that says the INSTRUMENT can be the finding

`--probe-ctrl` seeds every params byte `0xCD` so that *"RM wrote nothing"* is distinguishable
from *"RM wrote zeros"*. On `0x2080182a` that seed lands in **`capType`, an `[IN]` field**, and
the resulting `NV_ERR_NOT_SUPPORTED` was read for a rung as evidence that the control depends
on caller state. It does not: the same bare Subdevice answers `NV_OK` for `capType = 0`.

⇒ ★ **Before trusting any `--probe-ctrl` row in this directory, check the control's params
struct for `[IN]` fields.** The seed is sound on a pure-`[OUT]` struct and is an input
mutation on any other. R21's `GPU_GET_INFO_V2` and R22's `BUS_GET_INFO_V2` rows are safe
because both rungs *build* their request rather than seeding it whole; a bare `--probe-ctrl`
row is not.

### ★★★ And `cuinit_ioctl_trace_real_ga106.txt` can CLEAR a refusal, not only demand a serve

`[measured 2026-08-08]` `:49` shows a real GA106 answering `0x2080012f`
`GPU_QUERY_ECC_STATUS` with **`status=0x00000056`**. This port refuses it too, and it appears
in every boot's `unserviced fn 76` ledger — so that ledger entry is **not a gap**, and a rung
picked from the ledger alone would have chased it. The wall on that same boot is `0x20801303`
`FB_GET_INFO_V2`, which appears in the ledger **not at all** because the guest's own kernel
never forwards it. ⊘ The ledger is an instrument for what reaches the emulated GSP, and the
wall need not be there.

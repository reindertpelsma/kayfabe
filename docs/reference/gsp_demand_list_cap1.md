# The GSP demand list of `cap1_coldboot_hermetic`, and what each entry IS

**Task #179, the `replay-conformance` line.** Two answers — an ordered **demand list** and a
**DATA-vs-ACT** classification of every control in it — and two warnings (§1, §2) that must
be read before either is used.

| artefact | what it is |
|---|---|
| `docs/reference/gsp_demand_list_cap1.tsv` | the ordered demand list: 74 distinct entries, counts, first-occurrence record index |
| `docs/reference/gsp_demand_list_cap1.json` | the same, plus the **full ordered sequence** of all 178 command elements |
| `docs/reference/gsp_control_classification.tsv` | **DATA / ACT / MIXED / UNKNOWN** for all 53 distinct controls, each with an `ogkm-580` citation |
| `scripts/demand_list.py` | the extractor (trace → list) |
| `scripts/rm_ctrl_index.py` | the evidence tool (control id → `#define`, params struct, NVOC handler, `RMCTRL_FLAGS_*`) |

Regenerate:

```
scripts/demand_list.py traces/cap1_coldboot_hermetic.rec \
    --tsv docs/reference/gsp_demand_list_cap1.tsv \
    --json docs/reference/gsp_demand_list_cap1.json
```

---

## ⚠ 1. `cap1` IS A TRACE OF A BOOT THAT FAILS. THIS IS A LOWER BOUND, NOT THE LADDER.

`traces/README.md` and `docs/design/rpc_trace_capture.md` §0 both say it: `cap1` ends where
the C emulator stopped. Everything below is **what the driver got as far as asking for**
before that stop — never a statement that a working GA106 boot asks for nothing more.

This is not a caveat inherited from a doc; it is visible **inside the repo**, twice over,
and both measurements are quantified in §5:

- Nine of the 56 rows in the C's own captured control table `mode2_initctrl_ga106.h` are
  for controls `cap1` **never issues** — and all nine appear in
  `traces/real_ga106/rpc_transcript_real_ga106.txt`, an independent transcript of one
  **successful** `RmInitAdapter` on a real RTX 3060 (GA106, open 580.159.04, 2026-08-01).
- `traces/cap1b_coldboot_hermetic_d6.rec`, the same experiment re-captured with the full
  `nvidia-smi -q`, demands **97** distinct controls where `cap1` demands 53.

⇒ Read this list as *"at least these, in at least this order"*. A replay that serves
exactly this list and nothing else is not a replay of a booting driver.

## ⚠ 2. HOW MUCH OF IT IS CLASSIFIED, AND WHAT THE RESIDUE IS

**50 of 53 controls (94.3 %)** carry a bucket with an `ogkm-580` citation to the params
struct *and* to the code that decides direction. The residue is three ids:

| id | why it is `UNKNOWN` |
|---|---|
| `0x20810108` | `NV2081_BINAPI`. `__nvoc_export_info__BinaryApi` has `numEntries = 0` (`ogkm-580: src/nvidia/generated/g_binary_api_nvoc.c:384`), so CPU-RM has **no export, no flags, no `paramsSize` and no struct** for any BINAPI control; `binapiControl_IMPL` forwards the caller's buffer verbatim (`ogkm-580: src/nvidia/src/kernel/rmapi/binary_api.c:88-115`). |
| `0x20810110` | same |
| `0x20810111` | same |

⊘ These are not "probably DATA". CPU-RM models nothing of the payload, so **the direction
is undecidable from this tree** — the semantics live in closed GSP firmware, and only
observing real traffic (a payload byte-diff in vs out) can settle them. One further
control, `0x20802a06` (`CE_UPDATE_CLASS_DB`), is bucketed `MIXED` at **MEDIUM** confidence
for the same underlying reason: its CPU-side out-consumption is certain, but whether the
server-side "trigger" mutates or merely recomputes cannot be read here.

---

## 3. Why this classification is the half a trace cannot supply

`docs/design/rpc_trace_capture.md` §4 states the trap this task exists to avoid, and it is
the whole point of the second artefact:

> A replay that answers an **ACT** from a captured table does not fail at the control. It
> fails **late** — as a hang or wrong data hundreds of RPCs later, with nothing pointing
> back.

A trace enumerates *demand*. It cannot distinguish a control whose reply **is** the answer
from one whose reply merely **acknowledges** that something happened on the device, because
in a capture both look like `status = NV_OK` and some bytes. That distinction is a static
property of the driver, so it is read out of `ogkm-580` — never guessed from the trace and,
⊘ **never guessed from the name**. Three rows in the table are exactly why:

- `..._STATIC_KGR_GET_SM_ISSUE_RATE_MODIFIER` **reads** the fused issue-rate values.
  "MODIFIER" modifies nothing (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:1393-1407`).
- `CE_UPDATE_CLASS_DB` sounds like an act, and its reply **is** consumed —
  `stubbedCeMask` drives class-DB edits (`ogkm-580: src/nvidia/src/kernel/gpu/ce/kernel_ce.c:634-654`).
- `CE_GET_FAULT_METHOD_BUFFER_SIZE` sounds like data, and **is** data — but a wrong answer
  is not caught at the control: it sizes a buffer RM DMAs CE fault records into
  (`ogkm-580: src/nvidia/src/kernel/gpu/ce/kernel_ce.c:844-847`, `.../gpu.c:6030-6037`).

### The one mechanical witness, and its one direction

`RMCTRL_FLAGS_CACHEABLE` (0x400) is defined as *"the control output does not depend on the
input parameters and can be cached on the receiving end"*
(`ogkm-580: src/nvidia/inc/kernel/rmapi/control.h:255-260`). That is NVIDIA declaring a
control **DATA**, and it can be read straight out of the NVOC export table.

⊘ **It is a one-way witness.** Its absence is evidence of nothing — most of RM predates the
flag, and 30 of the 38 DATA rows below do not carry it. Inverting it would have
misclassified `INTERNAL_MEMSYS_GET_STATIC_CONFIG`, whose caller passes its own cached
config struct in as the params buffer (`ogkm-580: src/nvidia/src/kernel/gpu/mem_sys/kern_mem_sys.c:474-477`).

`RMCTRL_FLAGS_CACHEABLE_BY_INPUT` (0x20000, `…control.h:293-299`) is the second tier: DATA,
but the reply depends on the request, so a table must be **keyed on the params**, not on
the cmd id. Four rows in the list are input-keyed and are flagged as such
(`0x20801112` on `baseIndex`, `0x20800a61` on `runlistId`, `0x20802a0f` on `lceType`,
`0x20800102` / `0x20800810` on the index list).

---

## 4. The demand list

`cap1_coldboot_hermetic`: 359 062 records, dense, `n_errors = 0`, hermetic
(`m2fwd=off m2exec=off m2romregs=off`), sha256 `5fa95640…97788`. It holds **178 GSP command
elements**, resolving to **74 distinct demands**: 8 RPC functions other than
`GSP_RM_CONTROL`/`GSP_RM_ALLOC`, **53 controls**, and 13 allocation classes.

### 4.1 The split

| bucket | controls | share |
|---|---:|---:|
| **DATA** | 38 | 71.7 % |
| **ACT** | 11 | 20.8 % |
| **MIXED** | 1 | 1.9 % |
| **UNKNOWN** | 3 | 5.7 % |

The eleven ACTs — the rows a replay must **perform**, not answer:

| id | control | what must actually happen |
|---|---|---|
| `0x20800afe` | `INTERNAL_INIT_USER_SHARED_DATA` | GSP links the RUSD memdesc at the supplied physical address |
| `0x20800aff` | `INTERNAL_USER_SHARED_DATA_SET_DATA_POLL` | GSP's RUSD polling loop is reconfigured |
| `0x20800301` | `EVENT_SET_NOTIFICATION` | the notifier is armed/disarmed on GSP |
| `0x20800a70` | `INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR` | a sysmembar orders VIDMEM writes — **zero params** |
| `0x20800a6c` | `INTERNAL_MEMSYS_L2_INVALIDATE_EVICT` | L2 is invalidated/evicted (task #148) |
| `0x20802a0d` | `CE_UPDATE_PCE_LCE_MAPPINGS_V2` | the PCE↔LCE mapping is written |
| `0x20800a9f` | `INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER` | GSP pins and mirrors the client's PDEs |
| `0x90f10106` | `VASPACE_COPY_SERVER_RESERVED_PDES` | the same payload, client-facing; walker rebind + TLB invalidate |
| `0xa06f0103` | `GPFIFO_SCHEDULE` | the runlist is written (task #177) |
| `0xa06f0104` | `BIND` | the channel↔runlist binding is committed |
| `0x2080012b` | `GPU_PROMOTE_CTX` | GSP initialises/binds the GR context buffers |

★ Note what they have in common: **every one of them is on the path to first compute.** The
DATA majority is the bring-up *interrogation*; the ACTs are the memory barrier, the cache
invalidate, the page-table publication, the channel bind and the runlist write. The bucket
that a table can serve is the bucket that does not matter for `cuCtxCreate → matmul`.

### 4.2 The ordered head, to `GSP_INIT_DONE`

Ranks 0-25, in the order the driver issues them (record index in `cap1`):

```
 0  rpc      GSP_SET_SYSTEM_INFO                        141946
 1  rpc      SET_REGISTRY                               141947
 2  rpc      SET_GUEST_SYSTEM_INFO                      141948
 3  rpc      GET_GSP_STATIC_INFO                        141955
 4  rpc      INIT_GSP_TRACE_CRASH_BUFFER                141962
 5  ctrl     0x20800a36 INTERNAL_GPU_GET_CHIP_INFO      141969   DATA
 6  ctrl     0x20800a41 …GET_USER_REGISTER_ACCESS_MAP   141976   DATA   ← elemCount=3
 7  ctrl     0x208001b0 GPU_GET_CONSTRUCTED_FALCON_INFO 141985   DATA
 8  ctrl     0x20800a87 …NVLINK_GET_NVLINK_DEVICE_INFO  141999   DATA
 9  ctrl     0x20800a40 INTERNAL_GET_DEVICE_INFO_TABLE  142006   DATA   ← elemCount=7
10  ctrl     0x20801112 FIFO_GET_DEVICE_INFO_TABLE      142019   DATA
11  ctrl     0x20800a5c INTERNAL_INTR_GET_KERNEL_TABLE  142026   DATA
12  ctrl     0x20800a1c …MEMSYS_GET_STATIC_CONFIG       142033   DATA
13  ctrl     0x20801803 BUS_GET_PCI_BAR_INFO            142040   DATA
14  ctrl     0x20800af3 …CONF_COMPUTE_GET_STATIC_INFO   142048   DATA
15  ctrl     0x20800aac INTERNAL_BIF_GET_STATIC_INFO    142060   DATA
16  ctrl     0x20800a61 …FIFO_GET_NUM_CHANNELS  ×4      142067   DATA (keyed on runlistId)
17  ctrl     0x20802a08 CE_GET_FAULT_METHOD_BUFFER_SIZE 142088   DATA  ⚠ see §5.3
18  ctrl     0x20800afe INTERNAL_INIT_USER_SHARED_DATA  142095   ACT
19  ctrl     0x20800aff …USER_SHARED_DATA_SET_DATA_POLL 142102   ACT
20  alloc    NV01_ROOT                ×9                142109
21  alloc    NV01_DEVICE_0            ×9                142116
22  alloc    NV20_SUBDEVICE_0         ×9                142123
23  alloc    NV01_EVENT_KERNEL_CALLBACK_EX ×3           142130
24  ctrl     0x20800301 EVENT_SET_NOTIFICATION ×3       142137   ACT
25  ctrl     0x20800a59 INTERNAL_GMMU_GET_STATIC_INFO   142144   DATA
```

★ The order matches the real-hardware transcript exactly over its opening run —
`0x20800a36`, `0x20800a41`, `0x208001b0`×2, `0x20800a87`, `0x20800a40`, `0x20801112`,
`0x20800a5c`, `0x20800a1c`, `0x20801803` — against
`traces/real_ga106/rpc_transcript_real_ga106.txt`, which was logged from a genuine GA106's
`rpcRmApiControl_GSP` reply on 2026-08-01. Two independent instruments, one order.

The rest of the list, with names, counts and first-occurrence indices, is in the TSV; the
element-by-element sequence is in the JSON.

### 4.3 The eight non-control RPC functions

Decided the same way, from `ogkm-580: src/nvidia/generated/g_rpc-structures.h` and the
handlers in `ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c`. Lower rigour than §4.1 — each is
one struct and one handler rather than a caller study — so it is reported separately:

| fn | name | bucket | evidence |
|---:|---|---|---|
| 72 | `GSP_SET_SYSTEM_INFO` | ACT | `rpc_gsp_set_system_info_v` is an opaque `data` word carrying `GspSystemInfo` inward (`g_rpc-structures.h:1484-1489`) |
| 73 | `SET_REGISTRY` | ACT | packs the registry table in and is **async** — `rpcSetRegistry_v17_00`, `rpc.c:10668-10700`; no reply body is read |
| 1 | `SET_GUEST_SYSTEM_INFO` | ACT | all-in version/title strings (`g_rpc-structures.h:36-47`) |
| 65 | `GET_GSP_STATIC_INFO` | DATA | `portMemCopy(pSCI, …, rpcInfo, …)` copies the whole `GspStaticConfigInfo` out (`rpc.c:9696`) |
| 228 | `INIT_GSP_TRACE_CRASH_BUFFER` | ACT | `{pa, size}` in (`g_rpc-structures.h:1938-1944`) — GSP must map that buffer |
| 70 | `UPDATE_BAR_PDE` | ACT | `UpdateBarPde_v15_00 info` in (`g_rpc-structures.h:447-452`, `rpc.c:9703`) |
| 10 | `FREE` | ACT | `NVOS00_PARAMETERS` in, status out (`g_rpc-structures.h:160-165`); 45 calls in `cap1` |
| 47 | `UNLOADING_GUEST_DRIVER` | ACT | `{bInPMTransition, bGc6Entering, newLevel}` in (`g_rpc-structures.h:378-385`) |

★ `cap1` ends on `UNLOADING_GUEST_DRIVER` at record 357 346 — the trace covers a driver
**unload**, not only a bring-up.

⊘ The 13 allocation classes (`NV01_ROOT`, `NV01_DEVICE_0`, `NV20_SUBDEVICE_0`,
`NV01_EVENT_KERNEL_CALLBACK_EX`, `FERMI_VASPACE_A`, `AMPERE_CHANNEL_GPFIFO_A`,
`AMPERE_DMA_COPY_B`, `AMPERE_B`, `NV01_MEMORY_VIRTUAL`, `VOLTA_CHANNEL_GPFIFO_A`,
`FERMI_TWOD_A`, `NV40_I2C`, `NV2081_BINAPI`) are not bucketed here. `GSP_RM_ALLOC`
constructs server-side objects, so it is an act by construction; whether each *class* also
returns data through its alloc params is a separate question this task did not answer, and
it is recorded as open rather than assumed.

---

## 5. Cross-check against the C's captured table `mode2_initctrl_ga106.h`

The C artefact's table has 56 rows. `traces/c_oracle_census/initctrl_ga106_census.tsv`
classifies them — **29 complete, 16 truncated, 11 empty** — and
`scripts/census_initctrl.py --check` reports it still matches the C header today.

### 5.1 Demanded but absent from the table — 6

`0x00800294` `GPU_GET_BRAND_CAPS`, `0x20800102` `GPU_GET_INFO_V2`, `0x20800810`
`BIOS_GET_INFO_V2`, and the three BINAPI ids `0x20810108` / `0x20810110` / `0x20810111`.
All six are issued late (records 309 189-309 372), i.e. in `nvidia-smi`'s enumeration
rather than in bring-up — which is why the C's bring-up-era capture never saw them.

### 5.2 In the table but never demanded — 9, and every one is explained

`0x00730107`, `0x00730151`, `0x00730211`, `0x0073028b`, `0x20800a01`, `0x20800a49`,
`0x20800a4b`, `0x20800ac6`, `0x20800adf`.

★ **This set is exactly equal to the set of controls the real GA106 demands and `cap1` does
not** — nine ids, zero symmetric difference, checked against
`traces/real_ga106/rpc_transcript_real_ga106.txt`. Not one table row is unexplained, and
the nine rows the C captured but `cap1` never exercises are precisely the ones a
**successful** `RmInitAdapter` asks for. That is the sharpest single piece of evidence for
§1: the shortfall is `cap1`'s, not the table's.

### 5.3 Demanded, present, and the row is EMPTY — 10

⊘ Per the standing rule (`../nvidia-gpu-passthrough/CLAUDE.md`, the FIFTH LIMIT), an empty
capture is evidence of **nothing**, not evidence of emptiness. Treat every row here as
*unmeasured*:

| id | `psize` | count | bucket |
|---|---:|---:|---|
| `0x2080017e` `GPU_GET_VMMU_SEGMENT_SIZE` | 8 | 1 | DATA |
| `0x20800a4c` `INTERNAL_GPU_GET_SMC_MODE` | 4 | 2 | DATA |
| `0x20800aac` `INTERNAL_BIF_GET_STATIC_INFO` | 4 | 1 | DATA |
| `0x20800af3` `…CONF_COMPUTE_GET_STATIC_INFO` | 2 | 2 | DATA |
| `0x20802a06` `CE_UPDATE_CLASS_DB` | 4 | 2 | MIXED |
| `0x20802a08` `CE_GET_FAULT_METHOD_BUFFER_SIZE` | 4 | 5 | DATA |
| `0x20800a6c` `…MEMSYS_L2_INVALIDATE_EVICT` | 4 | 4 | **ACT** |
| `0xa06f0103` `GPFIFO_SCHEDULE` | 3 | 3 | **ACT** |
| `0xa06f0104` `BIND` | 4 | 1 | **ACT** |
| `0x20800a70` `…BUS_FLUSH_WITH_SYSMEMBAR` | **0** | 3 | **ACT** |

★★ **The classification separates two kinds of empty that the census cannot tell apart.**
`0x20800a70` is the one row whose `dlen = 0` is *correct*: its NVOC export declares
`paramSize = 0 /* Singleton parameter list */`
(`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:2986`) and its header says the command
"accepts no parameters" (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:1159-1161`).
There is nothing to have captured. The other nine are gaps. A census counting bytes reports
all ten identically; only reading the driver splits them — and note the direction of the
error it prevents, which is *not* the one the FIFTH LIMIT warns about: here the risk is
**refusing** a row that is genuinely complete.

★ And note which rows are empty: **three of the four ACTs in the table are among them**
(`0x20800a6c`, `0xa06f0103`, `0xa06f0104`), plus the singleton `0x20800a70`. An ACT has no
`[out]` fields, so an all-`[in]` control *legitimately* produces a short or empty capture —
which means an empty row is precisely the case where a replay is most tempted to serve
zeros and most certain to be wrong, because there was never anything to serve.

### 5.4 Demanded, present, and the row is TRUNCATED — 11

A limit the project's notes emphasise less than the empty rows, and it is the same defect
in a milder form: `dlen < psize`, so decoding the row as complete silently yields zeros in
the tail.

| id | `psize` | `dlen` | short by |
|---|---:|---:|---:|
| `0x20800a22` `…STATIC_KGR_GET_GLOBAL_SM_ORDER` | 34 592 | 16 376 | **18 216** |
| `0x20800b03` `…SM_ISSUE_RATE_MODIFIER_V2` | 16 352 | 8 192 | 8 160 |
| `0x20800a40` `INTERNAL_GET_DEVICE_INFO_TABLE` | 24 580 | 16 384 | 8 196 |
| `0x20802a0f` `…CE_GET_PCE_CONFIG_FOR_LCE_TYPE` | 28 | 16 | 12 |
| `0x20800a1f`, `0x20800a34`, `0x20800a9f` | | | 8 each |
| `0x208001b0`, `0x20800301`, `0x20800a41`, `0x20801112` | | | 4 each |

All eleven are DATA or ACT rows whose *tail* is missing; the three largest lose more than
half the reply. `0x20800a22` and `0x20800a40` both stop at a 16 KiB boundary, which is what
a truncating capture looks like rather than what a device returns.

---

## 6. Three things that refuted an expectation

1. **The demand list is not dominated by acts.** 71.7 % is DATA, and the ACT rows are a
   short, nameable list. The wall a replay meets is narrow — but every ACT sits on the path
   to first compute (§4.1).
2. **`CE_UPDATE_CLASS_DB` returns data.** The name says act; `stubbedCeMask` is read and
   drives class-DB edits (`ogkm-580: src/nvidia/src/kernel/gpu/ce/kernel_ce.c:634-654`). It
   is bucketed MIXED at MEDIUM confidence rather than ACT, and the reason is written down.
3. **The empty rows and the ACTs overlap, and not by accident.** An all-`[in]` control has
   no reply body to capture (§5.3). The C's table is missing bodies exactly where a body
   was never going to exist — which is the strongest available argument that
   `mode2_initctrl_ga106.h` should never have been the interface a replay talks through.

## 7. What this does not settle

- ⊘ The 13 allocation classes are unbucketed (§4.3).
- ⊘ Three BINAPI controls are `UNKNOWN` and are not decidable from open source (§2).
- ⊘ `0x20802a06` is MEDIUM confidence pending the GSP-side body.
- ⊘ Everything here is read out of `ogkm-580.159.04`. `research_clones/ogkm` is 610.43.02
  and disagrees with it (memory: `ogkm_is_versioned`); no claim here has been checked
  against 610, and none should be assumed to carry over.
- ⊘ Nothing here has been run against hardware. The classification is a **reading**, not a
  measurement; the trace side is a decode of a committed capture, and the two are different
  kinds of evidence throughout.

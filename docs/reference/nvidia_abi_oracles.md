# The NVIDIA ABI oracles — what each one says, and where they disagree

**What this file is.** The cited ground truth behind `crates/kayfabe-abi`, kept in
`docs/reference/` per this repo's convention: facts with a `file:line`, correctable in one
place, deliberately *not* mixed into a design doc. Every number here is asserted by a test in
`crates/kayfabe-abi/tests/oracle_layout.rs`, so this file and the code cannot drift silently.

**What it is not.** The generation strategy, the version-table design and the safety argument
live in `crates/kayfabe-abi/src/lib.rs`'s crate docs, next to the code they constrain. The
two-axis versioning research is `../../../nvidia-gpu-passthrough/docs/design/mode2_abi_agnostic_layer.md`.

---

## 0. The oracles

| oracle | what it is | version it speaks for | where |
|---|---|---|---|
| **ogkm** | NVIDIA's own FINN-generated headers | **610.43.02**, one snapshot | `../../../nvidia-gpu-passthrough/research_clones/ogkm/` |
| **ogkm-580** ★ | the same, at the **bench's exact driver** | **580.159.04** | `../../../nvidia-gpu-passthrough/research_clones/ogkm-580.159.04/` |
| **nvproxy** | gVisor's independent transcription, versioned | 535.104.05 → 590.48.01, 17 entries | `../../../nvidia-gpu-passthrough/gvisor/pkg/abi/nvgpu/`, `.../sentry/devices/nvproxy/version.go` |
| **the C artifact** | this project's working Mode-1/Mode-2 implementation | 535 / 570≡575 / 580 profiles | `../../../nvidia-gpu-passthrough/src/abi/`, `src/common/nvkvm_abi.h`, `tests/abi_parity/` |

The generated code comes from **ogkm**. nvproxy and the C artifact are the independent checks —
a generator validated only against its own input proves nothing.

★ **The bench runs 580.159.04** (`rm_semantics_measured.md` §0). As of 2026-07-28 that version is
**vendored directly** (`ogkm-580.159.04/version.mk:1`, tag `580.159.04` from
`github.com/NVIDIA/open-gpu-kernel-modules`, `git clone --depth 1 --branch 580.159.04`), so for
anything the two trees disagree on the bench claim is now `[src]` rather than an inference from a
boundary. `research_clones/ogkm/` stays pinned at 610.43.02 — existing `file:line` citations against
it must keep resolving; **cite the tree you mean by name**.

Between the two, 610.43.02 remains the *forward* oracle (what the driver is becoming) and
580.159.04 the *bench* oracle (what the bench actually runs). Where they differ, §6 records the
difference; neither is "the" answer.

---

## 1. ★ FINDING — `NVOS46_PARAMETERS`: the oracles disagree, and the C's own test is the outlier

| source | says | citation |
|---|---|---|
| ogkm 610.43.02 | **64** bytes; `flags2` @ +36, `kindOverride` @ +40, `dmaOffset` @ +48, `status` @ +56 | `nvos.h:2168` |
| nvproxy, < 580.65.06 | **56** bytes; `dmaOffset` @ +40, `status` @ +48 | `frontend.go:625-639` |
| nvproxy, ≥ 580.65.06 | **64** bytes | `frontend.go:654-668`, switched at `version.go:1057-1059` |
| C artifact — **runtime** | 56 for the 535/570 profiles, **64** for the 580 profile | `nvkvm_abi.h:66,76,86` (`.nvos46_size`, `.nvos46_status_off`) |
| C artifact — **parity test** | **56**, unconditionally | `abi_parity_test.go:68-71` |

**The reading.** ogkm, nvproxy and the C's *runtime* are consistent once you notice the struct is
versioned. The C's **parity test is the outlier**: it asserts one size for a struct the C's own
runtime knows has two, so the test is weaker than the code it guards and would stay green if the
580 branch of `nvkvm_abi.h` were deleted. Since the bench is 580.159.04 > 580.65.06, the C is
right at runtime on its own bench and its parity test is pinning a layout the bench does not use.

**Why it matters beyond bookkeeping.** The two shapes have *the same prefix*, so a stale 56-byte
reader on a 64-byte buffer does not fail — it reads `kindOverride` as the low half of `dmaOffset`
and returns a plausible wrong GPU VA. Length alone cannot catch that direction. Pinned by
`mean_wire.rs::the_same_bytes_decode_differently_under_the_two_tables`.

**Second-order finding.** The C selects its profile from the **major version alone**
(`nvkvm_abi.h:112-121`, `nvkvm_abi_id_for_major`), but the boundary is at **580.65.06**. Any
hypothetical 580.x below .65.06 is mis-classified. The same coarseness applies to
`NVOS47_PARAMETERS`, whose boundary is **550.54.04** (`frontend.go:707-710`), also mid-major.
And `nvkvm_abi_by_id` **falls back to the 570 profile** for an unrecognised id
(`nvkvm_abi.h:105-110`), so an unknown driver silently gets 575's struct sizes.
`kayfabe-abi` keys on all three components and refuses below its floor.

---

## 2. ★ FINDING — `sizeof(rpc_message_header_v03_00)` is 32, and the C emulator says 36 once

- `nvkvm_gpu_emul.c:1586` — `stl_le_p(el + 56, 36u); /* length = sizeof(rpc_message_header) */`,
  on the bare-header path that posts `GSP_INIT_DONE`.
- `nvkvm_gpu_emul.c:1637` — "the GSP message element is {48-byte element header, **32-byte
  rpc_message_header**, params…}, so params live at `el+48+32 = el+80`".
- `nvkvm_gpu_emul.c:1657` — `stl_le_p(el + 56, 32u + 32u); /* rpc.length = hdr(32) + body(32) */`.

ogkm agrees with **32**: seven `NvU32` plus a 4-byte union (`g_rpc-message-header.h:41-52`).
The `36` is benign *today* only because the message is zero-padded and both sides checksum the
declared length, so the extra four bytes are four zeros nobody reads. It is still a wrong constant
on the boot path, and it is exactly the class of thing a generated layout removes.

---

## 3. ★ FINDING — `NV0000_ALLOC_PARAMETERS` has only ONE oracle

`grep -r NV0000_ALLOC_PARAMETERS` finds **nothing** in `gvisor/pkg/` and **nothing** in
`nvidia-gpu-passthrough/src/`. Neither nvproxy nor the C artifact models the client-root alloc
params at all. So the full 120-byte layout (`cl0000.h:47-52`: `hClient`, `processID`,
`processName[100]`, `pOsPidInfo`) rests on ogkm 610.43.02 alone.

This matters because `processID` is **the decision-#14 grouping discriminator**
(`l1_concurrency.md` §12.27) — the single field that decides whether a guest client is a user
process or the guest kernel. The corroboration it *does* have is RM's own writer, which sets the
two prefix fields by name (`ogkm src/nvidia/inc/kernel/vgpu/rpc.h:55,70,74`):

```c
root_alloc_params.hClient = hclient;                       // :55
    ...
    root_alloc_params.processID = KERNEL_PID;              // :70   (privLevel >= RS_PRIV_LEVEL_KERNEL)
    ...
    root_alloc_params.processID = pClient->ProcID;         // :74   (was cited as :75 — that is
                                                           //        the NV_ASSERT on the next line;
                                                           //        corrected 2026-07-27, doc audit)
```

**Consequences taken in code, deliberately:**

- `ClientAllocFacts` is decoded from an **8-byte prefix contract**, not the whole struct — that is
  the exact extent of what is corroborated.
- `DriverAbi::alloc_param_size` returns **`None`** for `NV01_ROOT`/`NV01_ROOT_CLIENT`. Reporting
  120 would be a guessed size in the one table whose whole purpose is to refuse guessed sizes.
- `pOsPidInfo` has the shape of a recent addition (RM only sets it on the non-kernel path), so
  `sizeof` at 575 is genuinely unknown to us.

**The experiment that settles it:** vendor a second ogkm tag in `[550.54.04, 580.65.06)` and
regenerate. That also deletes `crates/kayfabe-abi/src/transcribed.rs` entirely.

★ **Partial progress, 2026-07-28.** `ogkm-580.159.04` is now vendored (§0) and its
`src/common/sdk/nvidia/inc/class/cl0000.h` is **byte-identical** to 610's, `NV_PROC_NAME_MAX_LENGTH`
still `100U` (`ogkm-580: src/common/sdk/nvidia/inc/nvlimits.h:47`). So the 120-byte layout —
`pOsPidInfo` included — is now confirmed at **two** tags spanning the 580→610 gap, and the "only one
oracle" hazard is narrowed to *"unknown below 580.159.04"* rather than *"unknown outside 610"*.
It does **not** settle 575: the target interval `[550.54.04, 580.65.06)` is still unvendored, and
`alloc_param_size` must keep returning `None`.

★ A related caveat found in the same macro: the whole `processID` assignment sits inside
`if (!IsT234DorBetter(pGpu))` (`rpc.h:57`). On Tegra T234D and later, RM does **not** set
`processID` at all, so it stays 0 and would decode as `User { pid: 0 }`. Irrelevant to a discrete
x86 target; a real hazard if this project ever targets Tegra.

---

## 4. Agreements worth writing down (all three oracles concur)

| struct | size | citations |
|---|---|---|
| `NVOS00_PARAMETERS` | 16 | ogkm `nvos.h:162`; nvproxy `frontend.go:255`; C `abi_parity_test.go:58` |
| `NVOS21_PARAMETERS` | 32 | `nvos.h:464`; `frontend.go:300`; `:56` |
| `NVOS54_PARAMETERS` | 32 | `nvos.h:2230`; `frontend.go:738`; `:59` |
| `NVOS55_PARAMETERS` | 28 | `nvos.h:2265`; `frontend.go:371`; `:60` |
| `NVOS64_PARAMETERS` | 48 | `nvos.h:479`; `frontend.go:788`; `:57` |
| `NVOS47_PARAMETERS` (≥550.54.04) | 48 | `nvos.h:2196`; `frontend.go:711`; `:78` |
| `NV0080_ALLOC_PARAMETERS` | 56 | `cl0080.h:54`; `classes.go:198`; `:120` |

`NVOS64`'s field *order* is pinned as well as its size, because `nvos64_abi_fix` was an order bug
and swapping two same-width fields leaves `sizeof` untouched.

`NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS`: the prefix `physAddress @+0`, `numEntries @+8`,
`flags @+12`, `hVASpace @+16` is confirmed by ogkm `ctrl0080dma.h:802-810` **and** by the C
emulator's live snoop offsets (`nvkvm_gpu_emul.c:2528-2536`, reading `cmd+120/128/132/136`). The
tail (`chId`, `subDeviceId`, `pasid`, total 32) has ogkm only; `pasid` looks like a recent
addition in the same family as the `NV_VASPACE_ALLOCATION_PARAMETERS` `+Pasid` growth the C
records at 580 (`nvkvm_abi.h:83`).

★ **Naming**: NVIDIA spells the Device's GPU index `deviceId`; this project's prose and
`AllocFacts::device_instance` call it the *device instance*. Same field — ogkm's own MIG shim
assigns `ws->nv0080Params.deviceId = migDev->deviceInstance` (`src/common/src/nv_smg.c:517`).

---

## 5. Why layouts are target-independent (x86_64 ≡ aarch64)

Derived rather than assumed, from `ogkm src/common/sdk/nvidia/inc/nvtypes.h`:

- Every SDK field uses a fixed-width NVIDIA typedef; no `long`, no `size_t`, no bare pointer.
- The one apparent exception is not one: `NvP64` is `void*` under `NV_64_BITS` (`:306`) and
  `NvU64` otherwise (`:326`) — **8 bytes on both arms**.
- `NV_ALIGN_BYTES(8)` / `NV_DECLARE_ALIGNED(x, 8)` expand to `__attribute__((aligned(8)))`
  (`:494`, `:508`). On any LP64 target a 64-bit scalar is already 8-aligned, so they are **no-ops**;
  they exist to fix up ILP32.

★ `NV_ALIGN_BYTES` expands to **nothing** on a compiler that is neither GCC-like nor `__arm`
(`:498-500`, with NVIDIA's own comment "XXX This is dangerously nonportable!"). Not a hazard for
this project — but it is why the generator treats an alignment attribute that would *raise* a
field's alignment as a hard error rather than emitting a plain `#[repr(C)]` mirror.

---

## 6. ★★ FINDING — the GSP message-queue element at 580 vs 610 (settles O4, O8, and half of O7)

Cites: `ogkm-580` = `research_clones/ogkm-580.159.04/`, `ogkm` = `research_clones/ogkm/` @610.43.02.
Paths below are relative to each tree's root. This section is the evidence behind
`mode2_gsp_port_plan.md` §9 D1–D3, §10 I6, §11 O4/O7/O8.

### 6.1 The element — 580 is the 48-byte form, confirmed first-hand

```c
// ogkm-580: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:43-51
typedef struct GSP_MSG_QUEUE_ELEMENT {
    NvU8  authTagBuffer[16];   // @0
    NvU8  aadBuffer[16];       // @16
    NvU32 checkSum;            // @32
    NvU32 seqNum;              // @36
    NvU32 elemCount;           // @40
    NV_DECLARE_ALIGNED(rpc_message_header_v rpc, 8);   // @48
} GSP_MSG_QUEUE_ELEMENT;
```

`GSP_MSG_QUEUE_ELEMENT_HDR_SIZE = NV_OFFSETOF(..., rpc) = 48` (`:93`), `SIZE_MIN = RM_PAGE_SIZE`,
`SIZE_MAX = 16 * SIZE_MIN` (`:91-92`), `HEADER_ALIGN 4` / `ELEMENT_ALIGN RM_PAGE_SHIFT` (`:101-104`).
Byte-identical to the C's transcription and to nouveau's r535/r570. **I6 is settled: the C is right
for 580.** There is no CC-only header growth at 580 — the auth tag and AAD are *in* the fixed
header, so `hdr_size_cc == hdr_size_plain == 48`.

### 6.2 ★ A 580 guest **does** read `elemCount` at +40, on the receive path, three times over

```c
// ogkm-580: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:652-659
if (i == 0) { /* Pull out the element count. This adjusts the loop condition. */
    nElements = pMQI->pCmdQueueElement->elemCount; }
```

and that value then drives, in the same function:

- the read/copy loop bound (`:628`, `:648-650`);
- the CC checksum span (`:676-677`) and the CC decrypt span (`:741-743`);
- **`msgqRxMarkConsumed(pMQI->hQueue, nElements)`** at `:774` — the ring advance.

580 **never derives** the element count from `rpc.length`; the only length check is a post-hoc
sanity test (`:760-770`) that neither cross-checks `elemCount` nor gates the consume. 610 is the
mirror image: `nElements` is derived (`ogkm: message_queue_cpu.c:698-701`) and there is no
`elemCount` field at all.

Two consequences the port must hold, and they are **different invariants per version**:

| version | what MUST agree with how far we advanced `writePtr` |
|---|---|
| 580 | `elemCount` — a mismatch desyncs the ring pointer *independently of* `seqNum` |
| 610 | `rpc.length` (via `ceil((hdrSize+length)/4096)`) |

★ And a hard bound on 580: the staging buffer is `1<<12 + GSP_MSG_QUEUE_ELEMENT_SIZE_MAX +
msgqGetMetaSize()` = one page + **64 KiB** + meta (`:132-134`, `:143-145`), while the copy loop
writes `nElements * 4096` into it with **no upper bound on `elemCount`** and a ring holding
`msgCount = (0x40000 - 4096)/4096 = 63` elements (`ogkm-580: src/common/shared/msgq/msgq.c:237-252`).
Emitting `elemCount > 16` corrupts the guest kernel heap (it lands on
`pMetaData` first). `elemCount ∈ [1, 16]` is not a style rule, it is a guest-memory-safety bound.

### 6.3 ★ MCTP/NVDM: **not present at 580 at all**, and the 610 words are these

The 610 header words the port carried as opaque placeholders are, transcribed:

| word | offset (610) | value | derivation |
|---|---|---|---|
| `mctpHeader` | @0 | **`0xC000_0001`** | `REF_NUM(VERSION[3:0],1) \| SEID=DEID=SEQ=0 \| EOM[30]=1 \| SOM[31]=1` |
| `nvdmHeader` | @4 | **`0x2510_DE7E`** | `TYPE[6:0]=_VENDOR_PCI 0x7e \| IC[7]=0 \| VENDOR_ID[23:8]=_NV 0x10de \| NVDM_TYPE[31:24]=NVDM_TYPE_RM_RPC 0x25` |

Field definitions `ogkm: src/nvidia/arch/nvalloc/common/inc/mctp_format.h:39-58`; constructors
`mctpCreateTransportHeader` `:78-94` and `mctpCreateNvdmHeader` `:108-118`; the SOM/EOM/SEID/DEID/SEQ
arguments the GSP path passes are literal `1,1,0,0,0` at
`ogkm: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:505-512`; `NVDM_TYPE_RM_RPC 0x25` at
`ogkm: .../nvdm_format.h:61`. `REF_NUM`/`REF_DEF`/`REF_VAL` are plain shift-and-mask
(`ogkm: src/common/sdk/nvidia/inc/nvmisc.h:336-341`). These are **exact**, not placeholders.

610 validates exactly two fields on receive, and nothing else in either word —
`REF_VAL(MCTP_HEADER_VERSION, mctpHeader) == 0x1` and
`REF_VAL(MCTP_MSG_HEADER_VENDOR_ID, nvdmHeader) == 0x10de`
(`ogkm: message_queue_cpu.c:737-758`). SOM/EOM/SEQ/NVDM-type are **not** checked. So the sufficient
610 emission is any word with nibble0 == 1 and vendor == 0x10de; we emit NVIDIA's own values anyway.

**At 580 these words do not exist.** `mctp_format.h` is not included by the 580 GSP path; the only
MCTP headers in the 580 tree are FSP/SEC2/NVSwitch
(`ogkm-580: src/nvidia/arch/nvalloc/common/inc/fsp/fsp_mctp_format.h`, `.../sec2/sec2_mctp_format.h`),
and `NVDM_TYPE_RM_RPC` is absent from the whole 580 tree. Bytes @0–@7 of a 580 element are
`authTagBuffer[0..8]`, which a CC-off guest never reads. **A 580 profile must carry
`transport: None`, not placeholder words** — writing 0xC0000001/0x2510DE7E there is inert (it only
feeds the checksum we compute anyway), but it encodes a protocol the guest is not speaking.

The bitfield *encoding* is stable across the break: 580's FSP header defines the same
`MCTP_HEADER_*` / `MCTP_MSG_HEADER_*` bit ranges and the same `0x7e`/`0x10de` constants
(`ogkm-580: fsp/fsp_mctp_format.h:34-53`). 610 hoisted that header to a common location and made
GSP a client of it. So the change is **which transport the GSP queue speaks**, not what MCTP means.

### 6.4 The break interval, narrowed to adjacent tags

Probed `src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h` at nine tags via
`raw.githubusercontent.com` (`mctpHeader` / `elemCount` presence):

| 575.64.05 | 580.65.06 | 580.159.04 | 580.173.02 | 590.44.01 | 590.48.01 | 595.44.02 | 595.84 | 610.43.02 |
|---|---|---|---|---|---|---|---|---|
| 48-byte | 48-byte | 48-byte | 48-byte | 48-byte | 48-byte | 48-byte | 48-byte | **16-byte + MCTP** |

`git ls-remote --tags` lists **no tag between 595.84 and 610.43.02**, so the interval is closed as
far as public tags allow: the break is **(595.84, 610.43.02]** — the whole 580/590/595 range is on
the 48-byte side. `ElementLayout`'s version predicate should therefore be "`>= 610` ⇒ MCTP form",
not "`> 570`".

### 6.5 Other 580-vs-610 differences that touch the port plan's citation table

| plan row / claim | 610 | **580** |
|---|---|---|
| `MESSAGE_QUEUE_INIT_ARGUMENTS` shape (D8, **O8**) | **9** fields — adds `queueElementHdrSize/SizeMin/SizeMax`, `queueHeaderAlign`, `queueElementAlign` (`gsp_init_args.h:30-45`) | **4** fields: `sharedMemPhysAddr`, `pageTableEntryCount`, `cmdQueueOffset`, `statQueueOffset` (`ogkm-580: gsp_init_args.h:29-34`); written at `ogkm-580: kernel_gsp.c:4486-4489`. **Identical to nouveau r570.** Geometry is *not negotiated* at 580 — it is compile-time in `message_queue_priv.h:91-104` and the faked GSP must hardcode 48/4096/65536/4/12 |
| `GSP_ARGUMENTS_CACHED` tail | + `rmStateMonitorBufferArgs`, `bindataArgs` | absent (`ogkm-580: gsp_init_args.h:45-64`) — different total size *and* different offsets after `messageQueueInitArguments` |
| runtime geometry fields on `MESSAGE_QUEUE_INFO` | `queueElementHdrSize` etc. set at `message_queue_cpu.c:82-91` | do not exist (`ogkm-580: message_queue_priv.h:53-73`) |
| **O7** "`GSP_RUN_CPU_SEQUENCER` is not implemented, do not emit it" | true — enum only (`rpc_global_enums.h:255`) | **false.** Fully implemented: dispatch `ogkm-580: kernel_gsp.c:1486-1487`, and it is one of six entries in the bootup-without-API-lock allowlist (`:1464-1481`, vs eight *different* entries at `ogkm: kernel_gsp.c:1419-1440`). The executor is `kgspExecuteSequencerCommand_TU102` (`ogkm-580: kernel_gsp_tu102.c:913`), **deleted at 610** |
| init RPCs `SET_SYSTEM_INFO` → `SET_REGISTRY` (plan cites `kgspSendInitRpcs`, `kernel_gsp.c:4686-4709`, called *inside* `kgspBootstrap`) | as cited (`kernel_gsp_tu102.c:571-583`) | **`kgspSendInitRpcs` does not exist at 580.** It is `kgspQueueAsyncInitRpcs_IMPL` (`ogkm-580: kernel_gsp.c:3753-3777`), called **before** `kgspBootstrap_HAL` (`:4141`), i.e. before FWSEC / Booter Load / RISCV start / status-queue link. Same two RPCs, same order — but they are already in the command queue, doorbell rung (`:425`), when the GSP "boots". Skipped only if SPDM is enabled (`:4123-4133`) |
| `INTERRUPT_PROCESSOR_SUSPENDED_VALUE 0x80000000` on MAILBOX0 (plan: `kernel_gsp_tu102.c:333, 336-357`) | `(mailbox & 0x80000000) != 0` — **masked** | `(mailbox == 0x80000000)` — **exact equality** (`ogkm-580: kernel_gsp_tu102.c:1226-1238`). Polled after fn-47 (`ogkm-580: kernel_gsp.c:4310`) *and* as a fallback in bootstrap (`kernel_gsp_tu102.c:551`). On the bench we must write the value, not OR the bit |
| GSP resume handoff | none — removed | 580 additionally polls **`NV_PGC6_BSI_SECURE_SCRATCH_14._BOOT_STAGE_3_HANDOFF == _VALUE_DONE`** and SEC2 `FALCON_MAILBOX0` in `_kgspIsReloadCompleted` / `CORE_RESUME` (`ogkm-580: kernel_gsp_tu102.c:319-329, 913-950`). A different register from anything 610 reads |
| `kgspBootstrap_TU102` ordering | as cited | same shape, different lines: `ogkm-580: kernel_gsp_tu102.c:493-578`; mailbox program `:533` (writer `:363-373`), Booter Load `:537`, RISCV-active-or-suspended `:551`, status-queue link **NORMAL-only** `:568-571`, `kgspWaitForRmInitDone` `:573` |

Rows that were checked and are **unchanged** (byte-identical files, or identical semantics at
different lines): the whole `msgq` layer — `msgq.c`, `msgq.h`, `msgq_priv.h` are byte-identical
apart from `#include` placement, so every `msgqRxLink` / `msgqTxGetFreeSpace` / `SWAP_RX` /
`-7` claim holds verbatim at 580 (note the path moved: `ogkm-580: src/common/shared/msgq/`);
`g_rpc-message-header.h` (`rpc_message_header_v03_00` still 32 bytes); `libos_init_args.h`;
`dev_gsp.h` (`NV_PGSP_QUEUE_HEAD`, MAILBOX0/1); `dev_fb.h`'s WPR2 registers (610 only *adds*
fault-buffer registers); `kernel_falcon_tu102.c`; `rpc_common.c` (cosmetic refactor only —
signature still written on send, still never checked on receive); `_checkSum32`; function numbers
`1/47/65/72/73/76`, events `0x1001/0x1003`; the `NV_ASSERT(0)` bootup gate; `maxRpcSize`; the
recursive-poll prohibition; `kgspWaitForRmInitDone` polling `(GSP_INIT_DONE, 0)`; the four
`kgspUnloadRm` callers; `cmdQueueOffset`/`statQueueOffset` as byte offsets.

---

## 7. Open items

1. **Vendor a second ogkm tag** in `[550.54.04, 580.65.06)`. Deletes `transcribed.rs`, settles
   `NV0000_ALLOC_PARAMETERS`'s size at 575, and is the "day-not-a-month" drill
   (`mode2_abi_agnostic_layer.md` §6, experiment V1) run for real.
2. **A regeneration CI job** — re-run the generator against a vendored tag and
   `git diff --exit-code`, so a hand edit to a generated file cannot survive review. It must be
   *optional* (skipped when the ogkm tree is absent), since ogkm is not a build dependency.
3. **The wire → `RmEvent` mapping**, once `kayfabe-core`'s `RmEvent`/`AllocFacts` settle.
4. **The rest of the slice**: per-class alloc params (channel, VASpace, memory — the fields
   `AllocFacts` still needs), the GSP-RPC payload structs, the UVM ioctls, and the per-command
   capability allowlist that closes the default-allow gap (`nvproxy_gap_analysis`).
5. **CI coverage for the generator crate.** `crates/kayfabe-abi/gen/` is deliberately its own
   cargo workspace (so ogkm is never a build dependency), which also means `cargo fmt --all`,
   `cargo clippy --workspace` and `cargo test --workspace` at the repo root do **not** reach it —
   the same gap the `fuzz` workspace has, and which CI closes for `fuzz` with a second
   `working-directory` step (`.github/workflows/ci.yml` — grep `working-directory: fuzz`;
   ~~pinned `:337-339`~~ — **citation drifted; it was at `:399` and `:425` as of 2026-07-27,
   and `:337-339` had become the `unsafe_code` lints gate, a different gate entirely. Cite the
   step, not the line**). The generator needs the
   same three steps. It is clean today (**20** unit tests — *was written as 21; counted
   2026-07-27: `gen/src/ctype.rs` 7 + `gen/src/parse.rs` 13, `emit.rs` and `main.rs` 0* —
   clippy-clean, rustfmt-clean, verified by hand),
   but "verified by hand" is exactly what this repo's gate discipline exists to replace.

   > ★ **This item is still open as of 2026-07-27** — `grep -n working-directory
   > .github/workflows/ci.yml` returns only the two `fuzz` steps; nothing reaches
   > `crates/kayfabe-abi/gen/`. Note the small irony that the *count* in this very item rotted
   > by one while the gate that would have caught it stayed unbuilt.
6. **Wire the ABI into the mean suite.** `testing_doctrine.md` §3.1 item 3 requires each
   milestone's cases to land in `tests/tests/l1_mean.rs`, not only in a fresh isolated file.
   `crates/kayfabe-abi/tests/mean_wire.rs` composes a realistic RM event stream, but it does so in
   its own file because `tests/tests/` was owned by another agent this round. Fold the decoded
   event stream into the shared mean run when the `RmEvent` mapping lands.

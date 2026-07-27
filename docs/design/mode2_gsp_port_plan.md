# `kayfabe-gsp` — the faked-GSP port plan

> **Status: DESIGN. No code was written and no bench was touched producing this.**
> `kayfabe-gsp` is a 34-line skeleton (`crates/kayfabe-gsp/src/lib.rs`); this file is the
> spec it gets built against.

## 0. How to read this file

**Tags**, as in `../reference/rm_semantics_measured.md`:

| tag | meaning |
|---|---|
| **[src]** | read from code, with `file:line`. The file is named by tree: `ogkm:`, `C:`, `rs:`. |
| **[measured]** | observed on hardware, with the run that observed it |
| **[inferred]** | a conclusion drawn from those. **Every one is also listed in §10** with the experiment that settles it. |
| **[open]** | not determined. §11. |

**The two source trees have different standing, and the difference is the point.**

| tree | standing | what a citation to it proves |
|---|---|---|
| `ogkm` = `/workspace/nvidia-gpu-passthrough/research_clones/ogkm` (**610.43.02**, `version.mk:1`) | **THE SPECIFICATION.** This is the guest's own code. It will not adapt to us. | that the guest driver *requires* something |
| `C` = `/workspace/nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c` (9 614 lines) | **EVIDENCE.** A working implementation on **GA106 / RTX 3060 / driver 580.159.04**. | that something *works on GA106 with 580* — never that it is the protocol |
| `nv` = `/workspace/nvidia-gpu-passthrough/research_clones/linux/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/{r535,r570}` | **INDEPENDENT CORROBORATION.** nouveau's clean-room GSP client for **r535 and r570** — a second implementation of the same protocol, written by different people, and the *nearest* trees to the bench's 580. | that a protocol reading is not a misreading of one header, **and** what the protocol looked like *before* 610 |

A `[src]` to `C:` is therefore always implicitly **[measured on GA106+580]**. Where this
plan claims a C behaviour generalises, it says why, and the strong form of "why" is *ogkm
does it generation-independently* or *nouveau independently agrees*.

★ **No tree here is the bench's driver.** ogkm is 610.43.02, nouveau carries r535 and r570,
and the bench runs 580.159.04 (`rs: crates/kayfabe-abi/src/versions.rs:BENCH_DRIVER`) — which
sits **between** r570 and 610 with no vendored source. §9 D1–D3 and D8 are where that gap is
load-bearing, and it is much bigger than the brief assumed: **three of the boot path's
structures have three different shapes across the three available trees.**

---

## 1. ★ The two axes, stated before anything else

The owner's constraint — *"it must remain agnostic for multiple GSP layouts"* — decomposes
into **two independent axes** that this project has already solved separately, and
conflating them is its own bug class.

| axis | varies with | existing seam | crate |
|---|---|---|---|
| **A — wire layout** | the **driver version** | `DriverAbiTable`, keyed on full `major.minor.patch`, `NoTableForVersion` below the floor (`rs: crates/kayfabe-abi/src/versions.rs:1-40`) | `kayfabe-abi` |
| **B — silicon behaviour** | the **GPU generation** | `trait Arch` + `GmmuFmt`/`UserdModel`/`PushbufferAbi` (`rs: crates/kayfabe-arch/src/lib.rs:334-395`) | `kayfabe-arch` + an arch-impl crate |

`kayfabe-gsp` is a **logic crate**: it is in both CI vocabulary gates already, and CLAUDE.md
rule 1 forbids a generation name or a `#[repr(C)]` NVIDIA layout inside it. So it may hold
**neither** a register offset **nor** a struct layout. It holds the *topology*.

### 1.1 INVARIANT — belongs in `kayfabe-gsp`

Each row cites the ogkm code that makes it version- and generation-independent
(`msgq` is a chip-agnostic shared library under `src/nvidia/src/libraries/`; `message_queue_cpu.c`
and `kernel_gsp.c` are `_IMPL`, not `_HAL`, i.e. one implementation for all chips).

1. **Ring discipline** — a `msgq` SPSC ring with a producer `writePtr` and a consumer
   `readPtr`, both **modulo `msgCount`**, free space `= readPtr + msgCount - writePtr - 1`.
   [src] `ogkm: src/nvidia/src/libraries/msgq/msgq.c:488-497` (`msgqTxGetFreeSpace`),
   `:639-667` (`msgqRxGetReadAvailable`).
2. **`MSGQ_FLAGS_SWAP_RX` pointer topology** — with both sides agreeing, each side writes
   the *other* queue's consumption pointer into **its own** backing store's rx header.
   [src] `ogkm: msgq.c:264-265, 268-278` and `:411-424`.
3. **The `msgqRxLink` acceptance predicate** — eight checks, listed in §4.2. [src]
   `ogkm: msgq.c:330-405`.
4. **Strictly monotonic per-message `seqNum`**, incremented once per *message* (not per
   element), never reset by a re-link. [src] `ogkm: message_queue_cpu.c:514, 620` (tx),
   `:762-780, 836` (rx).
5. **Element count derived from declared length**, `ceil((hdrSize + rpc.length) / elementSizeMin)`.
   [src] `ogkm: message_queue_cpu.c:698-705`; the primitive is
   `gspMsgQueueBytesToElements` (`ogkm: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:117-121`).
6. **Checksum = 64-bit XOR fold, folded to 32, over `hdrSize + rpc.length` rounded up to 8,
   with the `checkSum` field zeroed first, and the whole element must fold to 0.**
   [src] `ogkm: message_queue_priv.h:197-209` (`_checkSum32`),
   `ogkm: message_queue_cpu.c:515, 543, 726-734`.
7. **The RPC envelope's shape and its matching rule** — a reply is matched on
   `(function, sequence)`, both from `rpc_message_header_v`. [src]
   `ogkm: kernel_gsp.c:1824-1828` (`_kgspRpcDrainOneEvent`).
8. **The boot *ordering*** — FWSEC → boot-args published → Booter Load (WPR2 up) → init
   RPCs → status-queue link → `GSP_INIT_DONE`. [src] `ogkm: kernel_gsp_tu102.c:540-618`
   (`kgspBootstrap_TU102`). This is `_TU102`, i.e. **Turing→Ada**; Hopper+ overrides it
   (`ogkm: src/nvidia/src/kernel/gpu/gsp/arch/hopper/kernel_gsp_gh100.c`), so the *ordering*
   is a per-regime parameter and only the FSM's **shape** (a linear sequence of gated
   phases with one suspend/resume edge) is invariant. See §1.3.
9. **The three boot modes** `NORMAL / SR_RESUME / GC6_EXIT` and the fact that the latter two
   **skip** boot-args programming, init RPCs, and the status-queue link. [src]
   `ogkm: kernel_gsp_tu102.c:544, 561-566, 578-587, 607-611`.
10. **The bootup-window event allowlist** — during `kgspWaitForRmInitDone`'s poll, without
    the API lock, only 8 event functions may be delivered; anything else is `NV_ASSERT(0)`.
    [src] `ogkm: kernel_gsp.c:1419-1440`.
11. **`MSGQ_FLAGS_SWAP_RX` must be set in *our* tx header too.** `rxSwapped` is the **AND**
    of both sides' `flags` [src] `ogkm: msgq.c:411-412`; the guest always sets it
    (`message_queue_cpu.c:180`), and nouveau — an independent implementation — hardcodes
    `cmdq->tx.flags = 1` [src] `nv: r535/gsp.c:1171`. Getting this wrong flips the read-pointer
    polarity and **deadlocks silently, with no error**. (This resolves what was an `[inferred]`
    claim in an earlier draft.)
12. **Recursive polling is forbidden.** `_kgspRpcRecvPoll` asserts `!bPollingForRpcResponse`
    [src] `ogkm: kernel_gsp.c:2893`. ⇒ we must never post an unsolicited event that would make
    the guest issue a synchronous RPC while one is already outstanding.
13. **Large messages use `CONTINUATION_RECORD` (fn 71) with *incrementing* `rpc.sequence`**,
    and the guest asserts `lastSequence == firstSequence + recordCount`
    [src] `ogkm: src/nvidia/src/kernel/vgpu/rpc.c:2109-2147`, return path at `:2192, :2213`.
    `maxRpcSize = queueElementSizeMax - queueElementHdrSize` (`kernel_gsp.c:3186`).

### 1.2 PARAMETER — must NOT appear in `kayfabe-gsp`

**Axis B (generation) — behind `kayfabe-arch`:**

- Every register offset the FSM reacts to or serves. The C's whole set lives in one header
  already, which is the right shape: [src] `C: src/qemu/mode2_regs_ga10x.h` (its own comment
  says *"ALL arch-specific magic numbers live here … add `mode2_regs_<arch>.h` and select by chip"*).
- WPR2 geometry and its encoding (`NVKVM_WPR2_LO_VAL`/`HI_VAL`, `C: mode2_regs_ga10x.h:57-58`)
  and the fact that "up" is tested as `WPR2_ADDR_HI._VAL != 0` (`ogkm: kernel_gsp_tu102.c:1172-1180`).
- The **FWSEC/SEC2-booter mechanics**: that `STARTCPU` is `CPUCTL` bit 1, that a *normal*
  Booter Unload is distinguished from a Booter Load only by `SEC2 MAILBOX0 == 0xff`
  (`C: nvkvm_gpu_emul.c:4222-4234`, GA10x-conditioned), that the GSP core is RISC-V and its
  "active" bit is `RISCV_CPUCTL` bit 7 (`C: mode2_regs_ga10x.h:66-67`).
- Which mailbox carries the LibOS boot-args GPA: on Turing→Ada it is
  `NV_PGSP_FALCON_MAILBOX0/1` (`ogkm: kernel_gsp_tu102.c:392-403`), and the HAL name says
  so — `kgspProgramLibosBootArgsAddr_TU102`. Other regimes have other implementations.
- `INTERRUPT_PROCESSOR_SUSPENDED_VALUE = 0x80000000` read from `FALCON_MAILBOX0`
  (`ogkm: kernel_gsp_tu102.c:333, 346-348`) — a LibOS2/LibOS3 constant, not a protocol one.
- The interrupt vector the status queue is announced on (`SWGEN0` = falcon IRQ bit 6, GSP
  engine stall vector 155 on GA106 — `C: nvkvm_gpu_emul.c:1670`, explicitly derived from a
  captured GA106 interrupt table).
- The GSP cmd-queue doorbell register `NV_PGSP_QUEUE_HEAD(i) = 0x110c00 + i*8`, `__SIZE_1 = 8`
  [src] `ogkm: src/common/inc/swref/published/ampere/ga102/dev_gsp.h:38`, written by
  `kgspSetCmdQueueHead_TU102` (`ogkm: kernel_gsp_tu102.c:372-390`). The C hard-codes queue 0
  (`C: mode2_regs_ga10x.h:69`); **the register is indexed and the index is `queueIdx`**.
- ★ **Whole *steps* that exist only on some generations.** Hopper+ replaces the FWSEC/Booter
  sequence entirely: `kgspWaitForGfwBootOk_GH100` delegates to FSP secure boot
  [src] `ogkm: .../hopper/kernel_gsp_gh100.c:248-263`; the boot is a RISC-V BCR/STARTCPU path
  with the GSP-FMC args PA in the mailboxes [src] `:730-776`; and the driver then requires
  **`MAILBOX0` to read back 0** (a non-zero value that is not the boot-args PA is read as a
  fatal FMC error code) plus `FALCON_HWCFG2.RISCV_BR_PRIV_LOCKDOWN = UNLOCK`
  [src] `:500-544, 968-996`. `kgspIsWpr2Up_GH100` returns FALSE unconditionally under CC
  [src] `:236-245`, and `kgspTeardown_GH100` is a single RISC-V halt wait [src] `:1039-1049`.
  **This is the clearest evidence that the boot *sequence* is a parameter, not a protocol.**
- `MSGQ_MSG_SIZE_MIN = 16`, `MSGQ_META_MIN_ALIGN = 3`, `MSGQ_META_MAX_ALIGN = 12`
  [src] `ogkm: src/nvidia/inc/libraries/msgq/msgq.h:31-51` — driver constants used as bounds
  in the acceptance predicate, so they are Axis A, not chip.
- MCTP/NVDM constants (610+): `MCTP_MSG_HEADER_VENDOR_ID_NV = 0x10de`,
  `MCTP_MSG_HEADER_TYPE_VENDOR_PCI = 0x7e`, `NVDM_TYPE_RM_RPC = 0x25`
  [src] `ogkm: src/nvidia/arch/nvalloc/common/inc/mctp_format.h:39-58`,
  `.../nvdm_format.h:61`.

**Axis A (driver version) — behind `kayfabe-abi`:**

- `GSP_MSG_QUEUE_ELEMENT`'s layout and header size. **This changed between 580 and 610** — §9 D1.
- Whether MCTP/NVDM transport headers exist and are validated — §9 D2.
- `rpc_message_header_v`'s layout (already generated: `rs: crates/kayfabe-abi/src/generated/rpc.rs`,
  `RpcMessageHeaderV0300`, `SIZE = 32`).
- The `NV_VGPU_MSG_FUNCTION_*` / `NV_VGPU_MSG_EVENT_*` numbering
  (`ogkm: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h`).
- `queueElementSizeMin` / `SizeMax` / default queue sizes: **`RM_PAGE_SIZE` and `RM_PAGE_SIZE*16`,
  0x40000 each** [src] `ogkm: message_queue_cpu.c:70-72, 88-89`. These are *driver constants*,
  not chip constants — but see §1.3: they are also **discoverable at runtime** from the tx
  header, which is strictly better than either.

**Neither axis — a runtime feature flag:**

- **Confidential Compute.** `bEncryptionEnabled` shifts `queueElementHdrSize` by
  `sizeof(GSP_MSG_QUEUE_ENCRYPTION_TAG)` and changes both what the checksum covers and where
  the RPC header starts. [src] `ogkm: message_queue_cpu.c:78, 82-86`,
  `message_queue_priv.h:123-140`. Target is CC-off, but the header offset must be a *computed
  value*, never a constant.

### 1.3 ★ The discipline that makes most of this moot: **derive, don't declare**

The single highest-leverage design choice available here:

> **Every ring parameter we need is present in the tx header the guest itself wrote.**

`msgqTxCreate` publishes `version, size, msgSize, msgCount, flags, rxHdrOff, entryOff`
(`ogkm: msgq.c:234-250`), and the C already reads all seven out of the guest's
command-queue header and copies them into the status-queue header verbatim
(`C: nvkvm_gpu_emul.c:3437-3452`). That is not a hack — it is the correct architecture, and
it is what keeps `kayfabe-gsp` free of `RM_PAGE_SIZE`, `0x40000`, `msgCount = 63`, and the
`0x20` rxHdrOff the C hard-codes at `C:3358`.

★★ **And it goes further than the tx header.** On 610, `MESSAGE_QUEUE_INIT_ARGUMENTS` —
which the guest writes into the `RMARGS` region for us to read — carries **nine** fields, the
last five of which are exactly the parameters that would otherwise be constants:

```c
NvU64    sharedMemPhysAddr;   NvU32 pageTableEntryCount;
NvLength cmdQueueOffset;      NvLength statQueueOffset;
NvLength queueElementHdrSize; NvLength queueElementSizeMin; NvLength queueElementSizeMax;
NvU32    queueHeaderAlign;    NvU32 queueElementAlign;
```
[src] `ogkm: src/nvidia/inc/kernel/gpu/gsp/gsp_init_args.h:32-45`, populated at
`ogkm: kernel_gsp.c:5481-5490`. ⇒ **on 610 even the element header size is declared by the
guest**, so `bEncryptionEnabled` need not be inferred either (it is folded into
`queueElementHdrSize` at `ogkm: message_queue_cpu.c:82-86`).

The struct is **not** the same on older drivers — §9 D8 — so this is a *capability*, not an
assumption: read the fields when the version's layout has them, fall back to Axis-A constants
when it does not.

**Rule GSP-P1 (normative for the port).** `kayfabe-gsp` obtains ring geometry **only** from
what the guest declares — the tx header and `MESSAGE_QUEUE_INIT_ARGUMENTS` — validates it
against the `msgqRxLink` predicate (§4.2), and stores it in a `MsgqGeometry` value. It
contains **no** ring constant. Axis-A supplies only what the guest's version does not declare:
the element *field layout* always, and the header size on drivers whose init-args struct
predates the `queueElementHdrSize` field.

**What this does NOT solve, and where a real new seam is owed** — see §3.5 and §10-I7:
register *decode* (which BAR0 offset is `CPUCTL`) and register *service* (what value a WPR2
read returns) cannot be derived from the guest. `kayfabe-arch` has **no** seam for this
today. §3.5 names the trait to add.

---

## 2. Scope, and what `kayfabe-gsp` is not

Owned (from the crate's own doc comment, `rs: crates/kayfabe-gsp/src/lib.rs:1-19`, and
`C-repo: docs/design/mode2_rust_rewrite_architecture.md` §4.2):

1. the falcon boot FSM, **resettable in-process** (lesson L12);
2. the message-queue transport — geometry, ring, seqNum, checksum, element framing;
3. RPC decode/encode → abstract `RmEvent` (`rs: crates/kayfabe-core/src/rmgraph.rs:398`) and
   control intents.

Not owned:

- **Completion *policy*** — which os-event to post and when — is `kayfabe-completion`, per
  `Proc`. `kayfabe-gsp` owns the **single post point** and the flow-control gate. [src]
  `C-repo: mode2_rust_rewrite_architecture.md` §4.3.2.
- **The RM resource graph.** RPC decode emits `RmEvent`s; `RmGraph::apply` owns meaning.
- **Register *values* for non-GSP registers** (PTIMER, fuses, PCI-config mirror, PRAMIN).
  Those are the register model, which has **no crate yet** (§3.5, finding).
- **Any `#[repr(C)]`.** Wire layouts are `kayfabe-abi`'s, by CLAUDE.md rule 1.

---

## 3. The boot protocol as a state machine

### 3.1 What the guest actually does (ogkm — the specification)

`kgspBootstrap_TU102` in order [src] `ogkm: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:522-618`:

Before B0, `_kgspBootGspRm` **fails early if WPR2 is up** [src] `ogkm: kernel_gsp.c:4804-4812`
— this is the gate a stale emulator trips on a second `insmod`.

| # | step | ogkm line | guest-observable at our boundary |
|---|---|---|---|
| B0 | `kgspWaitForGfwBootOk` → `gpuWaitForGfwBootComplete` | `:1184-1202` | **three** things, in order: GSP falcon `CPUCTL.HALTED == TRUE` (a 2.05 s halt poll, `ogkm: kernel_falcon_tu102.c:331-359`); then `PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK` **bit 0 == 1**; then `..._GROUP_05(0)` bits 7:0 **== 0xFF**. [src] `ogkm: src/nvidia/src/kernel/gpu/arch/turing/kern_gpu_tu102.c:391-479` |
| B1 | `kgspExecuteScrubberIfNeeded` (if a scrubber ucode exists) | `:531-535` | falcon DMA + STARTCPU |
| B2 | **if NORMAL and frtsSize > 0**: `kflcnReset(GSP)` then `kgspExecuteFwsec` | `:544-557` | GSP falcon DMA (ucode load) + `CPUCTL` STARTCPU |
| B3 | `kflcnResetIntoRiscv` | `:559` | GSP falcon reset regs |
| B4 | **`kgspProgramLibosBootArgsAddr`** — `MAILBOX0 = lo32(addr)`, `MAILBOX1 = hi32(addr)` | `:562`, impl `:392-403` | **two mailbox writes. NORMAL only.** |
| B5 | `kgspExecuteBooterLoad(WprMeta PA)` | `:566-572` | SEC2 falcon DMA + mailbox args + STARTCPU → **WPR2 comes up** |
| B6 | **if NORMAL**: `kgspSendInitRpcs` = `GSP_SET_SYSTEM_INFO` (72) then `SET_REGISTRY` (73) | `:576-585`, impl `kernel_gsp.c:4686-4709` | **two commands on the cmd queue + doorbell, BEFORE the status queue exists** |
| B7 | `FALCON_OS = riscvDesc->appVersion` | `:588-589` | one register write |
| B8 | liveness gate: `kflcnIsRiscvActive(...) \|\| _kgspIsProcessorSuspended(...)` | `:592-603` | reads RISCV CPUCTL active bit / `FALCON_MAILBOX0 & 0x80000000` |
| B9 | **if NORMAL**: `GspStatusQueueInit` → `msgqRxLink` retry loop | `:607-611`, impl `message_queue_cpu.c:337-412` | **polls the status-queue tx header until it validates**, 4 s (`NV_U32_MAX` under `IS_EMULATION`), and **calls `kgspHealthCheck_HAL` every iteration** — a queued crashcat report converts the spin into an immediate `NV_ERR_RESET_REQUIRED` (`:398-403`) |
| B10 | `kgspWaitForRmInitDone` → `rpcRecvPoll(GSP_INIT_DONE, 0)` | `:613`, impl `kernel_gsp.c:6264-6283` | drains the status queue until `(function, sequence) == (0x1001, 0)` |

Teardown, `kgspUnloadRm_IMPL` → `kgspTeardown_TU102` [src] `ogkm: kernel_gsp.c:5213-5231`,
`kernel_gsp_tu102.c:660-703`:

| # | step | line | observable |
|---|---|---|---|
| T1 | `NV_RM_RPC_UNLOADING_GUEST_DRIVER` — **fn 47** | `kernel_gsp.c:5231`; id at `rpc_global_enums.h:57` | a **synchronous** command — `rpcUnloadingGuestDriver_v1F_07` ends in `_issueRpcAndWait` (`ogkm: rpc.c:9146-9170`). **We must reply**, or every `rmmod` blocks for the full RPC timeout |
| T2 | `kgspWaitForProcessorSuspend` — polls `FALCON_MAILBOX0` for bit 31 | `kernel_gsp_tu102.c:352-357` | **a poll that hangs the close if we never answer** |
| T3 | `kflcnReset(GSP)` + FWSEC-SB | `:672-696` | GSP falcon DMA + STARTCPU **(the "trailing teardown STARTCPU")**. The reset also polls `FALCON_DMACTL` until **both** `DMEM_SCRUBBING` and `IMEM_SCRUBBING` read DONE [src] `ogkm: kernel_falcon_tu102.c:177-194, 279-320` |
| T4 | `kgspExecuteBooterUnloadIfNeeded` → asserts WPR2 is **down** afterwards | `:699-701`; assert at `kernel_gsp_booter_tu102.c:184-187` | SEC2 STARTCPU with MAILBOX0 = the unload arg; then a WPR2 read that **must** read 0 |

And the gate that fails a second life: `_kgspBootGspRm` refuses if WPR2 is already up
[src] `ogkm: kernel_gsp.c:4805-4809` — *"unexpected WPR2 already up, cannot proceed with booting GSP"*.

### 3.2 The FSM

States are named for what is *true*, and every latch is a state component, not a bool
scattered across a struct. The C's eight scattered fields
(`C: nvkvm_gpu_emul.c:161-179, 187-196`: `bootargs_dumped, fwsec_ran, gsp_suspended,
gsp_reloaded, sec_mbox0, q_ready, mbox0, mbox1`) collapse into two values:

```text
BootPhase                     QueueState
─────────                     ──────────
Cold                          Unbound
  │ (STARTCPU on GSP falcon, phase==Cold|Halted)         │ boot-args published AND
  ▼                                                      │ RMARGS decodes AND
FwsecRan          {wpr2: Up}                             │ tx header validates
  │ (Booter Load: SEC2 STARTCPU, args != unload)         ▼
  ▼                                              Bound(MsgqGeometry, RingCursor)
Booted            {riscv_active: true}
  │ (queue bound + INIT_DONE posted)
  ▼
Running
  │ (fn-47 UNLOADING serviced)
  ▼
Suspending        {suspend_reported: true}   ← MAILBOX0 reads 0x80000000
  │ (teardown STARTCPU, or Booter Unload)
  ▼
Halted            {wpr2: Down}   ── QueueState forced to Unbound ──┐
  │                                                                │
  └────────────── next STARTCPU ─────────────────────────────────► Cold
```

**Guest-observable surface** — the *whole* read-side contract, i.e. everything a guest can
learn about our state. Each is a pure function of the FSM; nothing else may be served:

| what the guest reads | function of | ogkm consumer |
|---|---|---|
| GFW boot progress + PLM | constant (always complete / fully lowered) | `gpuWaitForGfwBootComplete` |
| GSP falcon `CPUCTL` | constant HALTED | `kflcnIsRiscvActive` neighbourhood |
| GSP `HWCFG2` RISCV_ENABLE | constant | `kflcnIsRiscvCpuEnabled`, `kernel_gsp_tu102.c:534-538` |
| falcon `DMATRFCMD` | constant IDLE | ucode-load loops |
| falcon `DMACTL` DMEM/IMEM scrubbing | constant DONE | `kflcnWaitForResetToFinish_TU102`, `ogkm: kernel_falcon_tu102.c:279-320` |
| **WPR2 ADDR_LO/HI** | `phase >= FwsecRan && phase < Halted` | `kgspIsWpr2Up_TU102:1172-1180`; gate at `kernel_gsp.c:4805` |
| **RISCV `CPUCTL` active** | same predicate | `kernel_gsp_tu102.c:592` |
| **`FALCON_MAILBOX0`** | `phase == Suspending` → `0x80000000`, else 0 | `_kgspIsProcessorSuspended:336-349` |
| falcon `IRQSTAT` bit 6 | `swgen0_pending` | `kgspService_TU102:1088` |
| falcon `IRQMASK`/`IRQDEST` | constant (SWGEN0 enabled) | same |
| **status-queue tx header in guest RAM** | `QueueState == Bound` | `msgqRxLink` |
| **status-queue elements + writePtr** | ring cursor | `GspMsgQueueReceiveStatus` |
| **cmd-queue readPtr (written into the *status* queue's rx header)** | ring cursor | `msgqTxGetFreeSpace` |
| an MSI/MSI-X raise | `swgen0_pending` rising edge | the guest ISR |

★ **Nothing else.** A read of any other offset returns the register model's default and
must not be a function of GSP state. This is stated as a rule because the C's `nvkvm_reg_read`
is a 100-line `switch` in which GSP state (`s->fwsec_ran`, `s->gsp_suspended`) is read at
four sites (`C: nvkvm_gpu_emul.c:1421, 1425-1426, 1431`) and mirrored into a RAM overlay at
a fifth (`C:1462-1474`) — five places that must agree. In Rust there is **one** function
`fn observe(&self, reg: GspReg) -> Option<u64>` and the RAM-overlay mirror is generated from
it, not written twice.

### 3.3 Transitions, exhaustively

| # | trigger | guard | effect | source |
|---|---|---|---|---|
| E1 | STARTCPU on GSP falcon | `phase ∈ {Cold, Halted}` | `phase ← FwsecRan` (WPR2 up) | B2. `C:4237-4262` |
| E2 | STARTCPU on GSP falcon | `phase == Suspending` | `phase ← Halted`. **WPR2 stays down. No queue re-bind. No INIT_DONE.** | T3 |
| E3 | STARTCPU on GSP falcon | `phase ∈ {FwsecRan, Booted, Running}` | **no-op** (idempotent; a re-STARTCPU without an intervening reset is not a new boot) | [inferred] I1 |
| E4 | SEC2 STARTCPU, args == unload-args | any | `wpr2 ← Down`; `phase ← Halted` | T4. `C:4222-4234` |
| E5 | SEC2 STARTCPU, args != unload-args | `phase == FwsecRan` | Booter Load; `phase ← Booted` | B5 |
| E6 | write to boot-args mailbox pair (both halves seen) | `phase ∈ {FwsecRan, Booted}` | **publish**: decode the LibOS region array → RMARGS → `MESSAGE_QUEUE_INIT_ARGUMENTS`; decode + validate the guest's cmd-queue tx header; **write the status-queue tx header**; `QueueState ← Bound`; **post `GSP_INIT_DONE`** | B4→B9. `C:3377-3494` |
| E7 | write to `QUEUE_HEAD(0)` | `QueueState == Bound` | service the command ring | B6. `C:4290-4293` |
| E8 | write to `QUEUE_HEAD(0)` | `QueueState == Unbound` | ★ **loud refusal, zero guest-RAM reads** — this is the security fix, §7 | §7 |
| E9 | fn-47 serviced | `phase == Running` | reply first, **then** `phase ← Suspending`. seqNums preserved. | T1→T2. `C:2450-2481` |
| E10 | `IRQSCLR` bit 6 write | any | `swgen0_pending ← false` | `C:4193-4200` |
| E11 | `device_reset` (§3.4) | any | `phase ← Cold`, `QueueState ← Unbound`, all seqNums 0 | L12 |

**Latches, and their exact scope.** The C has three one-shot latches, and *every one of them
is a bug surface*:

| C latch | C line | what it guards | Rust replacement |
|---|---|---|---|
| `bootargs_dumped` | `:161`, set `:4280`/`:4301` | "don't re-read the boot args" | **deleted.** E6 is idempotent by construction: re-publishing recomputes the geometry from the guest's *current* header and rebinds. A guest that re-publishes gets a correct rebind, which is exactly what a re-`insmod` needs. |
| `q_ready` | `:187`, set `:3485`, cleared `:2475` | "the queue GPA is valid" | **replaced by a type**: `QueueState::Bound(_)`. Not a flag beside a raw GPA — the GPA *only exists inside* the `Bound` variant, so a stale GPA is unrepresentable (§7). |
| `gsp_reloaded` | `:171`, set `:4211-4214`, consumed `:4258` | "distinguish re-boot from trailing teardown" | **deleted.** E2/E3 distinguish on `phase` alone. The C needed this heuristic because its `gsp_suspended` was cleared unconditionally at `:4257` *before* the classification could use it a second time. |

### 3.4 ★ `device_reset` — the spec, from the measurement

**[measured]** (`../reference/mode2_bench_lifecycle.md` §3, RTX 3060 / 580.159.04,
2026-07-25): after fn-47, the teardown STARTCPU arrives with `was_suspended == true` and
`C:4255-4283` classifies it as a **re-acquire**: it re-raises WPR2, re-posts `GSP_INIT_DONE`,
and re-latches `bootargs_dumped`/`q_ready`. The next driver life therefore points at the
**previous life's queue GPA** with a stale `cmd_readptr` and `stat_seqnum`, and because
`was_suspended` is now false the re-dump at `C:4280` never fires. Observed failure:
`msgqRxLink` timeout, **`-7`**, 71 064 retries.

★ **The return code corroborates the mechanism exactly.** `msgqRxLink` returns `-7` on
precisely one condition: `pQueue->rx.size != size` [src] `ogkm: msgq.c:387-390`. A freshly
allocated, zeroed status queue has `size == 0`, which passes the `-6` check
(`size < rx.entryOff + msgSize` → `0x40000 < 0` is false) and fails `-7`. So `-7` **is** the
signature of "the guest's new status queue never received a tx header" — the latch chain,
not WPR2, not a seqNum problem. Two independent sources agree.

**[src]** `C:4271` **WPR2 is correctly lowered**; `s->fwsec_ran = false` at `C:2471` really
does mirror Booter Unload. **A reset that models only WPR2 does not fix this.**

⇒ **The spec:**

```
fn device_reset(&mut self) {
    self.phase = BootPhase::Cold;          // WPR2 down, riscv inactive, not suspended
    self.queue = QueueState::Unbound;      // ← drops the QueueBinding BY VALUE.
    self.swgen0_pending = false;           // the GPA, the geometry, both cursors,
}                                          //   and both seqNums die with it.
```

Two properties this must have that the C's does not:

1. **It is total.** Every piece of GSP state is reachable from `phase` + `queue`, so the
   reset cannot forget a field. The C's reset is field-by-field at four separate sites
   (`C:2471-2475`, `C:4257-4258`, `C:9393-9399`, `C:3484-3485`) and they disagree.
2. **It is a *state transition*, not a poke.** E2 (trailing-teardown STARTCPU) and E11
   (`device_reset`) both land in a state where a subsequent STARTCPU is a genuine E1. There
   is no "was_suspended" to misread because the classification is the state itself.

**In-process resettability (L12) is a consequence, not a feature**: `device_reset` is a
pure method on a value, so a test may drive boot → run → teardown → boot again inside one
process with no QEMU restart, which is the whole point of the L12 lesson.

### 3.5 ★ FINDING — `kayfabe-arch` has no seam for any of this

`trait Arch` [src] `rs: crates/kayfabe-arch/src/lib.rs:334-395` exposes: `name`, `classify`,
`vchid_from_userd_flags`, `decode_doorbell`, `mmu`, `userd`, `engine_of_object`,
`is_case2_control`, `pushbuffer`. **None of these can express a register offset, a WPR2
encoding, or a falcon-boot convention.** There is also no `kayfabe-regs` crate, though
`C-repo: mode2_rust_rewrite_architecture.md` §4.2 lists one; the workspace has 16 crates and
none of them is it.

Per the coordinator's instruction — *name the trait method rather than hard-coding* — the
proposal is **one new Axis-B sub-seam**, on the existing `Arch`, returning trait objects in
the established `mmu()`/`userd()`/`pushbuffer()` style:

```rust
// kayfabe-arch — additions. No offsets here; this is the vocabulary + the seam.

/// The GSP-facing registers the boot FSM reacts to or serves. Abstract: no offsets.
pub enum GspReg {
    GfwBootProgress, GfwBootPlm,
    GspFalconCpuctl, GspFalconHwcfg2, GspFalconDmatrfcmd,
    GspFalconMailbox0, GspFalconMailbox1,
    GspFalconIrqstat, GspFalconIrqmask, GspFalconIrqdest, GspFalconIrqsclr,
    GspRiscvCpuctl,
    Sec2FalconCpuctl, Sec2FalconMailbox0, Sec2FalconDmatrfcmd,
    Wpr2AddrLo, Wpr2AddrHi,
    GspQueueHead(u8),
}

pub trait GspModel: Send + Sync {
    /// BAR + offset → which GSP register this is. `None` = not ours.
    fn decode_reg(&self, bar: BarId, off: u64) -> Option<GspReg>;
    /// Does this written value mean STARTCPU on a falcon CPUCTL?
    fn is_startcpu(&self, value: u64) -> bool;
    /// Do these SEC2 booter arguments mean *Unload* (WPR2 must come down)?
    /// GA10x: MAILBOX0 == 0xff. Deliberately a predicate, not a constant.
    fn is_booter_unload(&self, sec2_mailbox0: u32) -> bool;
    /// The value to serve for `reg` given the FSM's abstract observation.
    /// This is where WPR2 geometry, HALTED/ACTIVE bit positions and the
    /// suspend sentinel live.
    fn encode(&self, reg: GspReg, obs: &GspObservation) -> u64;
    /// The interrupt this generation announces status-queue traffic on.
    fn status_queue_irq(&self) -> IrqSpec;
    /// Bytes per LibOS region-array entry, and the "RMARGS" region id.
    fn libos_region_layout(&self) -> LibosRegionLayout;
}

pub trait Arch { /* … existing … */ fn gsp(&self) -> &dyn GspModel; }
```

`GspObservation` is the §3.2 table as a struct of `bool`/`Option` — the FSM's *abstract*
state, with no encoding in it. This keeps CLAUDE.md rule 1 intact: `kayfabe-gsp` names
`GspReg::Wpr2AddrHi`, never `0x1FA828`.

**[inferred] I7:** that this seam is sufficient. It is derived from the C's *complete* set of
GSP-state-dependent register behaviours (`C:1348-1474`, an exhaustively readable function)
plus ogkm's `_TU102` HAL set — but only one generation has ever been implemented, so
"sufficient" is a prediction. §10.

**[open] O1 — where does the *rest* of the register model live?** PTIMER, the display fuse,
the PCI-config mirror, PRAMIN, the CPU interrupt tree. They are not GSP, they are not
`kayfabe-arch` vocabulary, and `kayfabe_vmm::Device` has no implementor yet
(`rs: ARCHITECTURE.md:38-43` says the implementor will be `kayfabe_rt::SharedDevice`).
This plan does **not** decide it; it flags that `kayfabe-gsp` must not become the default
dumping ground by accident, and proposes the boundary be drawn at *"does the GSP boot FSM's
state appear in the answer?"* — which is exactly the §3.2 table and nothing else.

---

## 4. The message-queue transport

### 4.1 Geometry

Shared-memory layout, per queue [src] `ogkm: src/nvidia/inc/libraries/msgq/msgq_priv.h:40-46`:

```text
backing store:
  +0                     msgqTxHeader   (32 B) — written by the TX side
  + rxHdrOff             msgqRxHeader   ( 4 B) — written by the RX side
  + entryOff             entries[msgCount] × msgSize
```

`msgqTxHeader` [src] `ogkm: msgq_priv.h:48-59` — offsets are 4-byte fields in order:
`version@0, size@4, msgSize@8, msgCount@12, writePtr@16, flags@20, rxHdrOff@24, entryOff@28`.
`msgqRxHeader = { readPtr }` [src] `:61-65`.

Derived by `msgqTxCreate` [src] `ogkm: msgq.c:234-250`:

```
rxHdrOff = ALIGN_UP(sizeof(msgqTxHeader)=32, 1 << hdrAlign)
entryOff = ALIGN_UP(rxHdrOff + sizeof(msgqRxHeader)=4, 1 << entryAlign)
msgCount = (size - entryOff) / msgSize
```

with `hdrAlign = 4`, `entryAlign = RM_PAGE_SHIFT` for the GSP queues
[src] `ogkm: message_queue_cpu.c:90-91`. On a 4 KiB page that is `rxHdrOff = 32 (0x20)`,
`entryOff = 4096`, `msgCount = (0x40000 - 0x1000)/0x1000 = 63` — which is where the C's
hard-coded `0x20` (`C:3358`) and the observed `msgCount = 63` (`C:2461`) come from. **Under
GSP-P1 none of these is a constant in the port**; they are read out of the header.

**Where the shared region is.** `MESSAGE_QUEUE_INIT_ARGUMENTS` inside the LibOS `RMARGS`
region: `{ u64 sharedMemPhysAddr@0; u32 pageTableEntryCount@8; NvLength cmdQueueOffset@16;
NvLength statQueueOffset@24 }` [src] `C: nvkvm_gpu_emul.c:3411-3425`, cross-checked against
`C-repo: docs/design/mode2_m3_gsp_rpc.md` "UPDATE 3", which records a verified live read:
`sharedMemPA = 0x139c00000, pteCount = 129, cmdQOff = 0x1000, statQOff = 0x41000`.

★★ **The region is NOT contiguous, and the C's linear addressing is a latent bug.**
This was an open question in an earlier draft; it is now **[src]**-settled:

- The backing memory is allocated `NV_MEMORY_NONCONTIGUOUS`, `ADDR_SYSMEM`
  [src] `ogkm: message_queue_cpu.c:254-256`.
- The **first pages of the region are a page table describing the region itself** — an array
  of `RmPhysAddr` (u64), one per 4 KiB page, `pageTableEntryCount` entries, filled by
  `memdescGetPhysAddrs(..., RM_PAGE_SIZE, pageTableEntryCount, pPageTbl)`
  [src] `ogkm: message_queue_cpu.c:297-329`.
- `sharedMemPhysAddr = pPageTbl[0]` [src] `:329` — i.e. **the table is self-describing: entry
  0 is the page the table itself starts on.**
- `cmdQueueOffset = pageTableSize` and `statQueueOffset = cmdQueueOffset + commandQueueSize`
  [src] `ogkm: kernel_gsp.c:5483-5484` — **byte offsets into the region**, not addresses.

Numerically for the defaults: `numPtes = 128 + ceil(128·8 / 4096) = 129`,
`pageTableSize = 4096`, so `cmdQueueOffset = 0x1000` and `statQueueOffset = 0x41000` — exactly
the values the C observed live (`C-repo: docs/design/mode2_m3_gsp_rpc.md` "UPDATE 3").

The C ignores the table and computes `sharedMemPhysAddr + offset` (`C:3437`, `C:2387`,
`C:1610`). That is correct **only while the guest's 512 KiB sysmem allocation happens to be
physically contiguous**, which it evidently was on the bench. ⇒ **the port must walk the
table.** `MsgqGeometry` therefore holds a *page-descriptor*, not a base address, and every
element access resolves through it. §11-O2 is now only "how often does the guest allocation
actually fragment", which is a robustness question, not a correctness one.

### 4.2 The tx-header handshake, and its acceptance predicate

The guest is the TX side of the command queue and the RX side of the status queue. It writes
its own cmd-queue tx header in `msgqTxCreate` during `kgspConstructEngine`, and then, at boot
step B9, spins in `msgqRxLink` until **we** have written a valid status-queue tx header.

`msgqRxLink`'s acceptance predicate, in order [src] `ogkm: msgq.c:330-405`:

| # | check | return | line |
|---|---|---|---|
| 1 | not already linked | `-1` | `:335-338` |
| 2 | `msgSize >= MSGQ_MSG_SIZE_MIN` | `-2` | `:340-343` |
| 3 | `msgSize <= size` | `-3` | `:345-348` |
| 4 | backing store non-null | `-5` | `:350-353` |
| 5 | `size >= rx.entryOff + msgSize` | `-6` | `:378-381` |
| 6 | **`rx.size == size`** | **`-7`** | `:383-386` |
| 7 | `rx.msgSize == msgSize` | `-8` | `:387-390` |
| 8 | `rx.version == MSGQ_VERSION (0)` | `-9` | `:391-394` |
| 9 | `rx.rxHdrOff >= sizeof(msgqTxHeader)` **and** `rx.entryOff >= tx.rxHdrOff + sizeof(msgqRxHeader)` **and** `rx.msgCount == (size - rx.entryOff)/msgSize` | `-10` | `:396-402` |

★ Check 9 compares `rx.entryOff` against **`tx.rxHdrOff`** — the *command* queue's value, not
our own. That is why the C's "copy the guest's cmd-queue header verbatim, set `writePtr = 0`"
(`C:3437-3444`) is not a shortcut but the most robust possible strategy: it satisfies checks
6–9 by construction for any parameters the guest chose, **and** it carries the guest's `flags`
across, which is load-bearing (§1.1 item 11 — `flags` must have `MSGQ_FLAGS_SWAP_RX` set or the
pointer polarity flips and both sides deadlock with no error).

★ **A torn header read is safe.** Check 9's own comment says it exists *"to make sure the
header arrived intact"*, and `msgqRxLink` is retried in a loop — so a partially-written header
fails and is re-read. But we must still publish with a store barrier before the guest can act
on it, because check 6–8 could pass on a torn header whose `entryOff` is stale.

For reference, a guest with default parameters publishes
`version=0, size=0x40000, msgSize=0x1000, msgCount=63, writePtr=0, flags=1, rxHdrOff=32,
entryOff=0x1000` — independently reproduced by nouveau's `r535_gsp_shared_init`
[src] `nv: r535/gsp.c:1164-1181`, which is the strongest available confirmation that this
reading of `msgqTxCreate` is right.

On success `msgqRxLink` **zeroes `pReadOutgoing`** and sets `rxReadPtr = 0`
[src] `ogkm: msgq.c:426-441`. With `MSGQ_FLAGS_SWAP_RX` agreed by both sides
(`msgq.c:411-424`), `pReadOutgoing = &pOurRxHdr->readPtr` — the **command queue's** rx header.
So:

> **The guest publishes its consumption of the STATUS queue at `cmdQueueBase + rxHdrOff`,
> and we must publish our consumption of the COMMAND queue at `statQueueBase + rxHdrOff`.**

[src] `ogkm: msgq.c:411-424` + `msgq.c:704-745` (`msgqRxMarkConsumed`), and the C does exactly
this at `C:3352-3358` with a comment recording that writing it to `cmd_base + 0x20` instead
caused *"buffer is full"* once ~63 command elements had accumulated.

★ **Re-link resets *position*, never *seqNum*.** `msgqRxLink` sets `rxReadPtr = 0`; nothing
anywhere in the GSP tree assigns `rxSeqNum` except `++` (`ogkm: message_queue_cpu.c:836`) and
its zero-initialisation in `GspMsgQueueInit`. The C discovered this the hard way and recorded
it at `C:3459-3483`. ⇒ **on a rebind we reset `writePtr` to 0 and preserve `seqNum`** — unless
the guest's `MESSAGE_QUEUE_INFO` was itself destroyed, which happens only in `kgspDestruct`
(module unload). §11-O3.

### 4.3 The element, and the version cliff

**610.43.02** [src] `ogkm: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:52-67`:

```c
typedef struct GSP_MSG_QUEUE_ELEMENT {
    NvU32 mctpHeader;   // @0   MCTP transport header
    NvU32 nvdmHeader;   // @4   NVDM over MCTP
    NvU32 checkSum;     // @8
    NvU32 seqNum;       // @12
    NvU8  payload[];    // @16  = [GSP_MSG_QUEUE_ENCRYPTION_TAG if CC] ++ rpc_message_header_v ++ data
} GSP_MSG_QUEUE_ELEMENT;
```

`queueElementHdrSize = offsetof(payload) = 16` (+40 if CC) [src] `ogkm: message_queue_cpu.c:82-86`.

**580.159.04**, as transcribed by the C's own design doc from the 580 tree
[src] `C-repo: docs/design/mode2_m3_gsp_rpc.md` "The message element", and as implemented at
`C: nvkvm_gpu_emul.c:1561-1620`:

```c
authTagBuffer[16] @0 ; aadBuffer[16] @16 ; checkSum @32 ; seqNum @36 ;
elemCount @40 ; rpc_message_header_v @48
```

`queueElementHdrSize = 48`.

★ **nouveau independently confirms the 48-byte form for both r535 and r570** — byte-identical
field list, including `elemCount`, and `msg->elem_count = DIV_ROUND_UP(len, 0x1000)` on send
[src] `nv: r535/nvrm/gsp.h:808-816`, `nv: r535/rpc.c:93-102, 370`; the r570 tree reuses the
same `rpc.c`. So the break lands in the interval **(570, 610]** and the C — whose 580
transcription matches r535/r570 exactly — is right for its era.

**These are different protocols.** §9 D1–D3. The port must therefore treat the element
header as an Axis-A layout with at least these fields per version:

```rust
// kayfabe-abi
pub struct ElementLayout {
    pub hdr_size_plain: usize,     // 48 on 580, 16 on 610
    pub hdr_size_cc: usize,        // + sizeof(encryption tag)
    pub checksum_off: usize,       // 32 on 580, 8 on 610
    pub seqnum_off: usize,         // 36 on 580, 12 on 610
    pub elem_count_off: Option<usize>,  // Some(40) on 580, None on 610
    pub transport: TransportHdr,   // None | Mctp { version: u32, vendor: u32 }
}
```

### 4.4 The seqNum discipline, exactly

Three counters that are routinely confused. Naming them apart is half the work:

| name | who owns it | wraps? | ogkm |
|---|---|---|---|
| **`writePtr` / `readPtr`** — ring **positions**, in *elements* | producer / consumer | **yes**, modulo `msgCount` | `msgq.c:521-527, 555-560` |
| **element `seqNum`** — per **message** | the sender, `txSeqNum++` | **no** — a free-running `NvU32` | `message_queue_cpu.c:514, 620` |
| **`rpc_message_header.sequence`** — the RPC **transaction id** | whoever originated the request | n/a | `kernel_gsp.c:1824-1828` |

The guest's receive validates all three in this order [src] `ogkm: message_queue_cpu.c:660-786`:

1. read element 0; derive `nElements = ceil((hdrSize + rpc.length) / elementSizeMin)` (`:698-705`);
2. read the remaining `nElements - 1` **contiguous** elements (`:673-684`);
3. checksum over `hdrSize + rpc.length` must fold to 0 (`:724-734`);
4. **610 only:** `MCTP_HEADER_VERSION == 1` and NVDM vendor id == NV (`:737-758`);
5. `element.seqNum == pMQI->rxSeqNum` (`:762`);
6. finally `rxSeqNum++` and `msgqRxMarkConsumed(nElements)` (`:836-838`).

Then the RPC layer matches `(function, sequence)` against what it is polling for
(`kernel_gsp.c:1824-1828`); anything else is dispatched as an event, and an unrecognised
event is *logged and ignored* (`kernel_gsp.c:1587-1599`) — except during the bootup window,
where a non-allowlisted event is `NV_ASSERT(0)` (`kernel_gsp.c:1419-1440`).

★ Two things the guest **does not** check on receive, worth knowing precisely because it is
tempting to rely on them: `header_version` and `signature`. `NV_VGPU_MSG_SIGNATURE_VALID`
appears exactly once in the whole tree, in the *send* path
[src] `ogkm: src/nvidia/src/kernel/rmapi/rpc_common.c:154-184`. We should still emit
`0x03000000` / `0x43505256` (forward-compat, and the C already does at `C:1584-1585`), but no
test may assert that the guest rejects a wrong one — that would be asserting a behaviour the
driver does not have.

★ And one the transport **accepts but shouldn't be emitted**: `rpc.length == 0` passes the
length sanity check (`msgLen == hdrSize`, `ogkm: message_queue_cpu.c:826-833`) and
`bytesToElements(hdrSize, 4096) == 1`, so a zero-length message silently consumes one element
and then produces garbage upstream. `MsgLen::new` (§7 G3) must therefore have a **lower** bound
of `sizeof(rpc_message_header_v)`, not zero.

#### ★ What "over-posting desyncs the RPC path" means, precisely

Two distinct failures, and the crate doc's `⚠8` conflates them:

**(a) Ring overwrite → unrecoverable seqNum gap.** If we advance `writePtr` past elements the
guest has not consumed, we overwrite them. The guest then reads an element whose `seqNum` is
**greater** than its `rxSeqNum`. The recovery branch at `message_queue_cpu.c:768-782` handles
**only** `seqNum < rxSeqNum` (an old package: skip it and converge). For `seqNum > rxSeqNum`
there is no recovery — three retries, then `NV_ERR_INVALID_DATA`, and `rxSeqNum++` still
happens at `:836`, so the streams stay one apart forever. **This is the desync.**

**(b) Full-ring aliasing.** `msgqRxGetReadAvailable` returns `writePtr + msgCount - rxReadPtr`
reduced modulo `msgCount` (`msgq.c:653-665`), so *exactly* `msgCount` outstanding elements
reads as **0 available** — full is indistinguishable from empty. The producer-side invariant
that prevents this is `msgqTxGetFreeSpace`'s `-1`: `free = readPtr + msgCount - writePtr - 1`
(`msgq.c:488-497`). **At most `msgCount - 1` elements may be outstanding.**

#### ★ The C has no flow control at all, and this is the port's single biggest transport win

`nvkvm_m3_post_status` (`C:1561-1620`) **never reads the guest's status-queue `readPtr`.**
Grep the file's complete guest-RAM read set — there are exactly eight `pci_dma_read` sites
(`C:2387, 2406, 3390, 3417, 3437, 4368, 4627, 5238`) and none of them is
`cmdQueueBase + rxHdrOff`. So the C cannot compute free space and posts unconditionally.

Its mitigation is a proxy: `nvkvm_gsp_deliver_events` refuses to post an event batch while
`gsp_swgen0_pending` is set (`C:1697-1703`), whose comment says exactly why —
*"Posting an event batch on every doorbell (hundreds of times) overflows the ring before the
guest drains it → the guest's rpcRecvPoll sees a seqNum gap ('Bad sequence number') and the
whole RPC path breaks."* That is failure (a), diagnosed correctly and worked around
indirectly. It is also the mechanism behind `⚠8`'s *"one SWGEN0 batch outstanding"*
serialisation that `mode2_rust_rewrite_architecture.md` §4.3.2 wants removed.

**Rule GSP-P2 (normative).** `kayfabe-gsp` implements real flow control:

```rust
fn free_elements(&self, gm: &mut dyn Vmm) -> Result<u32, GspFault> {
    let rp = read_u32(gm, self.geom.cmd_base + self.geom.rx_hdr_off)?;   // guest's status readPtr
    if rp >= self.geom.msg_count.get() { return Err(GspFault::PeerReadPtrOutOfRange(rp)); }
    Ok((rp + self.geom.msg_count.get() - self.write_ptr - 1) % self.geom.msg_count.get())
}
fn post(&mut self, msg: &Message, ..) -> Result<(), GspFault> {
    if msg.elements() > self.free_elements(..)? { return Err(GspFault::QueueFull); }
    /* … */
}
```

This mirrors `msgqTxGetFreeSpace` line for line, including its **`txReadPtr >= msgCount → 0`**
refusal (`msgq.c:483-486`) — which is also a hostile-input guard, since `readPtr` is
guest-writable. `QueueFull` is then a *retryable* condition that `kayfabe-completion` handles
as back-pressure, replacing the one-batch-outstanding serialisation with per-`Proc` queuing
above a correctly-flow-controlled single transport.

### 4.5 Checksum

64-bit XOR fold, reduced to 32 by `hi ^ lo` [src] `ogkm: message_queue_priv.h:197-209`.
The routine reads in `NvU64` steps `while (p < pEnd)`, i.e. it **reads up to the next 8-byte
boundary past `uLen`**, which its own comment licenses (*"assumes that the data is padded out
with zeros to the next 8-byte alignment"*). The sender zero-pads to 8 before folding
(`message_queue_cpu.c:498-500`). The C's `(len + 7) & ~7` (`C:1605`) is exactly equivalent.
Coverage is `hdrSize + rpc.length` in the plain case and the **whole element run**
(`nElements * elementSizeMin`) when CC is on (`message_queue_cpu.c:519-543` vs `:713-730`).

---

## 5. Staged build order

Nine stages. **Six need no GPU.** Each names what it *cannot* prove.

| stage | needs GPU? | what it builds | proves | cannot prove |
|---|---|---|---|---|
| **S0 — ABI extension** | **no** | `kayfabe-abi`: `msgqTxHeader`/`msgqRxHeader`, `LibosMemoryRegionInitArgument`, **all three shapes of** `MESSAGE_QUEUE_INIT_ARGUMENTS` (D8), `ElementLayout` per version (D1), the remaining `NV_VGPU_MSG_*` ids the boot path uses (1, 51, 65, 70, 71, 72, 73, 76, 0x1001, 0x1003). ★ **First task: vendor a 580.159.04 tree** (§11-O4/O8) | generator-vs-rustc layout agreement; version key refuses below floor; the 610-vs-r535 element split is expressed as data | that the **580** shapes are right, until the 580 tree is vendored |
| **S1 — ring algebra** | **no** | `MsgqGeometry` (decode + the 9-check predicate, **carrying the page-table descriptor** per §4.1), `MsgCount(NonZeroU32)`, `Slot`, free/available algebra | the transport maths, against ogkm line-for-line; proptest: slot always `< msgCount`; `free + outstanding + 1 == msgCount`; every `msgqRxLink` rejection code `-1..-10` reproducible; **a fragmented (non-contiguous) region resolves correctly** | that a real guest agrees (S6) |
| **S2 — element codec** | **no** | encode/decode, checksum, multi-element split/join, MCTP/NVDM on 610, CC-off header arithmetic | round-trip identity; guest-side checksum of our output folds to 0; a 1-bit flip anywhere is detected | nothing about ordering |
| **S3 — boot FSM** | **no** | `BootPhase`/`QueueState`, transitions E1–E11, `device_reset`, `observe()` | exhaustive transition coverage; **the §3.4 reset chain as a named regression** (`cb23_*`, per `c_bug_regression_matrix.md` row 23) | which real register writes drive which transition (needs S5/S6) |
| **S4 — RPC dispatch** | **no** | envelope decode → `RmEvent` / control intent; response encode; the *async* set (72, 73) that expects no reply; the **first two synchronous RPCs after `INIT_DONE`** — `SET_GUEST_SYSTEM_INFO` (1) and `GET_GSP_STATIC_INFO` (51), `ogkm: kernel_gsp.c:5153-5165`; the mandatory fn-47 reply (G8) | the ogkm-sourced async set (`kernel_gsp.c:4694-4706`); reply `(function, sequence)` echo; a bounded `MsgLen` | the long tail of control semantics (that is `kayfabe-fwd`) |
| **S5 — trace replay harness** | **no** (needs recorded traces) | §6 | ★ **the oracle gate.** Byte-equivalence with the C on everything the guest observes, minus an enumerated must-differ table | anything the recorded traces did not exercise |
| **S6 — hybrid bring-up** | **YES** | Rust GSP behind the C QEMU device; boot a live guest to `GSP_INIT_DONE` | the FSM against the real driver; the S1 geometry against real parameters | multi-process (the C cannot: `../reference/mode2_bench_lifecycle.md` §1) |
| **S7 — lifecycle conformance** | **YES** | `rmmod`/`insmod` and two sequential CUDA processes | **the C bug is fixed**: `-7` must not recur, and the second process must reach `cuInit` | the seqNum-flavour question (§11-O3) only becomes reachable *here* |
| **S8 — second axis** | **YES** for the chip half | a second driver version (S0 tables) and/or a second generation behind `GspModel` | that neither axis is a retrofit | — |

**Ordering rationale.** S0→S5 is a strict dependency chain and is the whole of the
"needs NO real GPU" claim in `C-repo: mode2_rust_rewrite_architecture.md` §4.5 step 2. S6 is
the strangler seam. **S5 must be green before S6 consumes a bench slot** — the bench is
serialized and costs a fresh boot per run (`CLAUDE.md`, L12), so every defect found in S0–S5
is one that does not cost a boot.

**CI, from day one** (all from `rs: .github/workflows/ci.yml`, per the survey):
`kayfabe-gsp` is already inside both vocabulary gates (hexagonal-boundary and VMM-vocabulary),
**including comments**; the generation-name grep gate applies; `#![forbid(unsafe_code)]` is
workspace-wide. ★ **The mutation job's `-f` scope does not include the crate** — S3 must add
`-f 'crates/kayfabe-gsp/**/*.rs'` or the 91% gate silently ignores every line of it.
Per `testing_doctrine.md` §3.1, "done" for each stage means wired into
`tests/tests/l1_mean.rs`, not only an isolated file.

---

## 6. The trace format and the replay harness

### 6.1 What to capture — **both**, plus the guest-RAM reads

BAR0 accesses alone are insufficient: the entire queue protocol happens in guest RAM, and a
replay that cannot answer a `pci_dma_read` is not hermetic. RPC elements alone are
insufficient: the boot FSM is driven by MMIO. So the record is a single interleaved stream:

```rust
enum TraceEvent {
    MmioWrite { bar: u8, off: u64, size: u8, val: u64 },
    MmioRead  { bar: u8, off: u64, size: u8, val: u64 },   // val = what the C SERVED
    GuestRead { gpa: u64, bytes: Vec<u8> },                // what the C's DMA read RETURNED
    GuestWrite{ gpa: u64, bytes: Vec<u8> },                // what the C WROTE  ← assertion target
    IrqRaise  { spec: IrqSpec },                           //                    ← assertion target
    Clock     { ns: u64 },                                 // PTIMER determinism
}
struct Record { seq: u64, ev: TraceEvent }
```

**Ordering guarantee.** A single `u64` counter incremented on every record, in the device's
MMIO/DMA path. This is a **total order** because the C device is single-threaded — the file
asserts this itself twice, at `C:1573` and `C:1653` (*"device emu is single-threaded"*, as the
justification for `static` scratch buffers). [inferred] I2: that no other QEMU thread reaches
these paths. §10.

**Interleaving is the payload, not metadata.** The classic replay failure is recording reads
and writes in separate streams and losing "the C wrote the status tx header *before* it posted
INIT_DONE". One stream, one counter.

### 6.2 How to record without perturbing the C

The capture surface is **small and already funnelled**:

- **All** guest-RAM writes go through one wrapper, `nvkvm_dmaw` (`C:827-837`) — 6 call sites.
- Guest-RAM reads are 8 `pci_dma_read` sites (`C:2387, 2406, 3390, 3417, 3437, 4368, 4627, 5238`).
- MMIO enters at `nvkvm_bar0_read`/`nvkvm_bar0_write` (`C:1496, 3504`) plus the rom-device
  thunk `nvkvm_gsp_falcon_write` (`C:4328-4331`).
- IRQ raises: `msix_notify`/`pci_set_irq` inside `nvkvm_gsp_raise_swgen0` (`C:1678-1682`).

⇒ ~18 call sites, one append-only binary sink, **no control-flow change**. Two hard rules:

1. **Compile-time gated** (`#ifdef NVKVM_TRACE_CAPTURE`), default off, so the perf baseline is
   untouched — L12's rom-device read-overlay trick exists precisely because vmexit cost is
   load-bearing, and `qemu_log`-based tracing already costs enough that the C disables MMIO
   tracing for PTIMER and PRAMIN (`C:1519-1523`).
2. **Not the existing `qemu_log` path.** It is lossy (no DMA payloads) and its formatting cost
   is inside the traced path. Records go to a raw fd.

★ **Ownership note:** this patch is to `nvkvm_gpu_emul.c`, which this plan does not own. It is
~60 lines and mechanical; it should be raised as its own task, and S5 is blocked on it.

**One un-recorded input remains: the rom-device overlay.** BAR0 page `0x110000` is served from
a RAM buffer, so the guest's *reads* of `IRQSTAT`/`MAILBOX0`/`CPUCTL`/`DMATRFCMD` **never
trap** (`C:1462-1477`, `C:4315-4326`) and therefore never appear in the trace. The replay must
compare against the *buffer contents* instead: record a `MmioRead`-equivalent snapshot of the
overlay page whenever `nvkvm_gsp_falcon_sync` runs. Without this, the harness silently
verifies nothing about the most-read registers in the system. [src] `C:1462-1477`.

### 6.3 What "identical" means — and the part that would enshrine the hole

★ **The differential is over a decoded projection, never over raw bytes.**

```rust
enum Observation {                       // what a guest could possibly notice
    TxHeaderPublished { queue: QueueId, hdr: MsgqTxHeader },
    ElementPosted     { slot: Slot, seq_num: u32, env: RpcEnvelope, payload: Blake3 },
    ReadPtrAcked      { queue: QueueId, value: u32 },
    RegisterServed    { reg: GspReg, value: u64 },
    IrqRaised         { spec: IrqSpec },
    Refused           { fault: GspFault },     // ← Rust-only; the C has no such concept
}
```

Byte-diffing would enshrine three things we intend not to reproduce: the C's zero padding, its
`rpc.length = 36` for a 32-byte header (`C:1586`; already caught and recorded at
`rs: crates/kayfabe-abi/src/view.rs:331-339`), and its uninitialised element tails. Decoded
diffing lets a divergence be expressed as a **field-level exception with a reason**.

Every assertion falls in exactly one of three classes, and the class is **declared in a table,
not inferred**:

**(1) MUST-MATCH** — anything ogkm shows the guest reading or validating: tx-header fields,
element checksum, `seqNum`, envelope `(function, sequence, rpc_result)`, the readPtr ack, every
row of the §3.2 observable table, every IRQ raise. Default class.

**(2) MUST-DIFFER** — an enumerated ledger. An entry is only admissible with **all four**:

```rust
struct Divergence {
    id: &'static str,             // e.g. "GSP-D1"
    c_site: &'static str,         // C: file:line
    c_behaviour: &'static str,
    our_behaviour: &'static str,
    guest_visible_consequence: &'static str,   // ← if "none", it is class (3), not (2)
    independent_oracle: &'static str,          // ogkm file:line, or a measured host trace.
                                               //   "it is cleaner" is NOT admissible.
}
```

Opening ledger (the ones this plan has already established):

| id | C site | C does | we do | oracle |
|---|---|---|---|---|
| GSP-D1 | `C:1586` | `rpc.length = 36` for a bare header | 32 | `ogkm: g_rpc-message-header.h:41-52`; already asserted at `rs: view.rs:331-339` |
| GSP-D2 | `C:1561-1620` | posts without reading the guest readPtr | flow-controlled, `QueueFull` | `ogkm: msgq.c:488-497, 534-545` |
| GSP-D3 | `C:1615` | `% s->q_msgcount` with no zero guard | `NonZeroU32`, unrepresentable | `../reference/mode2_bench_lifecycle.md` §4 [measured] |
| GSP-D4 | `C:2380-2410` | parses guest RAM on a doorbell while `q_ready` is stale | E8 refusal, **zero reads** | `../reference/mode2_bench_lifecycle.md` §4 [measured]; 508 log lines/bring-up |
| GSP-D5 | `C:4255-4283` | misclassifies the teardown STARTCPU as a re-acquire | E2 | `ogkm: msgq.c:383-386` (`-7` ⇔ no tx header) + [measured] §3.4 |
| GSP-D6 | `C:2406` | reads one 4096-byte element and **skips** continuation elements | reads `nElements`, bounded by `queueElementSizeMax` | `ogkm: message_queue_cpu.c:673-705` |
| GSP-D7 | `C:1697-1703` | one SWGEN0 batch outstanding, globally | per-`Proc` queuing above a flow-controlled transport | `C-repo: mode2_rust_rewrite_architecture.md` §4.3.2 (`⚠8`) |
| GSP-D8 | `C:3437, 2387, 1610` | addresses the shared region as `sharedMemPhysAddr + offset` (assumes contiguity) | resolves through the region's own page table | `ogkm: message_queue_cpu.c:254-256, 297-329` |
| GSP-D9 | `C:3388-3407` | scans at most 16 LibOS regions | bounds by the descriptor's declared size, capped at `LIBOS_MEMORY_REGION_INIT_ARGUMENTS_MAX` | `ogkm: libos_init_args.h:31` |

**(3) UNCONSTRAINED** — bytes no guest reads. Excluded **by construction**: `Observation`
only carries decoded fields, so padding and log text are not comparable in the first place.
No allowlist exists to rot.

#### ★ The sharpest expression of must-differ: the negative trace

A bug-for-bug differential would make GSP-D4 a *requirement*. So the harness carries a
**negative trace class** whose passing condition is that we **differ**:

> Replay the recorded stale-state bring-up — the one that produced 508 lines of
> `cmd fn=1959520414 seq=4055862830 -> echo NV_OK` and `reqPsize=4158089418`
> (`../reference/mode2_bench_lifecycle.md` §4) — and assert the Rust core emits
> **exactly one** `Observation::Refused(GspFault::QueueNotBound)` and **zero**
> `ElementPosted`, **zero** `GuestRead`.

The C's output is the *anti*-expectation. This satisfies `testing_doctrine.md` §2's
"assert the exact thing, never an absence": the assertion is a specific fault variant plus a
zero **count** of a named event, with the non-vacuity arm being the *positive* replay of the
same trace prefix, where the counts are non-zero.

Symmetrically, every MUST-DIFFER row needs a **bite check** (`testing_doctrine.md` §1c):
reverting our behaviour to the C's must turn the test red. For GSP-D3 that is mechanical —
swap `NonZeroU32` for `u32` and the code no longer compiles, which is the strongest form.

---

## 7. Security posture — the class made unrepresentable

The C's two guest-reachable defects [measured] `../reference/mode2_bench_lifecycle.md` §4:

- **arbitrary guest RAM parsed as GSP RPC and answered `NV_OK`**, reachable by a guest
  **reloading its own driver**;
- **unguarded `% s->q_msgcount`** at `C:1615` → SIGFPE, a guest-reachable QEMU crash.

Both are *the same root cause*: the queue binding is a set of loose fields
(`q_shmem, q_cmd_base, q_stat_base, q_msgsize, q_msgcount, q_*_entryoff`, `C:188-196`) guarded
by a separate boolean `q_ready`, and the four reset sites disagree about which of them to
clear. Fixing them is not the goal; **making the shape impossible is.**

**G1 — the binding is a value, and the GPA lives only inside it.**

```rust
pub enum QueueState { Unbound, Bound(QueueBinding) }

pub struct QueueBinding {              // no Default, no Clone, private fields
    geom: MsgqGeometry,                // validated by the §4.2 predicate at construction
    cmd:  RingCursor,
    stat: RingCursor,
}
impl QueueBinding {
    fn bind(gm: &mut dyn Vmm, args: MessageQueueInitArgs, layout: ElementLayout)
        -> Result<Self, GspFault>;     // the ONLY constructor
}
```

Service takes `&QueueBinding`. `device_reset` sets `QueueState::Unbound`, **dropping the
binding by value**. There is no field to leave stale, because the fields do not exist outside
the variant. ⇒ "parse guest RAM without a live binding" is not a bug to avoid; it does not
type-check. Same discipline as `GpaSpace::release(arena: GpaArena)` taking by value
(`rs: crates/kayfabe-core/src/gpa.rs`).

**G2 — the divisor is a type.** `MsgCount(NonZeroU32)`, validated at geometry decode. Every
wrap goes through `MsgCount::slot(&self, n: u32) -> Slot`. Division by zero is not a check we
pass; it is a value we cannot construct. Kills GSP-D3 at compile time.

**G3 — extent comes from geometry, never from a length field.** An element copy is exactly
`geom.element_size` bytes. Total message length is bounded by ogkm's own rule
(`msgLen >= hdrSize && msgLen <= queueElementSizeMax`, `ogkm: message_queue_cpu.c:491-495`
and the mirror at `:827-833`), expressed as a fallible constructor
`MsgLen::new(rpc_length, &geom) -> Result<MsgLen, GspFault>`. `kayfabe-abi` already has the
envelope half: `rpc_payload_len` errors with `AbiError::RpcLength { declared, available }`
(`rs: crates/kayfabe-abi/src/view.rs:342-357`).

**G4 — copy once, decide on the copy.** The parser is `fn(&ElementBuf) -> …` and is never
handed a `&mut dyn Vmm`. It **cannot** re-read guest memory, so the TOCTOU class that
`guest_memory_lock.md` §0 defines is out of reach by signature. This is also the mechanism
behind §8's GL11 answer.

**G5 — every guest-supplied scalar is range-checked at decode, with a named fault.** Not
`is_err()` (`testing_doctrine.md` §2): `GspFault::{ PeerReadPtrOutOfRange(u32),
PeerWritePtrOutOfRange(u32), GeometryRejected(RxLinkCode), MsgLenOutOfRange{..},
QueueNotBound, QueueFull, ChecksumMismatch{..}, SeqNumGap{expected,got},
TransportHeaderInvalid{..} }`. Note `PeerWritePtrOutOfRange` mirrors ogkm's own guard
(`msgq.c:655-658`, `rx.writePtr >= rx.msgCount → 0`) — the guest's producer index is hostile
input to us exactly as ours is to it.

**G6 — reply size is clamped to the request.** The C found this the expensive way: an
over-size control reply overran libcuda's stack buffer and zeroed a saved `rbp`
(`C:3237-3252`, the M9 clamp). Expressed as `ReplyParams::clamped_to(request_psize)`, so the
unclamped reply has no constructor.

**G7 — no event may be posted during the bootup window** except the eight in ogkm's allowlist
(`ogkm: kernel_gsp.c:1419-1440`); anything else is `NV_ASSERT(0)` in the guest. `POST_EVENT`
(0x1003) is **not** on that list. Enforced by making the post entry point require
`phase == Running`, which is a state the FSM only reaches after `GSP_INIT_DONE`.

**G8 — liveness obligations are part of the contract, not politeness.** Two guest polls hang
the *guest* indefinitely if we stop answering, and both are on the teardown path — i.e. exactly
where a fault-and-stop posture is most tempting:

- **fn-47 must be replied to.** It is synchronous (`ogkm: rpc.c:9146-9170`), so an
  unanswered fn-47 blocks `rmmod` for the full RPC timeout.
- **`MAILBOX0` must then report suspended** (`ogkm: kernel_gsp_tu102.c:352-357`).

⇒ `GspFault` handling must distinguish *refuse this message* from *stop serving the device*.
A refusal is per-message and the FSM keeps answering polls; there is no fault in this crate
that stops the observable surface, because that would convert a contained refusal into a guest
hang — the F1/F2 shape in `core_security_threat_model.md`.

**G9 — no unsolicited event may be posted while the guest is inside a synchronous poll**, or
we can drive it into the recursive-poll assert (`ogkm: kernel_gsp.c:2893`). Since we cannot
observe the guest's poll state directly, the conservative rule is the one the FSM already
gives us: events only in `phase == Running`, and only for functions on the event allowlist.
**[inferred] I9** — that "`Running` implies not-inside-a-boot-poll" is sufficient. §10.

**Threat-model fit.** These are all A1 (hostile guest userspace) / A3 (compromised guest
kernel) reaches under `core_security_threat_model.md`; the outcome for each is a **contained
loud refusal**, which is I4's requirement. §2.1's *"a shared table is evidence, never
authority"* is what G1+G4 implement for the msgq specifically.

---

## 8. ★ The GL11 question — answered

`gl11_region_arguments.md` §2.2 asks the GSP milestone for a yes/no: does the boot path
contain a command where **all three** hold — a non-undoable host verb issued mid-parse, **and**
a later step that must re-read guest memory, **and** that read being un-hoistable into the
first copy?

### **NO. Do not register the GSP command queue as `LockPath`.**

The evidence is a complete enumeration, not a sample. `nvkvm_gpu_emul.c` reads guest RAM at
exactly **eight** sites; here is every one, with what it is:

| C line | what | in the command-service path? |
|---|---|---|
| `2387` | cmd-queue `writePtr` (4 B) | yes — the producer index. §2.1 item 1 explicitly permits this. |
| `2406` | **one 4096-byte element copy** | yes — the single copy. Extent is a constant, not a length field. |
| `3390` | LibOS region array entry | boot handshake only |
| `3417` | RMARGS / `MESSAGE_QUEUE_INIT_ARGUMENTS` | boot handshake only |
| `3437` | the guest's cmd-queue tx header | boot handshake only |
| `4368` | BAR1 aperture read | different region class (§1 row 1/2) |
| `4627` | page-table entry read | copy-once/commit-point class, excluded by GL2's rate rule |
| `5238` | `nvkvm_phys_rd32` — semaphore/instance-block | volatile/atomic class |

**Command path.** Two reads: an index, then one fixed-extent copy. Everything afterwards —
including `nvkvm_m2_shadow_fwd` (`C:2312`, `C:6466`), which is where real host RM ioctls are
issued — operates on `cmd`/`resp`, which are private buffers. **The second conjunct of §2.2
fails: no step re-reads guest memory after a host verb.**

**Boot handshake.** Three *dependent* reads (`3390` → `3417` → `3437`), which looks like the
shape §2.2 is worried about. It is not: no host verb is issued anywhere in
`nvkvm_m3_dump_bootargs`. The one host-verb-issuing call in that function,
`nvkvm_m2_reap_dead` (`C:2084`, called at `C:3474`), runs **after all three reads**. **The
first conjunct fails.**

**Three qualifications, so this is falsifiable rather than merely reassuring:**

1. **The Rust port will add a read the C does not perform.** GSP-D6: the C skips continuation
   elements (`C:3341-3350` advances `cmd_readptr` past them without reading them); we must
   read them. But that read's extent comes from the *first copy's* `rpc.length`, bounded by
   `queueElementSizeMax`, and it happens **before** any host verb. GL11 §2.1 item 4 permits
   this verbatim: *"a second, separately bounded copy whose length came from the first copy."*
   Still no.
2. **This is an argument about the boot path**, which is what §2.2 asks. It is **not** an
   argument about the steady-state RPC path once `kayfabe-fwd` issues non-undoable host verbs.
   S6 must re-check §2.2 against `kayfabe-fwd`, and this plan says so rather than implying
   coverage it does not have.
3. **G4 makes the answer structural, not incidental.** The parser's signature cannot reach
   guest memory. So the conclusion is not "we checked and there is no such command" but
   "such a command cannot be written in this crate."

**Consequence for `gl11_region_arguments.md` §4**, stated as that file asks: with §2 answering
NO, the region lock's only remaining candidate is §3 (instance blocks), whose likely outcome
is BAR2-trapped. **The lock plausibly has no members at all** — a capability kept for a case
not yet met, at zero cost unarmed. That should be *stated* in the mechanism decision, not
assumed away.

---

## 9. Where the C and ogkm disagree — findings, not resolutions

### D1 ★★ The queue element header is a different structure between 580 and 610

| | 580.159.04 (bench; the C implements it) | 610.43.02 (vendored ogkm) |
|---|---|---|
| @0 | `authTagBuffer[16]` (CC) | `mctpHeader` |
| @4 | ↑ | `nvdmHeader` |
| @8 | ↑ | `checkSum` |
| @12 | ↑ | `seqNum` |
| @16 | `aadBuffer[16]` (CC) | **payload begins** |
| @32 | `checkSum` | — |
| @36 | `seqNum` | — |
| @40 | **`elemCount`** | — |
| @48 | rpc header | — |
| header size | **48** | **16** (+40 if CC) |

[src] 580: `C-repo: docs/design/mode2_m3_gsp_rpc.md` "The message element", transcribed from a
580.159.04 tree, and implemented at `C: nvkvm_gpu_emul.c:1561-1620`.
[src] 610: `ogkm: message_queue_priv.h:52-67`, `message_queue_cpu.c:82-86`.
[src] **r535 + r570 (independent): the 48-byte form, byte-identical to the C's**, including
`elemCount` — `nv: r535/nvrm/gsp.h:808-816`, `nv: r535/rpc.c:93-102`, and r570 reuses the same
`rpc.c`.

Neither is wrong; they are different driver versions, and with nouveau as a third source the
break is bracketed to **(570, 610]**. **The finding is that the C hard-codes 48/40/32/36 at
~15 sites** (`C:1583-1602, 2406-2419, 2734-2735, 3341-3350`, and every `cmd + N` offset in
`service_cmdq`), so it is a 580-only implementation with no version key.
⇒ Axis A, §4.3's `ElementLayout`.

### D2 ★ 610 validates MCTP/NVDM transport headers; the C writes neither

`GspMsgQueueReceiveStatus` rejects an element whose `MCTP_HEADER_VERSION != 1` or whose NVDM
vendor id is not NV [src] `ogkm: message_queue_cpu.c:737-758`, and the sender fills them via
`mctpCreateTransportHeader(SOM=1, EOM=1, 0,0,0)` / `mctpCreateNvdmHeader(NVDM_TYPE_RM_RPC)`
[src] `:505-512`. The C never writes offsets 0–7 of a status element with anything but zero
(`C:1583`: `memset(el, 0, …)` then fields from +32 up).

⇒ **A port that ships only the C's encoding would be rejected on 610** with
`"MCTP protocol violation"`. Whether 580 has the check is **[open] O4** — no 580 tree is
vendored.

### D3 ★ `elemCount` is not how 610's guest counts elements

610 derives `nElements` from `hdrSize + rpc.length` [src] `ogkm: message_queue_cpu.c:698-705`;
there is no `elemCount` field. The C writes one at +40 (`C:1602`). On 610 that offset is
**inside the RPC header** (payload@16 + 24 = `rpc.sequence`), so replaying the C's encoder
against 610 would corrupt the transaction id. Corollary: the C's own multi-element split
(`C:1596-1613`) is already computing `nelems` the 610 way and *also* writing the 580 field —
so the algorithm is right and only the field placement is version-bound.

### D4 fn-47's "TWO distinct triggers" — the comment describes one RPC with several callers

`C:2452-2456` claims two distinct triggers (rmmod, and a GPU-idle release). ogkm shows **one**
RPC emission point in the non-vGPU path — `kernel_gsp.c:5231`, inside `kgspUnloadRm_IMPL` —
reached from `gpuStateUnload`/`gpuStateDestroy` (`gpu.c:3919`, `:4252`), PM suspend
(`gpu_suspend.c:120`), and a force-unload control (`subdevice_ctrl_gpu_kernel.c:689`). So
there are several *callers* of one unload, not two RPCs with different meanings. This is
**consistent** with the measurement that `rmmod` emits none (the state was already unloaded by
the process-exit path). ⇒ the design consequence in `../reference/mode2_bench_lifecycle.md` §2
stands: **there is no second RPC to disambiguate on**, and a `device_reset` armed only on
fn-47 never runs on a true driver restart.

### D5 ★★ "A re-acquire does not re-write the boot-args mailbox" contradicts ogkm

`C:4270-4277` asserts that a GSP re-acquire *"reuses the existing boot-args + GSP message
queue and does NOT re-write the boot-args mailbox"*, and the whole `was_suspended` re-dump
path at `C:4278-4281` exists to compensate.

ogkm says a **NORMAL** boot always programs the mailboxes: `kgspProgramLibosBootArgsAddr_HAL`
is called unconditionally inside `if (bootMode == KGSP_BOOT_MODE_NORMAL)`
[src] `ogkm: kernel_gsp_tu102.c:561-563`. A boot that skips it is `SR_RESUME` or `GC6_EXIT` —
and those **also** skip `kgspSendInitRpcs` (`:576`) and **`GspStatusQueueInit`** (`:607-611`),
because *"GSP-RM will restore queue state."*

⇒ Exactly one of these is true, and they demand opposite designs:

- **(a)** the observed re-acquire was a **non-NORMAL** boot ⇒ our FSM must *not* rebind or
  re-post `INIT_DONE` on it at all, and the C's re-dump is actively wrong (it re-links a queue
  the guest did not re-link);
- **(b)** it was NORMAL and the mailbox write *did* occur, but the C's `bootargs_dumped` latch
  (still `true`, never cleared on a mailbox write) **swallowed** it ⇒ the C's re-dump is a
  workaround for its own latch, and deleting the latch (§3.3) fixes it properly.

**This is the single most consequential open item in the plan.** §11-O5 names the experiment.
Note that under either answer the §3.3 latch deletion is correct — which is why S3 can proceed
before it is settled.

### D6 ★ "The mailbox path at `C:4301` is dead on GA106" — not established, and contradicted

The briefed premise cites the C's own 2026-06-03 note. That note
(`C-repo: docs/design/mode2_m3_gsp_rpc.md`, "UPDATE (2026-06-03): the stall is PRE-mailbox")
was **superseded the same day** by UPDATE 2 (the PTIMER fix, which unblocked the driver past
the stall it describes) and UPDATE 3, which records the mailbox capture **firing** and yielding
a verified live read: `sharedMemPA=0x139c00000 pteCount=129 cmdQOff=0x1000 statQOff=0x41000`.
Citing it as confirmation is citing a retracted observation.

Structurally, on the **first** boot of a QEMU lifetime the mailbox path is the *only* trigger
for the handshake — `was_suspended` is false, so `C:4278` cannot fire, and `q_shmem` is
demonstrably populated. What is defensible from the 2026-07-25 log is narrower and different:
**in the failing second driver life, no mailbox writes were observed**, which is exactly what
`_kgspBootGspRm` bailing at *"unexpected WPR2 already up"* (`ogkm: kernel_gsp.c:4805-4809`)
before reaching step B4 would produce.

⇒ **The correct statement is that the mailbox path is *shadowed by a stuck latch*, not dead**,
and that upgrades it from dead code to the load-bearing entry point the reset must restore.
The briefed premise is wrong as stated; the reset spec it was used to support is unaffected —
§3.4 does not depend on it.

### D8 ★★ `MESSAGE_QUEUE_INIT_ARGUMENTS` has **three** shapes, and the C parses one of them

| tree | fields |
|---|---|
| **r535** (`nv: r535/nvrm/gsp.h:578-585`) | `sharedMemPhysAddr, pageTableEntryCount, cmdQueueOffset, statQueueOffset, locklessCmdQueueOffset, locklessStatQueueOffset` — **6** |
| **r570** (`nv: r570/nvrm/gsp.h:497-502`) | the first **4** only — the lockless pair was *removed* |
| **ogkm 610** (`ogkm: gsp_init_args.h:32-45`) | the 4, **plus** `queueElementHdrSize, queueElementSizeMin, queueElementSizeMax, queueHeaderAlign, queueElementAlign` — **9** |

The C reads exactly 32 bytes and interprets them as the first four fields
(`C: nvkvm_gpu_emul.c:3411-3425`). That is correct for r570-shaped and 610-shaped structs, and
correct-by-luck for r535's (the extra fields are appended, so the prefix is stable).

Two consequences:

1. **`NvLength` is `size_t`, and the struct is not packed**, so there are 4 padding bytes after
   `pageTableEntryCount` (u32 at +8) before `cmdQueueOffset` (u64 at +16). The C's offsets
   (`+0, +8, +16, +24`) are right — but they are right because it hard-codes them, not because
   it modelled the padding. A generated layout must derive it.
2. ★ **On 610 the guest hands us the element header size**, which is the field the port would
   otherwise have to key on driver version (§4.3). That makes GSP-P1 stronger where the field
   exists and is why the port must treat the init-args struct as version-keyed with a
   *capability* check, not as a fixed 32-byte prefix.

### D9 The C scans 16 LibOS regions; the spec allows 4096

`LIBOS_MEMORY_REGION_INIT_ARGUMENTS_MAX = 4096`
[src] `ogkm: src/common/uproc/os/common/include/libos_init_args.h:31`; the entry is
`{ LibosAddress id8; LibosAddress pa; LibosAddress size; NvU8 kind; NvU8 loc; }` [src] `:49-56`
(→ 32 bytes with alignment, matching the C's `LIBOS_REGION_STRIDE 32`), with
`CONTIGUOUS=1, RADIX3=2, LOC_SYSMEM=1, LOC_FB=2` [src] `:35-45`. The C caps its scan at
**16** entries and stops at the first all-zero entry (`C:3388-3407`).

That the array is zero-terminated is **[inferred] I8** — the header does not say so; RMARGS is
inserted at an index the driver tracks (`ogkm: kernel_gsp.c:6224-6229`) and the descriptor is
`portMemSet`-zeroed on allocation. The 16-entry cap is a **parameter**, and a guest with more
regions before `RMARGS` would silently not be found. Bound the scan by
`region_size / stride`, capped at `LIBOS_MEMORY_REGION_INIT_ARGUMENTS_MAX`.

### D7 The `rpc.length = 36` constant

`C:1586` writes 36 for a bare header; `sizeof(rpc_message_header_v03_00) == 32`
[src] `ogkm: g_rpc-message-header.h:41-52`, and the same C file's own arithmetic uses 32 at
`C:1637` and `C:1657`. Already caught and recorded at `rs: crates/kayfabe-abi/src/view.rs:331-339`.
Survives only because both sides checksum the *declared* length over zero-padded memory.
Listed here for completeness as GSP-D1.

---

## 10. Every `[inferred]` claim, and what settles it

| id | claim | why inferred | settles it |
|---|---|---|---|
| **I1** | E3 — a STARTCPU while already booted is a no-op | ogkm's `kgspBootstrap` has no "re-STARTCPU while running" path, and the C's `!s->fwsec_ran` guard (`C:4259`) has the same effect; but neither *states* idempotency | S5: replay a trace containing consecutive STARTCPUs and assert no second `INIT_DONE`. If none exists, S6 with a deliberate double-STARTCPU |
| **I2** | the C device is single-threaded on every traced path, so one counter is a total order | asserted twice in the C's own comments (`C:1573`, `C:1653`) as the justification for `static` buffers, never verified | add a `assert(qemu_mutex_iothread_locked())`-equivalent to the capture hook and run the existing bench suite once |
| ~~I3~~ | ~~`MSGQ_FLAGS_SWAP_RX` agreement~~ | **RESOLVED to [src]** — `rxSwapped` is the AND of both `flags` (`ogkm: msgq.c:411-412`), the guest always sets it (`message_queue_cpu.c:180`), and nouveau hardcodes `tx.flags = 1` (`nv: r535/gsp.c:1171`). Now §1.1 item 11 | — (keep the S1 assertion anyway; it is cheap and it is the one that deadlocks silently) |
| **I4** | the boot-args mailbox pair is complete when the **high** half is written | the C keys on MAILBOX1 (`C:4298-4302`) and ogkm writes lo then hi (`kernel_gsp_tu102.c:401-402`) — a write *order*, not a *protocol* guarantee | S5: check every recorded trace for a hi-then-lo ordering. Robust design: treat the pair as complete when both halves have been written since the last reset, not on a specific half |
| **I5** | preserving `seqNum` across a rebind is right for an idle-release re-acquire but **not** for a true `rmmod`/`insmod` | `MESSAGE_QUEUE_INFO` is destroyed in `kgspDestruct` (module unload) and not on an idle release, so `rxSeqNum` survives one and not the other — but no run has ever got far enough to observe the `insmod` case (`../reference/mode2_bench_lifecycle.md` §3) | S7, after the latch fix. **The seqNum question is downstream of the latch chain and cannot be answered before it** |
| **I6** | the C's element-header offsets are 580-correct | transcribed into a design doc from a 580 tree that is not vendored; the implementation works on the bench, which is strong but indirect | vendor a 580.159.04 ogkm tag and regenerate (§11-O4) |
| **I7** | the proposed `GspModel` seam is sufficient for a second generation | derived from one generation's complete register set plus ogkm's `_TU102` HAL; only one generation has been implemented | S8, or a paper exercise: enumerate `kernel_gsp_gh100.c`'s HAL overrides and check each maps to a `GspModel` method |
| **I8** | the LibOS region array is zero-terminated | the header defines no terminator (`ogkm: src/common/uproc/os/common/include/libos_init_args.h:31-56`); the C relies on it (`C:3399-3401`) and it works, but the mechanism is "the descriptor was zeroed on allocation", not a declared sentinel | S5: check every recorded trace's region array against `region_size / 32` entries and confirm nothing non-zero follows the first zero. Robust design: bound by the descriptor's declared **size**, treat a zero entry as *skip*, not *stop* |
| **I9** | `phase == Running` is sufficient to guarantee the guest is not inside a boot-time poll that would trip the recursive-poll assert (`ogkm: kernel_gsp.c:2893`) | `Running` is entered only after `GSP_INIT_DONE` is consumed, which is the last boot poll — but the guest issues synchronous RPCs continuously afterwards, and the assert is about *any* nested poll | S5/S6: the completion plane's post point must be observed never to fire between a command's arrival and its reply. This is really a `kayfabe-completion` obligation and should be recorded as one |

---

## 11. What could not be determined

| id | question | why it matters | experiment |
|---|---|---|---|
| **O1** | where does the non-GSP register model live? | `kayfabe-gsp` will become the dumping ground by default | a design decision, not an experiment. Proposed boundary: *"does GSP FSM state appear in the answer?"* — §3.2's table, nothing more |
| ~~**O2**~~ | ~~is the shared region contiguous?~~ | **RESOLVED: it is `NV_MEMORY_NONCONTIGUOUS` and self-describing** (`ogkm: message_queue_cpu.c:254-256, 297-329`). The port **must** walk the table; the C's linear addressing is a latent bug (§4.1) | residual, purely informational: dump the 129-entry array on one bench boot to see how often it is in fact contiguous — that tells us whether the C was lucky or whether contiguity is typical. **Read-only, no GPU work** |
| **O3** | do `rxSeqNum`/`txSeqNum` reset on a true `rmmod`/`insmod`? | decides whether rebind preserves or zeroes seqNums (I5) | S7. Not reachable until the latch chain is fixed |
| **O4** | is the 580 element layout as transcribed, and does 580 validate MCTP? | D1/D2 — the bench runs 580, the vendored tree is 610 | vendor `open-gpu-kernel-modules` at 580.159.04 and regenerate `ElementLayout` from it. **No GPU. Should be S0's first task.** |
| **O5** | ★ which boot mode does a post-fn-47 re-acquire use? | D5 — decides whether the FSM rebinds and re-posts `INIT_DONE` on it at all | in-guest `dmesg` at `LEVEL_INFO` during a cuCtxDestroy→cuCtxCreate cycle: `kgspBootstrap` logs distinguishably, and `GspStatusQueueInit` logging *"Status queue linked"* (`message_queue_cpu.c:377`) fires only on NORMAL. **One bench boot** |
| **O6** | does the guest ever post a *command* larger than one element during boot? | decides whether GSP-D6's multi-element read is on the boot critical path or only the steady state | S5, from a recorded trace: count commands whose `rpc.length > elementSizeMin - hdrSize`. **No GPU** — the trace already exists once §6.2's patch lands |
| ~~**O7**~~ | ~~`GSP_RUN_CPU_SEQUENCER` (0x1002)~~ | **RESOLVED: not implemented in 610.** The only occurrence in the tree is the enum definition (`ogkm: rpc_global_enums.h:255`); it is absent from `_kgspProcessRpcEvent`'s switch, so it falls to `default:` and is logged-and-ignored. **Do not emit it.** The C is right to omit it | — |
| **O8** | does the bench's 580 declare `queueElementHdrSize` in `MESSAGE_QUEUE_INIT_ARGUMENTS` (610-shaped) or not (r570-shaped)? | D8 — decides whether the element header size is derivable or must come from an Axis-A constant on the bench | falls out of O4 (vendor a 580 tree). Until then the port must implement **both** paths, which it should anyway |

---

## 12. Citation table

Spot-checkable index. `ogkm` = `/workspace/nvidia-gpu-passthrough/research_clones/ogkm` @ 610.43.02;
`nv` = `/workspace/nvidia-gpu-passthrough/research_clones/linux/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/`;
`C` = `/workspace/nvidia-gpu-passthrough/src/qemu/`; `C-repo` = `/workspace/nvidia-gpu-passthrough/`;
`rs` = `/workspace/nvkvm-rs/`.

| claim | source | file:line |
|---|---|---|
| vendored ogkm is 610.43.02 | ogkm | `version.mk:1` |
| bench driver is 580.159.04 | rs | `crates/kayfabe-abi/src/versions.rs` (`BENCH_DRIVER`) |
| `msgqTxHeader` / `msgqRxHeader` fields | ogkm | `src/nvidia/inc/libraries/msgq/msgq_priv.h:48-65` |
| queue layout (tx hdr, rx hdr, entries) | ogkm | `msgq_priv.h:40-46` |
| `MSGQ_VERSION = 0` | ogkm | `msgq_priv.h:38` |
| `rxHdrOff`/`entryOff`/`msgCount` derivation | ogkm | `src/nvidia/src/libraries/msgq/msgq.c:234-250` |
| `MSGQ_FLAGS_SWAP_RX` pointer topology | ogkm | `msgq.c:264-278`, `:411-424` |
| `msgqRxLink` 9-check predicate; **`-7` = `rx.size != size`** | ogkm | `msgq.c:330-405`, esp. `:383-386` |
| `msgqRxLink` zeroes readPtr, `rxReadPtr = 0` | ogkm | `msgq.c:426-441` |
| `msgqTxGetFreeSpace` = `rp + count - wp - 1`; `rp >= count → 0` | ogkm | `msgq.c:483-497` |
| `msgqTxSubmitBuffers` wrap + free check | ogkm | `msgq.c:534-570` |
| `msgqRxGetReadAvailable`; `wp >= count → 0` | ogkm | `msgq.c:639-667` |
| `msgqRxMarkConsumed` writes `pReadOutgoing` | ogkm | `msgq.c:704-745` |
| `GSP_MSG_QUEUE_ELEMENT` (610) | ogkm | `src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:52-67` |
| `_checkSum32` XOR fold + past-end read licence | ogkm | `message_queue_priv.h:191-209` |
| `gspMsgQueueBytesToElements` | ogkm | `message_queue_priv.h:117-121` |
| `queueElementHdrSize` incl. CC | ogkm | `src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:82-86` |
| `queueElementSizeMin/Max`, header/entry align | ogkm | `message_queue_cpu.c:88-91` |
| default queue sizes 0x40000; PTE count derivation | ogkm | `message_queue_cpu.c:70-72, 120-136` |
| `msgqTxCreate(..., MSGQ_FLAGS_SWAP_RX)` | ogkm | `message_queue_cpu.c:172-178` |
| `GspStatusQueueInit` retry loop, 4 s / `NV_U32_MAX` on emu | ogkm | `message_queue_cpu.c:337-412` |
| send: length bound, zero-pad-to-8, MCTP/NVDM fill, `seqNum = txSeqNum`, checksum | ogkm | `message_queue_cpu.c:487-543` |
| `txSeqNum++` after submit | ogkm | `message_queue_cpu.c:610-620` |
| receive: nElements from declared length | ogkm | `message_queue_cpu.c:698-705` |
| receive: checksum over `hdrSize + rpc.length` | ogkm | `message_queue_cpu.c:724-734` |
| receive: **MCTP/NVDM validation** | ogkm | `message_queue_cpu.c:737-758` |
| receive: seqNum check + old-package recovery (only `<`) | ogkm | `message_queue_cpu.c:762-782` |
| receive: `rxSeqNum++` at `exit:` even on failure | ogkm | `message_queue_cpu.c:836` |
| `rpc_message_header_v03_00` fields, size 32 | ogkm | `src/nvidia/generated/g_rpc-message-header.h:41-52` |
| `NV_VGPU_MSG_SIGNATURE_VALID = 0x43505256` | ogkm | `src/nvidia/inc/kernel/vgpu/rpc_headers.h:61` |
| header version MAJOR 3 / MINOR 0 | ogkm | `rpc_headers.h:56-59` |
| `UNLOADING_GUEST_DRIVER = 47` | ogkm | `src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:57` |
| `GSP_SET_SYSTEM_INFO = 72`, `SET_REGISTRY = 73`, `GSP_RM_CONTROL = 76` | ogkm | `rpc_global_enums.h:82-86` |
| `GSP_INIT_DONE = 0x1001`, `POST_EVENT = 0x1003` | ogkm | `rpc_global_enums.h:254-256` |
| `kgspBootstrap_TU102` full ordering | ogkm | `src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:522-618` |
| mailbox programming is NORMAL-only | ogkm | `kernel_gsp_tu102.c:561-563` |
| `kgspProgramLibosBootArgsAddr_TU102` writes MAILBOX0/1 | ogkm | `kernel_gsp_tu102.c:392-403` |
| init RPCs + status-queue link are NORMAL-only | ogkm | `kernel_gsp_tu102.c:576-585, 607-611` |
| `kgspSendInitRpcs` = SET_SYSTEM_INFO then SET_REGISTRY | ogkm | `src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:4686-4709` |
| `INTERRUPT_PROCESSOR_SUSPENDED_VALUE = 0x80000000` on MAILBOX0 | ogkm | `kernel_gsp_tu102.c:333, 336-357` |
| `kgspIsWpr2Up_TU102` = `WPR2_ADDR_HI._VAL != 0` | ogkm | `kernel_gsp_tu102.c:1172-1180` |
| **"unexpected WPR2 already up"** gate | ogkm | `kernel_gsp.c:4805-4809` |
| `kgspTeardown_TU102`: reset, FWSEC-SB, Booter Unload | ogkm | `kernel_gsp_tu102.c:660-703` |
| Booter Unload asserts WPR2 down | ogkm | `kernel_gsp_booter_tu102.c:175-187` |
| fn-47 emission point | ogkm | `kernel_gsp.c:5231` (in `kgspUnloadRm_IMPL:5213`) |
| `kgspUnloadRm` callers | ogkm | `gpu.c:3919`, `gpu.c:4252`, `gpu_suspend.c:120`, `subdevice_ctrl_gpu_kernel.c:689` |
| reply matched on `(function, sequence)` | ogkm | `kernel_gsp.c:1824-1828` |
| unexpected events logged + ignored | ogkm | `kernel_gsp.c:1587-1599` |
| bootup-window event allowlist + `NV_ASSERT(0)` | ogkm | `kernel_gsp.c:1419-1440` |
| `kgspWaitForRmInitDone` polls `(0x1001, 0)` | ogkm | `kernel_gsp.c:6264-6283` |
| C: state fields | C | `qemu/nvkvm_gpu_emul.c:161-196` |
| C: checksum + post_status | C | `nvkvm_gpu_emul.c:1535-1620` |
| C: **unguarded `% q_msgcount`** | C | `nvkvm_gpu_emul.c:1615` (guarded sibling `:1608-1609`) |
| C: `rpc.length = 36` | C | `nvkvm_gpu_emul.c:1586` |
| C: SWGEN0 one-batch gate + its rationale | C | `nvkvm_gpu_emul.c:1683-1710` |
| C: cmd-queue service; the two guest reads | C | `nvkvm_gpu_emul.c:2380-2410` |
| C: fn-47 handler + "TWO distinct triggers" | C | `nvkvm_gpu_emul.c:2450-2481` |
| C: continuation elements skipped, not read | C | `nvkvm_gpu_emul.c:3341-3350` |
| C: readPtr ack into the status queue's rx hdr | C | `nvkvm_gpu_emul.c:3352-3358` |
| C: boot-args / RMARGS / tx-header handshake | C | `nvkvm_gpu_emul.c:3377-3494` |
| C: M9 reply-paramsSize clamp | C | `nvkvm_gpu_emul.c:3237-3252` |
| C: `gsp_reloaded` latch | C | `nvkvm_gpu_emul.c:4205-4214` |
| C: SEC2 Booter-Unload detection (mbox0 == 0xff) | C | `nvkvm_gpu_emul.c:4216-4234` |
| C: **the STARTCPU misclassification** | C | `nvkvm_gpu_emul.c:4237-4287` |
| C: QUEUE_HEAD doorbell → service | C | `nvkvm_gpu_emul.c:4290-4293` |
| C: mailbox capture → dump_bootargs | C | `nvkvm_gpu_emul.c:4295-4303` |
| C: rom-device overlay (reads never trap) | C | `nvkvm_gpu_emul.c:1462-1477`, `:4315-4331` |
| C: GA10x register offsets (all of them, one file) | C | `qemu/mode2_regs_ga10x.h` |
| 580 element layout, transcribed | C-repo | `docs/design/mode2_m3_gsp_rpc.md` ("The message element") |
| verified live queue GPA (mailbox path fired) | C-repo | `docs/design/mode2_m3_gsp_rpc.md` ("UPDATE 3") |
| retracted "0 mailbox writes" note | C-repo | `docs/design/mode2_m3_gsp_rpc.md` ("UPDATE (2026-06-03)") |
| `kayfabe-gsp` scope | rs | `crates/kayfabe-gsp/src/lib.rs:1-19` |
| `Arch` / `GmmuFmt` / `UserdModel` / `PushbufferAbi` | rs | `crates/kayfabe-arch/src/lib.rs:334-395` |
| `RmEvent` | rs | `crates/kayfabe-core/src/rmgraph.rs:398` |
| `RpcEnvelope`, `SIZE = 32`, the 36-vs-32 finding | rs | `crates/kayfabe-abi/src/view.rs:301-357` |
| version table, `NoTableForVersion`, exact-boundary rationale | rs | `crates/kayfabe-abi/src/versions.rs:1-40` |
| `Vmm` / `Device` / `GuestRamMap::resolve` | rs | `crates/kayfabe-vmm/src/lib.rs:705, 844-849` |
| no `Device` implementor yet | rs | `ARCHITECTURE.md:38-43` |
| C-bug matrix row 23 (the GSP reboot FSM, open) | rs | `docs/design/c_bug_regression_matrix.md` |
| GL11 §2.2's falsification conditions | rs | `docs/design/gl11_region_arguments.md` |
| measured: one CUDA process per QEMU lifetime; `-7`; the security defects | rs | `docs/reference/mode2_bench_lifecycle.md` §1–§4 |
| L12 / L13 / ⚠8 / §4.2 / §4.5 | C-repo | `docs/design/mode2_rust_rewrite_architecture.md` |
| **msgq constants** `MSGQ_MSG_SIZE_MIN 16`, align bounds, `MSGQ_FLAGS_SWAP_RX 1` | ogkm | `src/nvidia/inc/libraries/msgq/msgq.h:31-51` |
| `msgqTxCreate` GSP parameters (0x40000, 4096, hdrAlign 4, entryAlign 12, SWAP_RX) | ogkm | `message_queue_cpu.c:88-91, 172-180` |
| region is `NV_MEMORY_NONCONTIGUOUS` sysmem | ogkm | `message_queue_cpu.c:254-256` |
| self-describing page table; `sharedMemPA = pPageTbl[0]` | ogkm | `message_queue_cpu.c:297-329` |
| `cmdQueueOffset`/`statQueueOffset` are byte offsets | ogkm | `kernel_gsp.c:5481-5490` |
| **`MESSAGE_QUEUE_INIT_ARGUMENTS` — 9 fields (610)** | ogkm | `src/nvidia/inc/kernel/gpu/gsp/gsp_init_args.h:32-45` |
| `MESSAGE_QUEUE_INIT_ARGUMENTS` — **6 fields (r535)**, incl. lockless queues | nv | `r535/nvrm/gsp.h:578-585` |
| `MESSAGE_QUEUE_INIT_ARGUMENTS` — **4 fields (r570)** | nv | `r570/nvrm/gsp.h:497-502` |
| **`GSP_MSG_QUEUE_ELEMENT` 48-byte form with `elemCount` (r535/r570)** | nv | `r535/nvrm/gsp.h:808-816`; send side `r535/rpc.c:93-102, 370` |
| independent confirmation of the tx-header values + SWAP_RX polarity | nv | `r535/gsp.c:1164-1181` |
| `LibosMemoryRegionInitArgument`, `..._MAX 4096`, kind/loc enums | ogkm | `src/common/uproc/os/common/include/libos_init_args.h:31-56` |
| RMARGS inserted into the libos init-args array | ogkm | `kernel_gsp.c:6224-6229` |
| MCTP/NVDM field defs + vendor id | ogkm | `src/nvidia/arch/nvalloc/common/inc/mctp_format.h:39-58`; `nvdm_format.h:61` |
| RPC header filled on send; signature written, never checked on receive | ogkm | `src/nvidia/src/kernel/rmapi/rpc_common.c:154-184` |
| `rpc.sequence` assignment (`pRpc->sequence++`) | ogkm | `kernel_gsp.c:402-406` |
| fn-47 is synchronous (`_issueRpcAndWait`) | ogkm | `src/nvidia/src/kernel/vgpu/rpc.c:9146-9170` |
| `CONTINUATION_RECORD` sequence discipline | ogkm | `src/nvidia/src/kernel/vgpu/rpc.c:2109-2147, 2192, 2213` |
| recursive-poll prohibition | ogkm | `kernel_gsp.c:2893` |
| `maxRpcSize` | ogkm | `kernel_gsp.c:3186` |
| GFW boot: falcon-halt poll, PLM bit 0, progress 0xFF | ogkm | `src/nvidia/src/kernel/gpu/arch/turing/kern_gpu_tu102.c:391-479`; halt poll `kernel_falcon_tu102.c:331-359` |
| `kflcnStartCpu_TU102` (CPUCTL vs CPUCTL_ALIAS) | ogkm | `kernel_falcon_tu102.c:236-249` |
| falcon reset → `DMACTL` DMEM/IMEM scrubbing DONE | ogkm | `kernel_falcon_tu102.c:177-194, 279-320` |
| RISCV active = `NV_PRISCV_RISCV_CPUCTL.ACTIVE_STAT` | ogkm | `src/nvidia/src/kernel/gpu/falcon/arch/ampere/kernel_falcon_ga102.c:53-55` |
| `msgqRxLink` loop runs `kgspHealthCheck` each iteration | ogkm | `message_queue_cpu.c:398-403` |
| **Hopper+**: FSP secure boot, BCR/STARTCPU, MAILBOX0-must-be-0, `HWCFG2.RISCV_BR_PRIV_LOCKDOWN=UNLOCK`, WPR2-under-CC, halt-only teardown | ogkm | `src/nvidia/src/kernel/gpu/gsp/arch/hopper/kernel_gsp_gh100.c:236-263, 500-544, 730-776, 968-996, 1039-1049` |
| `NV_PGSP_QUEUE_HEAD(i) = 0x110c00 + i*8`; MAILBOX0/1 = 0x110040/44 | ogkm | `src/common/inc/swref/published/ampere/ga102/dev_gsp.h:27-38` |
| WPR2 register addresses + `_VAL` field | ogkm | `src/common/inc/swref/published/turing/tu102/dev_fb.h:39-44` |
| GFW_BOOT / PLM register addresses | ogkm | `src/common/inc/swref/published/turing/tu102/dev_gc6_island{,_addendum}.h` |
| `_kgspBootGspRm` fails early if WPR2 up | ogkm | `kernel_gsp.c:4804-4812` |
| the two post-INIT_DONE RPCs (`SET_GUEST_SYSTEM_INFO`, `GET_GSP_STATIC_INFO`) | ogkm | `kernel_gsp.c:5153-5165` |

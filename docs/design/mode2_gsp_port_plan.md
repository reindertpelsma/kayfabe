# `kayfabe-gsp` — the faked-GSP port plan

> ## ★★ VERSION CORRECTION (2026-07-28) — read this before anything below
>
> Everything in §1–§12 that cites `ogkm:` without a version tag was read from
> **610.43.02**. The bench runs **580.159.04**, and the matching tree is now vendored at
> `/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04/`. **580 and 610
> disagree materially on the GSP path**, not cosmetically: the element layout, the element
> *count* semantics, the transport headers, the init-args struct, the bootup event
> allowlist, the boot-FSM ordering of the init RPCs, and the suspend sentinel's comparison
> are all different. §14 is the correction log; §0.1 is the citation rule that stops this
> class of error recurring; `gsp_580_correction_brief.md` is the code-change brief.
>
> **Two rows changed status.** §11-O7 is **no longer RESOLVED** — its answer was 610's and
> is *backwards* for the bench. §11-O4/O8 are now **answered from source**.
>
> **This is a recurrence, not a one-off.** "ogkm" was treated as *the* specification when it
> is a *versioned* specification. A version-split fact is a **seam**, not a defect to paper
> over: the owner directive is that the GSP layer must not be tied to a driver version, so
> both answers are recorded side by side and neither is promoted to "the" answer.

> **Status (★ corrected 2026-07-27, was "DESIGN … a 34-line skeleton"): S0–S5 ARE BUILT.**
> `kayfabe-gsp` is **~3,550 lines across 8 files**; §13 records what landed. This file began
> as the spec and is now **spec + build log** — read §13 before trusting §1–§12, because the
> build settled several things the spec had marked `[inferred]`.
>
> **S6–S8 remain unbuilt and need the bench.** No bench was touched producing any of this.
>
> *(Found by the S6 agent while surveying: the header still advertised a skeleton after the
> crate was written. Exactly the append-only failure `testing_doctrine.md` §6 describes — the
> build log at the bottom was right and the header at the top, which is what a reader meets
> first, was not.)*

## 0. How to read this file

**Tags**, as in `../reference/rm_semantics_measured.md`:

| tag | meaning |
|---|---|
| **[src]** | read from code, with `file:line`. The file is named by tree: `ogkm:`, `C:`, `rs:`. |
| **[measured]** | observed on hardware, with the run that observed it |
| **[inferred]** | a conclusion drawn from those. **Every one is also listed in §10** with the experiment that settles it. |
| **[open]** | not determined. §11. |

### 0.1 ★★ STANDING CITATION RULE — every `ogkm` citation carries its version tag

> **`ogkm` is not a tree. It is a *family* of trees, and a claim that does not say which one
> it came from is UNVERIFIED.**

Two trees are vendored, and they are different specifications:

| tag in a citation | path | NVIDIA version | standing |
|---|---|---|---|
| **`ogkm-580:`** | `research_clones/ogkm-580.159.04/` | **580.159.04** (`version.mk:1`) | ★ **the bench's own driver.** A `ogkm-580:` citation says what the guest we actually face requires. This is the tree that governs when the two disagree and only one can be built. |
| **`ogkm-610:`** | `research_clones/ogkm/` | **610.43.02** (`version.mk:1`) | a *future* driver, and the second point that makes a fact version-split rather than universal. |

Rules, normative for this file and for `kayfabe-gsp`'s doc comments:

1. **Every `ogkm` citation is written `ogkm-580:` or `ogkm-610:`.** A bare `ogkm:` is a
   defect; treat the claim it supports as unverified until re-read against a tagged tree.
2. **Line numbers belong to the tagged tree only.** They drift between tags even where the
   code is byte-identical — `kgspWaitForRmInitDone` is `kernel_gsp.c:5214` at 580 and
   `:6264` at 610, same function. Never carry a line number across a tag.
3. **Paths drift too.** The whole `msgq` library moved: `src/nvidia/src/libraries/msgq/` and
   `src/nvidia/inc/libraries/msgq/` at 610 are `src/common/shared/msgq/` and
   `src/common/shared/msgq/inc/msgq/` at 580.
4. **A claim verified at only one tag is `[src@580]` or `[src@610]`, never `[src]`.** `[src]`
   unqualified means *checked at both and identical*.
5. **When the tags disagree, both go in the document.** Picking one is how §11-O7 came to be
   marked RESOLVED with the answer that is wrong for the machine we run on.

**The two source trees have different standing, and the difference is the point.**

| tree | standing | what a citation to it proves |
|---|---|---|
| `ogkm-580` = `.../research_clones/ogkm-580.159.04` (**580.159.04**, `version.mk:1`) | ★ **THE SPECIFICATION WE FACE.** The bench's guest driver, verbatim. | that the driver we boot *requires* something |
| `ogkm-610` = `/workspace/nvidia-gpu-passthrough/research_clones/ogkm` (**610.43.02**, `version.mk:1`) | **A SECOND SPECIFICATION.** A driver we do not run yet; its disagreements with 580 are where the version seam has to exist. | that the guest driver *requires* something **at 610** |
| `C` = `/workspace/nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c` (9 614 lines) | **EVIDENCE.** A working implementation on **GA106 / RTX 3060 / driver 580.159.04**. | that something *works on GA106 with 580* — never that it is the protocol |
| `nv` = `/workspace/nvidia-gpu-passthrough/research_clones/linux/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/{r535,r570}` | **INDEPENDENT CORROBORATION.** nouveau's clean-room GSP client for **r535 and r570** — a second implementation of the same protocol, written by different people, and the *nearest* trees to the bench's 580. | that a protocol reading is not a misreading of one header, **and** what the protocol looked like *before* 610 |

A `[src]` to `C:` is therefore always implicitly **[measured on GA106+580]**. Where this
plan claims a C behaviour generalises, it says why, and the strong form of "why" is *ogkm
does it generation-independently* or *nouveau independently agrees*.

★ **This paragraph used to say "no tree here is the bench's driver". That is no longer
true, and it was the load-bearing defect in this plan.** `ogkm-580` is now vendored, and it
settles §9 D1–D3, D8, §10-I6 and §11-O4/O8 from source rather than from transcription. What
survives is the *shape* of the warning: **three of the boot path's structures have different
shapes across the available trees**, and now we can see all of them rather than inferring
the middle one.

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
5. ★★ **NOT INVARIANT — this is version-split, and it was the worst error in this list.**
   *Where the receiver gets its element count from* differs between the two tags, and the
   count is what drives the **ring advance**, so getting it wrong desynchronises the ring
   permanently.
   - **[src@610]** derived from the declared length,
     `ceil((hdrSize + rpc.length) / elementSizeMin)` — `ogkm-610: message_queue_cpu.c:698-705`,
     consumed at `:838`. The primitive is `gspMsgQueueBytesToElements`
     (`ogkm-610: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:117-121`).
   - **[src@580] read out of the element**, `nElements = pMQI->pCmdQueueElement->elemCount`
     at `ogkm-580: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:652-658`. It bounds the
     copy loop (`:628, 648-650`), the CC checksum span (`:676-677`), the CC decrypt span
     (`:742-743`), and — the load-bearing one — **`msgqRxMarkConsumed(hQueue, nElements)` at
     `:774`**, the ring advance. 580 **never** derives the count from `rpc.length`; the
     `msgLen` check at `:760-770` runs *after* the element has already been consumed and
     gates nothing.

   ⇒ **Two different version-specific invariants, and each is a real obligation:**
   - on **580**, the `elemCount` we write must equal how far we advanced `writePtr`, and the
     `elemCount` the *guest* wrote is authoritative for how far we advance `readPtr`;
   - on **610**, `rpc.length` must equal how far we advanced `writePtr`, and there is no
     `elemCount` field to agree with.

   The **shape** that is invariant is only this: *the producer's ring advance and the
   consumer's ring advance must be computed from the same number, and that number is
   declared somewhere in the element.* Where it is declared is Axis A.
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
    the API lock, only an allowlisted event function may be delivered; anything else is
    `NV_ASSERT(0)`. ★ **The allowlist itself is version-split — the `NV_ASSERT(0)` gate is
    the invariant, its contents are Axis A:**
    - **[src@580]** `ogkm-580: kernel_gsp.c:1469-1474` — **six** entries:
      `GSP_RUN_CPU_SEQUENCER`, `UCODE_LIBOS_PRINT`, `GSP_LOCKDOWN_NOTICE`,
      `GSP_POST_NOCAT_RECORD`, `GSP_INIT_DONE`, `OS_ERROR_LOG`.
    - **[src@610]** `ogkm-610: kernel_gsp.c:1424-1431` — **eight**, and they are not a
      superset: `GSP_RUN_CPU_SEQUENCER` is **gone**, and
      `PFM_REQ_HNDLR_STATE_SYNC_CALLBACK`, `GSP_LOAD_EXEC_GENERIC_BOOTLOADER`,
      `GSP_LOAD_EXEC_HS_BINARY` are added.

    Only `UCODE_LIBOS_PRINT`, `GSP_LOCKDOWN_NOTICE`, `GSP_POST_NOCAT_RECORD`,
    `GSP_INIT_DONE`, `OS_ERROR_LOG` are on **both**; that five-way intersection is the only
    safe set for a version-agnostic emitter, and `POST_EVENT` (0x1003) is on neither.
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
- The suspend sentinel on `FALCON_MAILBOX0` — a LibOS2/LibOS3 constant, not a protocol one.
  ★ **The comparison is version-split, and the difference decides what we may write:**
  - **[src@580]** `ogkm-580: kernel_gsp_tu102.c:1226-1238` — `return (mailbox == 0x80000000);`
    **exact equality**, with the constant inlined (there is no
    `INTERRUPT_PROCESSOR_SUSPENDED_VALUE` symbol at 580).
  - **[src@610]** `ogkm-610: kernel_gsp_tu102.c:333, 348` —
    `#define INTERRUPT_PROCESSOR_SUSPENDED_VALUE 0x80000000` and
    `return (mailbox & INTERRUPT_PROCESSOR_SUSPENDED_VALUE) != 0;` — a **mask**.

  ⇒ we must **write the whole value, never OR the bit into a shadow**: a mailbox shadow that
  still holds a boot-args half and gets bit 31 set reads as suspended at 610 and hangs the
  teardown poll forever at 580. Writing `0x80000000` exactly satisfies both. On 580 this poll
  is reached from two places: after fn-47 (`ogkm-580: kernel_gsp.c:4310`,
  `kgspWaitForProcessorSuspend_HAL`) and as a **bootstrap liveness fallback**
  (`ogkm-580: kernel_gsp_tu102.c:551`, `kflcnIsRiscvActive || _kgspIsProcessorSuspended`).
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
- MCTP/NVDM constants — **610 only.** `MCTP_MSG_HEADER_VENDOR_ID_NV = 0x10de`,
  `MCTP_MSG_HEADER_TYPE_VENDOR_PCI = 0x7e`, `NVDM_TYPE_RM_RPC = 0x25`
  [src@610] `ogkm-610: src/nvidia/arch/nvalloc/common/inc/mctp_format.h:39-58`,
  `.../nvdm_format.h:61`. Assembled: `mctpHeader = 0xC000_0001`,
  `nvdmHeader = 0x2510_DE7E` (derivations in `../reference/nvidia_abi_oracles.md` §6).
  ★ **At 580 there is no MCTP on the GSP path at all.** `mctp_format.h` does not exist;
  the only MCTP in the 580 tree is FSP / SEC2 / NVSwitch
  (`fsp_mctp_format.h`, `sec2_mctp_format.h`, `src/common/nvswitch/…`), and
  `NVDM_TYPE_RM_RPC` **does not appear anywhere in it**. Bytes @0–@7 of a 580 element are
  `authTagBuffer[0..8]`, which a CC-off guest never reads. ⇒ a 580 profile carries
  `transport: None` — **not measured placeholder words**, because there is nothing there to
  measure.

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

★★ **And it goes further than the tx header — but ONLY on 610.**
`MESSAGE_QUEUE_INIT_ARGUMENTS`, which the guest writes into the `RMARGS` region for us to
read, carries **nine** fields at 610 — the last five being exactly the parameters that would
otherwise be constants:

```c
NvU64    sharedMemPhysAddr;   NvU32 pageTableEntryCount;
NvLength cmdQueueOffset;      NvLength statQueueOffset;
NvLength queueElementHdrSize; NvLength queueElementSizeMin; NvLength queueElementSizeMax;
NvU32    queueHeaderAlign;    NvU32 queueElementAlign;
```
[src@610] `ogkm-610: src/nvidia/inc/kernel/gpu/gsp/gsp_init_args.h:32-45`, populated at
`ogkm-610: kernel_gsp.c:5481-5490`. ⇒ **on 610 even the element header size is declared by
the guest**, so `bEncryptionEnabled` need not be inferred either (it is folded into
`queueElementHdrSize` at `ogkm-610: message_queue_cpu.c:82-86`).

★★ **On 580 it is FOUR fields and none of them is geometry** — §11-O8, now answered:

```c
NvU64 sharedMemPhysAddr;  NvU32 pageTableEntryCount;
NvLength cmdQueueOffset;  NvLength statQueueOffset;
```
[src@580] `ogkm-580: src/nvidia/inc/kernel/gpu/gsp/gsp_init_args.h:29-34`, populated at
`ogkm-580: kernel_gsp.c:4486-4489`. Identical to nouveau's r570 shape.

⇒ **queue geometry is NOT negotiated at 580.** It is compile-time — `queueElementHdrSize =
offsetof(GSP_MSG_QUEUE_ELEMENT, rpc) = 48`, `queueElementSizeMin = RM_PAGE_SIZE = 4096`,
`queueElementSizeMax = 4096*16 = 65536`, `GSP_MSG_QUEUE_HEADER_ALIGN = 4`,
`GSP_MSG_QUEUE_ELEMENT_ALIGN = RM_PAGE_SHIFT = 12`
([src@580] `ogkm-580: message_queue_priv.h:91-104`) — so the faked GSP must supply
48/4096/65536/4/12 from an Axis-A table on the bench, and the "derive, don't declare"
capability simply is not offered by that driver.

`GSP_ARGUMENTS_CACHED` differs too: 580's has no `rmStateMonitorBufferArgs` and no
`bindataArgs`, so its **size and every post-`MESSAGE_QUEUE_INIT_ARGUMENTS` offset differ**
between the tags. Anything that reads past the queue-init block must be version-keyed.

The struct therefore has **three** shapes across the available trees — §9 D8 — so GSP-P1 is a
*capability*, not an assumption: read the fields when the version's layout has them, fall
back to Axis-A constants when it does not. **On the bench, the fallback is the only path.**

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

`kgspBootstrap_TU102` in order [src@610] `ogkm-610: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:522-618`.
★ **The 580 function is `ogkm-580: kernel_gsp_tu102.c:493-578` and it is a different
sequence — see §3.1a. Read that before implementing the FSM.**

Before B0, `_kgspBootGspRm` **fails early if WPR2 is up** — this is the gate a stale
emulator trips on a second `insmod`. [src@610] `ogkm-610: kernel_gsp.c:4804-4812`;
[src@580] `ogkm-580: kernel_gsp.c:3864-3876`, same message, same effect.

| # | step | ogkm line | guest-observable at our boundary |
|---|---|---|---|
| B0 | `kgspWaitForGfwBootOk` → `gpuWaitForGfwBootComplete` | `:1184-1202` | **three** things, in order: GSP falcon `CPUCTL.HALTED == TRUE` (a 2.05 s halt poll, `ogkm: kernel_falcon_tu102.c:331-359`); then `PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK` **bit 0 == 1**; then `..._GROUP_05(0)` bits 7:0 **== 0xFF**. [src] `ogkm: src/nvidia/src/kernel/gpu/arch/turing/kern_gpu_tu102.c:391-479` |
| B1 | `kgspExecuteScrubberIfNeeded` (if a scrubber ucode exists) | `:531-535` | falcon DMA + STARTCPU |
| B2 | **if NORMAL and frtsSize > 0**: `kflcnReset(GSP)` then `kgspExecuteFwsec` | `:544-557` | GSP falcon DMA (ucode load) + `CPUCTL` STARTCPU |
| B3 | `kflcnResetIntoRiscv` | `:559` | GSP falcon reset regs |
| B4 | **`kgspProgramLibosBootArgsAddr`** — `MAILBOX0 = lo32(addr)`, `MAILBOX1 = hi32(addr)` | `:562`, impl `:392-403` | **two mailbox writes. NORMAL only.** |
| B5 | `kgspExecuteBooterLoad(WprMeta PA)` | `:566-572` | SEC2 falcon DMA + mailbox args + STARTCPU → **WPR2 comes up** |
| B6 | ★ **610 ONLY.** **if NORMAL**: `kgspSendInitRpcs` = `GSP_SET_SYSTEM_INFO` (72) then `SET_REGISTRY` (73) | `:576-585`, impl `ogkm-610: kernel_gsp.c:4686-4709` | **two commands on the cmd queue + doorbell, BEFORE the status queue exists.** ★ **At 580 this step is not here at all** — the same two RPCs are queued *before the whole bootstrap*, §3.1a A2 |
| B7 | `FALCON_OS = riscvDesc->appVersion` | `:588-589` | one register write |
| B8 | liveness gate: `kflcnIsRiscvActive(...) \|\| _kgspIsProcessorSuspended(...)` | `:592-603` | reads RISCV CPUCTL active bit / `FALCON_MAILBOX0 & 0x80000000` |
| B9 | **if NORMAL**: `GspStatusQueueInit` → `msgqRxLink` retry loop | `:607-611`, impl `message_queue_cpu.c:337-412` | **polls the status-queue tx header until it validates**, 4 s (`NV_U32_MAX` under `IS_EMULATION`), and **calls `kgspHealthCheck_HAL` every iteration** — a queued crashcat report converts the spin into an immediate `NV_ERR_RESET_REQUIRED` (`:398-403`) |
| B10 | `kgspWaitForRmInitDone` → `rpcRecvPoll(GSP_INIT_DONE, 0)` | `:613`, impl `kernel_gsp.c:6264-6283` | drains the status queue until `(function, sequence) == (0x1001, 0)` |

### 3.1a ★★ The 580 boot order is NOT the 610 boot order — the init RPCs move

This is a **boot-FSM ordering difference**, so it lands on `crates/kayfabe-gsp/src/boot.rs`
and not only on a table in a document.

At 610, `kgspSendInitRpcs` is step **B6**, *inside* `kgspBootstrap_TU102`, after Booter Load.
At 580 **`kgspSendInitRpcs` does not exist**. The same two RPCs, in the same order, are sent
by `kgspQueueAsyncInitRpcs_IMPL` ([src@580] `ogkm-580: kernel_gsp.c:3753-3777`) — called from
`kgspInitRm_IMPL` at `ogkm-580: kernel_gsp.c:4141`, which is **before**
`_kgspBootGspRm` (`:4184`) and therefore before FWSEC, before Booter Load, before RISC-V
start and before the status-queue link. It is skipped only under SPDM
(`ogkm-580: kernel_gsp.c:4123-4133`). The comment in the driver says why:
*"Stuff the message queue with async init messages that will be run before OBJGPU is
created."*

**And the doorbell rings with them.** `rpcSendMessage` calls `kgspSetCmdQueueHead_HAL`
unconditionally after every successful submit ([src@580] `ogkm-580: kernel_gsp.c:425`), and
`_kgspRpcSanityCheck` (`:281-321`) has **no** "is the GSP up yet" gate. So on **every clean
580 boot**:

| # | 580 order | our boundary sees |
|---|---|---|
| A0 | `kgspWaitForGfwBootOk_HAL` (`ogkm-580: kernel_gsp.c:4100`) — hoisted **out** of bootstrap | the GFW/PLM reads (610's B0) |
| A1 | `kgspSetupLibosInitArgs`, `kgspPopulateGspRmInitArgs` (`:4118, :4121`) | nothing — guest-RAM writes only |
| A2 | `kgspQueueAsyncInitRpcs` (`:4141`) | **two commands land in the cmd ring and `QUEUE_HEAD(0)` is written TWICE — while our `QueueState` is still `Unbound`** |
| A3 | `_kgspBootGspRm` (`:4184`) → WPR2-up gate (`:3876`) → `kgspBootstrap_HAL(NORMAL)` (`:3908`) | — |
| A4 | scrubber, FWSEC, `kflcnResetIntoRiscv` | GSP falcon DMA + STARTCPU |
| A5 | `kgspProgramLibosBootArgsAddr` (NORMAL only) | the two mailbox writes |
| A6 | `kgspExecuteBooterLoad` | SEC2 STARTCPU → WPR2 up |
| A7 | `FALCON_OS = appVersion` | one register write |
| A8 | liveness: `kflcnIsRiscvActive \|\| _kgspIsProcessorSuspended` (`ogkm-580: kernel_gsp_tu102.c:551`) | RISCV active / MAILBOX0 |
| A9 | `GspStatusQueueInit` (NORMAL only) | the `msgqRxLink` spin |
| A10 | `kgspWaitForRmInitDone` (`ogkm-580: kernel_gsp.c:5214`) | drains for `(0x1001, 0)` |

★★ **Two consequences the plan did not have, and both are real:**

1. **E8 is not an attack signature at 580 — it fires twice on a healthy boot.** §3.3-E8 and
   ledger row GSP-D4 describe "a doorbell while `Unbound`" as *the* guest-reachable defect.
   At 580 that is also **normal, expected, protocol-correct guest behaviour**, twice, before
   any mailbox write. The refusal (read no guest RAM) stays right; its **classification** must
   not be "hostile", it must not escalate, and the negative-trace test's "exactly one
   `Refused`" arm has to account for the pre-bind pair.
2. **At bind time the command ring already has a backlog.** The guest's cmd `writePtr` is
   **2**, not 0, when E6 publishes. A doorbell-only service path never sees those two
   commands until the guest's *next* doorbell. It happens to recover — the guest sends
   `SET_GUEST_SYSTEM_INFO` (1) right after `INIT_DONE` and that doorbell drains all three in
   sequence order — but recovery-by-luck is not a design. **E6 must drain the command queue
   after publishing**, exactly as it would on a doorbell.

Teardown, `kgspUnloadRm_IMPL` → `kgspTeardown_TU102` [src@610] `ogkm-610: kernel_gsp.c:5213-5231`,
`kernel_gsp_tu102.c:660-703`. **[src@580]**: `kgspUnloadRm` has the same four callers
(`ogkm-580: gpu.c:3653`, `gpu.c:3973`, `gpu_suspend.c:121`,
`subdevice_ctrl_gpu_kernel.c:632` — the plan's §12 line numbers are 610's), fn-47 is emitted
at `ogkm-580: kernel_gsp.c:4301`, and `kgspWaitForProcessorSuspend_HAL` follows at `:4310`:

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
| **`FALCON_MAILBOX0`** | `phase == Suspending` → **exactly** `0x80000000` (written, never OR-ed onto the boot-args shadow — §1.2), else the shadow | `ogkm-580: kernel_gsp_tu102.c:1226-1238` (`==`); `ogkm-610: :336-349` (`&`) |
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

**610.43.02** [src@610] `ogkm-610: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:52-67`:

```c
typedef struct GSP_MSG_QUEUE_ELEMENT {
    NvU32 mctpHeader;   // @0   MCTP transport header
    NvU32 nvdmHeader;   // @4   NVDM over MCTP
    NvU32 checkSum;     // @8
    NvU32 seqNum;       // @12
    NvU8  payload[];    // @16  = [GSP_MSG_QUEUE_ENCRYPTION_TAG if CC] ++ rpc_message_header_v ++ data
} GSP_MSG_QUEUE_ELEMENT;
```

`queueElementHdrSize = offsetof(payload) = 16` (+40 if CC) [src@610] `ogkm-610: message_queue_cpu.c:82-86`.

**580.159.04**, now **[src@580] from the driver itself** — this used to be a transcription
into a C-side design doc, which is exactly the kind of second-hand claim §0.1 exists to
forbid. `ogkm-580: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:43-51`:

```c
typedef struct GSP_MSG_QUEUE_ELEMENT {
    NvU8  authTagBuffer[16];   // @0
    NvU8  aadBuffer[16];       // @16
    NvU32 checkSum;            // @32
    NvU32 seqNum;              // @36
    NvU32 elemCount;           // @40   ← load-bearing, §4.4 / §4.6
    NV_DECLARE_ALIGNED(rpc_message_header_v rpc, 8);   // @48
} GSP_MSG_QUEUE_ELEMENT;
```

`GSP_MSG_QUEUE_ELEMENT_HDR_SIZE = NV_OFFSETOF(GSP_MSG_QUEUE_ELEMENT, rpc) = 48`
[src@580] `ogkm-580: message_queue_priv.h:93`. It is **not** shifted by CC at 580: the CC tag
*is* `authTagBuffer`/`aadBuffer`, already inside the 48, so the 610 "+40 if CC" arithmetic has
no 580 analogue.

★ **nouveau independently confirms the 48-byte form for both r535 and r570** — byte-identical
field list, including `elemCount`, and `msg->elem_count = DIV_ROUND_UP(len, 0x1000)` on send
[src] `nv: r535/nvrm/gsp.h:808-816`, `nv: r535/rpc.c:93-102, 370`; the r570 tree reuses the
same `rpc.c`.

★★ **The break interval is narrower than this plan said: `(595.84, 610.43.02]`, not
`(570, 610]`.** Nine release tags were probed; the 48-byte-with-`elemCount` form is present at
**575.64.05, 580.65.06, 580.159.04, 580.173.02, 590.44.01, 590.48.01, 595.44.02, 595.84**, and
the 16-byte-with-MCTP form appears only at **610.43.02**. ⇒ the whole 580/590/595 range is on
the 48-byte side, and **`ElementLayout`'s predicate is `major >= 610`, not `> 570`.** (Only the
two tags named in §0.1 are vendored here; the other seven are relayed evidence — §14.4.)

**These are different protocols.** §9 D1–D3. The port must therefore treat the element
header as an Axis-A layout with at least these fields per version:

```rust
// kayfabe-abi
pub struct ElementLayout {
    pub hdr_size_plain: usize,     // 48 below 610, 16 at 610+
    pub hdr_size_cc: usize,        // 610+: + sizeof(encryption tag). Below 610 the CC
                                   //   buffers are already inside the 48 — same value.
    pub checksum_off: usize,       // 32 below 610, 8 at 610+
    pub seqnum_off: usize,         // 36 below 610, 12 at 610+
    pub elem_count_off: Option<usize>,  // Some(40) below 610, None at 610+
    pub transport: TransportHdr,   // None below 610 (no MCTP exists in that tree at all);
                                   //   Mctp { 0xC000_0001, 0x2510_DE7E } at 610+
}
```

★ **`elem_count_off` is not a decoration — it changes the ring's advance rule**, on both the
send and the receive side. §4.4 states both rules; §4.6 states the safety bound that comes
with it.

### 4.4 The seqNum discipline, exactly

Three counters that are routinely confused. Naming them apart is half the work:

| name | who owns it | wraps? | ogkm |
|---|---|---|---|
| **`writePtr` / `readPtr`** — ring **positions**, in *elements* | producer / consumer | **yes**, modulo `msgCount` | `msgq.c:521-527, 555-560` |
| **element `seqNum`** — per **message** | the sender, `txSeqNum++` | **no** — a free-running `NvU32` | `message_queue_cpu.c:514, 620` |
| **`rpc_message_header.sequence`** — the RPC **transaction id** | whoever originated the request | n/a | `kernel_gsp.c:1824-1828` |

★★ **The guest's receive order is version-split, and the split is at step 1 — the step that
also decides how far the ring advances.** The plan previously listed only 610's order and
called step 4 "610 only", which understated it by a lot.

**At 580** [src@580] `ogkm-580: message_queue_cpu.c:608-786` — `GspMsgQueueReceiveStatus`:

| # | step | line |
|---|---|---|
| 1 | read element 0, then **`nElements = pCmdQueueElement->elemCount`** — a *field read*, not a derivation. This adjusts the loop bound in place | `:648-658` |
| 2 | read the remaining `nElements - 1` elements, each `GSP_MSG_QUEUE_ELEMENT_SIZE_MIN` bytes, appended to the staging buffer | `:628, 648-650` |
| 3 | checksum: CC-off over `HDR_SIZE + rpc.length` (`:680-681`); **CC-on over `nElements * SIZE_MIN`** (`:676-677`) | `:674-690` |
| 4 | *(no transport-header check exists at 580)* | — |
| 5 | `seqNum == rxSeqNum`, with recovery only for `<` (`:699-714`) | `:693` |
| 6 | CC-on decrypt spans `(nElements * SIZE_MIN) - HDR_SIZE` | `:741-743` |
| 7 | **post-hoc** length sanity: `msgLen = 48 + rpc.length` must be in `[sizeof(GSP_MSG_QUEUE_ELEMENT)=80, 65536]`. **This runs after the element is already committed and gates nothing** | `:760-770` |
| 8 | `msgqRxMarkConsumed(hQueue, nElements)` — ★ **the ring advance, by `elemCount`** — then `rxSeqNum++` on success | `:774-782` |

**At 610** [src@610] `ogkm-610: message_queue_cpu.c:640-838`:

| # | step | line |
|---|---|---|
| 1 | read element 0; **derive** `nElements = ceil((hdrSize + rpc.length) / elementSizeMin)` | `:698-705` |
| 2 | read the remaining `nElements - 1` **contiguous** elements | `:673-684` |
| 3 | checksum over `hdrSize + rpc.length` must fold to 0 | `:724-734` |
| 4 | **`MCTP_HEADER_VERSION == 1` and NVDM vendor id == NV** | `:737-758` |
| 5 | `element.seqNum == pMQI->rxSeqNum` | `:762` |
| 6 | length sanity `msgLen ∈ [queueElementHdrSize, queueElementSizeMax]` | `:824-833` |
| 7 | `msgqRxMarkConsumed(nElements)` then `rxSeqNum++` | `:836-838` |

★ **What 610 checks at step 4 and what it does NOT.** Only the version nibble (`== 1`) and
the vendor id (`== 0x10de`). SOM, EOM, the sequence field and the NVDM *type* byte are
**unchecked** — so **no test may assert that the guest rejects a wrong SOM/EOM/SEQ/NVDM-type**.
That would be asserting a behaviour the driver does not have, which §4.4's `signature`
paragraph already forbids for the same reason.

★ **The lower length bound is also version-split, in our favour on the bench.** 580 requires
`48 + rpc.length >= sizeof(GSP_MSG_QUEUE_ELEMENT) = 80`, i.e. `rpc.length >= 32`; 610 requires
only `>= queueElementHdrSize`, i.e. `rpc.length >= 0`. So the `MsgLen::new` lower bound of
`sizeof(rpc_message_header_v)` that §7-G3 adds is *enforced by the driver itself* at 580 and
is our own addition only at 610.

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
Verified identical at both tags; the 580 copy is `ogkm-580: message_queue_priv.h:112-125`.

### 4.6 ★★ SAFETY INVARIANT — `elemCount > 16` corrupts the guest kernel heap

**This is the most important single fact in this document and it had never been written
down.** It is a bound on **what we are allowed to emit**, not a style rule, and it is the
only place in the whole GSP path where a value we choose can write outside a guest kernel
allocation.

The mechanism, entirely [src@580]:

1. The guest's receive staging buffer is allocated **once**, at
   `ogkm-580: message_queue_cpu.c:132-134`:
   `workAreaSize = (1 << GSP_MSG_QUEUE_ELEMENT_ALIGN) + GSP_MSG_QUEUE_ELEMENT_SIZE_MAX + msgqGetMetaSize()`
   = **4096 + 65536 + sizeof(msgqMetadata)**, from `portMemAllocNonPaged` — the kernel heap.
2. It is carved at `:143-145`:
   `pCmdQueueElement = ALIGN_UP(pWorkArea, 4096)` and
   `pMetaData = pCmdQueueElement + GSP_MSG_QUEUE_ELEMENT_SIZE_MAX`.
   ⇒ the element staging area is **exactly 65536 bytes = 16 elements of 4096**, and
   `pMetaData` — the live `msgq` handle — is the very next byte.
3. The copy loop at `:628, 648-650` runs `for (i = 0; i < nElements; i++)` and does
   `portMemCopy(pTgt, 4096, pNextElement, 4096); pTgt += 4096;` with **no bound on `nElements`
   other than the ring**. `nElements` came from `elemCount` at `:658` — a field **we write**.
4. The loop stops only when `msgqRxGetReadBuffer` returns `NULL`, which happens when `i`
   reaches the elements actually available in the ring
   (`ogkm-580: src/common/shared/msgq/msgq.c:673-693`). With the default geometry the ring
   holds `msgCount = 63` elements, so up to **62** copies are reachable.

⇒ **A status element whose `elemCount` exceeds 16 makes the guest kernel memcpy past the end
of a `portMemAllocNonPaged` allocation.** The first thing it overwrites is `pMetaData`, the
`msgq` metadata the guest is *actively using* — so the corruption is immediately
self-amplifying — and at the reachable maximum it writes `(62 − 16) × 4096 = 188 416` bytes
past the staging area, far beyond `workAreaSize`.

**Invariant GSP-S1 (normative, and it is a safety property, not a correctness one):**

> On any driver whose `ElementLayout` has an `elem_count_off`, the value we write there and
> the number of elements we advance `writePtr` by **must both be `<= queueElementSizeMax /
> queueElementSizeMin`** (16 with the bench's geometry, and derived from the geometry, never
> the literal 16). A message that would need more must be refused before it is encoded, with
> a named fault — never truncated, never clamped silently.

Three notes on why the obvious objections do not weaken it:

- *"`MsgLen` already bounds `rpc.length` by `element_size_max`, so the count can never exceed
  16."* True **today, by derivation**, and that is precisely the fragility: at 580 the
  guest reads the **field**, not the derivation, so the two are only equal because one
  encoder computes both. Any future path that sets `elemCount` from anything other than the
  same `MsgLen` — a continuation record, a CC layout change, a replayed trace, a fuzz
  harness — breaks the coupling silently. The bound must be checked where the field is
  written.
- *"The guest is the victim, so this is the guest's bug."* It is, and it is also *our*
  obligation: the guest kernel is inside the security boundary we are defending
  (`core_security_threat_model.md`), and a QEMU device that can corrupt guest kernel memory
  from an unvalidated width is the same defect class as `C:1615`'s SIGFPE, pointed the other
  way.
- *"610 has no `elemCount`, so this is 580-only."* The **field** is 580-only. The **bound** is
  not: 610 derives the count from `rpc.length`, and `rpc.length > queueElementSizeMax -
  hdrSize` is already refused there (`ogkm-610: message_queue_cpu.c:826-833`). Same ceiling,
  reached two different ways — which is exactly what makes it an invariant rather than a
  version quirk.

---

## 5. Staged build order

Nine stages. **Six need no GPU.** Each names what it *cannot* prove.

| stage | needs GPU? | what it builds | proves | cannot prove |
|---|---|---|---|---|
| **S0 — ABI extension** | **no** | `kayfabe-abi`: `msgqTxHeader`/`msgqRxHeader`, `LibosMemoryRegionInitArgument`, **all three shapes of** `MESSAGE_QUEUE_INIT_ARGUMENTS` (D8), `ElementLayout` per version (D1), the remaining `NV_VGPU_MSG_*` ids the boot path uses (1, 51, 65, 70, 71, 72, 73, 76, 0x1001, 0x1003). ★ **DONE 2026-07-28: the 580.159.04 tree is vendored** (§0.1), so O4/O8 are answered and the 580 shapes are `[src@580]`, no longer transcribed | generator-vs-rustc layout agreement; version key refuses below floor; the 580-vs-610 element split is expressed as data | — (its "cannot prove" row is discharged) |
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

[src@580] `ogkm-580: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:43-51`, header size at
`:93` — **read from the driver, 2026-07-28.** (This row previously cited a *transcription* into
`C-repo: docs/design/mode2_m3_gsp_rpc.md`; the transcription turns out to be correct, but it
was second-hand and §0.1 now forbids that form.) Implemented at `C: nvkvm_gpu_emul.c:1561-1620`.
[src@610] `ogkm-610: message_queue_priv.h:52-67`, `message_queue_cpu.c:82-86`.
[src] **r535 + r570 (independent): the 48-byte form, byte-identical**, including
`elemCount` — `nv: r535/nvrm/gsp.h:808-816`, `nv: r535/rpc.c:93-102`, and r570 reuses the same
`rpc.c`.

Neither is wrong; they are different driver versions, and the break is now bracketed to
**(595.84, 610.43.02]** — 575/580/590/595 are all on the 48-byte side (§4.3). **The finding is
that the C hard-codes 48/40/32/36 at ~15 sites** (`C:1583-1602, 2406-2419, 2734-2735,
3341-3350`, and every `cmd + N` offset in `service_cmdq`), so it is a 580-only implementation
with no version key. ⇒ Axis A, §4.3's `ElementLayout`, with the predicate **`major >= 610`**.

### D2 ★ 610 validates MCTP/NVDM transport headers; the C writes neither

`GspMsgQueueReceiveStatus` rejects an element whose `MCTP_HEADER_VERSION != 1` or whose NVDM
vendor id is not NV [src@610] `ogkm-610: message_queue_cpu.c:737-758`, and the sender fills
them via `mctpCreateTransportHeader(SOM=1, EOM=1, 0,0,0)` /
`mctpCreateNvdmHeader(NVDM_TYPE_RM_RPC)` [src@610] `:505-512`, giving
`mctpHeader = 0xC000_0001` and `nvdmHeader = 0x2510_DE7E`
(`../reference/nvidia_abi_oracles.md` §6). The C never writes offsets 0–7 of a status element
with anything but zero (`C:1583`: `memset(el, 0, …)` then fields from +32 up).

★ **The check validates the version nibble and the vendor id and NOTHING ELSE** — SOM, EOM,
the sequence field and the NVDM type byte are unread. **No test may assert that the guest
rejects them.**

⇒ **A port that ships only the C's encoding would be rejected on 610** with
`"MCTP protocol violation"`.

★★ **O4 is ANSWERED: 580 has no MCTP anywhere near the GSP path.** `mctp_format.h` does not
exist in the 580 tree; the only MCTP there is FSP (`fsp_mctp_format.h`), SEC2
(`sec2_mctp_format.h`) and NVSwitch, and `NVDM_TYPE_RM_RPC` does not appear in the tree at
all. Bytes @0–@7 of a 580 element are `authTagBuffer[0..8]`, which a CC-off guest never reads.
⇒ **the 580 profile is `transport: None` — not a placeholder word, because there is no word.**

### D3 ★★ `elemCount` — the field 610 removed and 580 *depends on*

610 derives `nElements` from `hdrSize + rpc.length` [src@610] `ogkm-610: message_queue_cpu.c:698-705`;
there is no `elemCount` field. On 610 offset +40 is **inside the RPC header**
(payload@16 + 24 = `rpc.sequence`), so replaying the C's encoder against 610 would corrupt the
transaction id.

★★ **The reverse error is worse, and it is the one the bench would hit.** At 580 `elemCount`
is not decorative: it is the *only* source of the receiver's element count
([src@580] `ogkm-580: message_queue_cpu.c:652-658`) and therefore of `msgqRxMarkConsumed`'s
ring advance (`:774`). Consequences, both directions:

- **Emitting (status queue).** Our `elemCount` and our `writePtr` advance must be the same
  number, or the guest consumes a different number of slots than we produced and the ring
  desynchronises permanently. Deriving both from one `MsgLen` satisfies this — but see §4.6
  for why the bound must still be checked at the write.
- **Consuming (command queue).** The *guest's* `elemCount` is what its own producer advanced
  `writePtr` by (`ogkm-580: message_queue_cpu.c:482, 578` — it submits `pCQE->elemCount`
  buffers). So at 580 **our `readPtr` advance must come from the element's `elemCount` field,
  not from `ceil(msgLen / elementSizeMin)`.** For a conforming guest the two agree; for a
  hostile or buggy one they do not, and a derivation-based consumer silently desynchronises
  the ring in a way that no later check catches. The derivation stays as a *cross-check* that
  refuses by name when the two disagree — it must not be the source.

Corollary: the C's own multi-element split (`C:1596-1613`) computes `nelems` the 610 way and
*also* writes the 580 field, so the C is correct only because a conforming guest makes the two
identical.

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
| ★ **ogkm-580** (`ogkm-580: gsp_init_args.h:29-34`, written at `kernel_gsp.c:4486-4489`) | the same **4** as r570. **[src@580] — this is the bench's shape, and O8 is answered: 580 declares NO geometry.** |
| **ogkm-610** (`ogkm-610: gsp_init_args.h:32-45`) | the 4, **plus** `queueElementHdrSize, queueElementSizeMin, queueElementSizeMax, queueHeaderAlign, queueElementAlign` — **9** |

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
3. ★★ **On the bench that capability is absent**, so the port must ship the fallback path and
   hardcode 48 / 4096 / 65536 / 4 / 12 from Axis A (§1.3). A port that only implemented the
   610 "derive it" path would have nothing to read on the machine it runs on.
4. ★ **`GSP_ARGUMENTS_CACHED` differs beyond the queue block.** 580's has
   `messageQueueInitArguments, srInitArguments, gpuInstance, bDmemStack, profilerArgs,
   sysmemHeapArgs` ([src@580] `ogkm-580: gsp_init_args.h:36-64`); 610's adds
   `rmStateMonitorBufferArgs` and `bindataArgs`. Since `MESSAGE_QUEUE_INIT_ARGUMENTS` is the
   **first** member and grows by 40 bytes at 610, **every subsequent offset in that struct
   differs between the tags**. Nothing in the boot path reads them today; anything that starts
   to must be version-keyed, not offset-transcribed.

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
| ~~I6~~ | ~~the C's element-header offsets are 580-correct~~ | **RESOLVED to [src@580], 2026-07-28.** `ogkm-580: message_queue_priv.h:43-51` + `:93` gives `authTag@0, aad@16, checkSum@32, seqNum@36, elemCount@40, rpc@48, HDR_SIZE=48`. The transcription was right — but the *habit* that made it `[inferred]`, citing a doc instead of a tree, is the same habit that left §11-O7 marked RESOLVED with the wrong version's answer. §0.1 | — |
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
| ~~**O4**~~ | ~~is the 580 element layout as transcribed, and does 580 validate MCTP?~~ | **ANSWERED 2026-07-28** by vendoring `ogkm-580`. Layout: yes, exactly as transcribed (§9 D1). MCTP: **no — 580 has no MCTP on the GSP path at all**, `mctp_format.h` does not exist and `NVDM_TYPE_RM_RPC` is absent from the tree (§9 D2). ⇒ 580 profile = `transport: None` | — |
| **O5** | ★ which boot mode does a post-fn-47 re-acquire use? | D5 — decides whether the FSM rebinds and re-posts `INIT_DONE` on it at all | in-guest `dmesg` at `LEVEL_INFO` during a cuCtxDestroy→cuCtxCreate cycle: `kgspBootstrap` logs distinguishably, and `GspStatusQueueInit` logging *"Status queue linked"* (`message_queue_cpu.c:377`) fires only on NORMAL. **One bench boot** |
| **O6** | does the guest ever post a *command* larger than one element during boot? | decides whether GSP-D6's multi-element read is on the boot critical path or only the steady state | S5, from a recorded trace: count commands whose `rpc.length > elementSizeMin - hdrSize`. **No GPU** — the trace already exists once §6.2's patch lands |
| ★★ **O7** | `GSP_RUN_CPU_SEQUENCER` (0x1002) — **RESOLVED STATUS WITHDRAWN 2026-07-28.** It was marked RESOLVED on **610** evidence and the **580** answer is the opposite; see the restatement immediately below this table. | it decides whether the faked GSP can ever drive a CPU-side sequencer — including the SEC2 **CORE_RESUME** path, which bears on "GPU restart must work without a bolt-on" | version-split, both answers now `[src]`. What remains open is narrower and named in O7a |
| ~~**O8**~~ | ~~does the bench's 580 declare `queueElementHdrSize` in `MESSAGE_QUEUE_INIT_ARGUMENTS`?~~ | **ANSWERED 2026-07-28: NO.** 580's struct has the r570 **4** fields (`ogkm-580: gsp_init_args.h:29-34`, written at `kernel_gsp.c:4486-4489`); 610's has 9. ⇒ **queue geometry is not negotiated at 580** and must come from Axis A: 48 / 4096 / 65536 / 4 / 12 (`ogkm-580: message_queue_priv.h:91-104`). §1.3, §9 D8 | — |
| ~~**O7a**~~ | ~~at 580, is a `GSP_RUN_CPU_SEQUENCER` event *required* or merely *accepted*?~~ | ★★★ **RESOLVED 2026-08-06 — merely ACCEPTED, on both versions, and the contract is `emit-never`.** See §11.2 | — |

### 11.1 ★★ O7 restated — it is version-split, and 580 is the one that governs

The RESOLVED text above said *"not implemented, do not emit it"*. **That is 610's answer.**
The bench runs 580, where it is **fully implemented**:

| | 580.159.04 | 610.43.02 |
|---|---|---|
| in `_kgspProcessRpcEvent`'s dispatch switch | ★ **YES** — `ogkm-580: kernel_gsp.c:1486-1487`, `case NV_VGPU_MSG_EVENT_GSP_RUN_CPU_SEQUENCER: nvStatus = _kgspRpcRunCpuSequencer(...)` | **no** — falls to `default:`, logged and ignored |
| on the bootup-window allowlist | ★ **YES**, and it is the **first** entry — `ogkm-580: kernel_gsp.c:1469` | **no** |
| executor | `kgspExecuteSequencerBuffer_IMPL` (`ogkm-580: kernel_gsp.c:5259-5394`) → `kgspExecuteSequencerCommand_HAL` → `kgspExecuteSequencerCommand_TU102` (`ogkm-580: kernel_gsp_tu102.c:913`) / `_GA102` (`kernel_gsp_ga102.c:136`) | **deleted** — no `ExecuteSequencerBuffer`, no `RunCpuSequencer`, no `GSP_SEQ_BUF_OPCODE` anywhere outside the enum |
| the enum entry | `ogkm-580: rpc_global_enums.h:254` | `ogkm-610: rpc_global_enums.h:255` — the only surviving trace |

**What governs the bench:** at 580 an emitted `GSP_RUN_CPU_SEQUENCER` is *dispatched*, is
*allowed during the bootup poll*, and *executes register writes / polls / falcon resets on the
guest's CPU side*. So:

- The claim *"do not emit it"* is still the right **default** — but for a different reason
  (we have no sequencer buffer to send), and it is no longer true that emitting it would be
  harmlessly ignored. At 580 a malformed sequencer buffer runs `GSP_SEQ_BUF_OPCODE_REG_WRITE`
  and friends inside the guest kernel (`ogkm-580: kernel_gsp.c:5293-5394`).
- The **allowlist** consequence is the sharper one: §1.1 item 10's list has **six** entries at
  580 and **eight different** ones at 610, and `GSP_RUN_CPU_SEQUENCER` is on exactly one of
  them. Any code or test that encodes "the bootup allowlist" as a single set is wrong at one
  of the two tags.
- ★ **The resume tie-in.** At 580, `GSP_SEQ_BUF_OPCODE_CORE_RESUME` — reset into RISC-V,
  re-program the LibOS boot-args address, start SEC2, wait for
  `NV_PGC6_BSI_SECURE_SCRATCH_14._BOOT_STAGE_3_HANDOFF == _VALUE_DONE`, then check SEC2
  `FALCON_MAILBOX0 == 0` — lives **inside the sequencer executor**
  (`ogkm-580: kernel_gsp_tu102.c:913-960`), and the executor is reachable **only** from
  `_kgspRpcRunCpuSequencer`, i.e. only from an event **we** send. At 610 the same handoff
  survives as `kgspExecuteCoreResume_TU102` (`ogkm-610: kernel_gsp_falcon_tu102.c:455`).

  ⊘ **CORRECTED 2026-08-06 — this bullet used to end *"at 580 a GSP resume is RPC-driven and
  at 610 it is host-driven"*, and the second half is FALSE.** At 610 `kgspExecuteCoreResume`
  has exactly two callers — `kgspLoadAndExecuteGenericBootloader_TU102`
  (`ogkm-610: kernel_gsp_falcon_tu102.c:563`) and `kgspLoadAndExecuteHsBinary_GA102`
  (`ogkm-610: kernel_gsp_falcon_ga102.c:401`) — and **both are reached only from RPC event
  handlers**: `_kgspRpcLoadAndExecuteGenericBootloader` / `_kgspRpcLoadAndExecuteHsBinary`
  (`ogkm-610: kernel_gsp.c:432-458`), dispatched at `:1579-1584` for events
  `GSP_LOAD_EXEC_GENERIC_BOOTLOADER` (0x1026) / `GSP_LOAD_EXEC_HS_BINARY` (0x1027), with their
  params read **out of the RPC body**. ★ The functions are *local to a falcon file*; that is
  locality of **definition**, which was misread as locality of **triggering**. See §11.2.


### 11.2 ★★★ O7a RESOLVED — CPU-assist events are `emit-never`, and it is version-INDEPENDENT

**The question:** at 580 is `GSP_RUN_CPU_SEQUENCER` *required* on a path we must support, or
merely *accepted*? **Answer: merely accepted — and the same holds at 610 for its successors.**

**What the two versions actually do** (`[src]`, dispatch read, not just tables):

| | 580.159.04 | 610.43.02 |
|---|---|---|
| route to `CORE_RESUME` | `GSP_RUN_CPU_SEQUENCER` (0x1002) → `_kgspRpcRunCpuSequencer` (`ogkm-580: kernel_gsp.c:1486`) → `kgspExecuteSequencerBuffer` → opcode arm (`kernel_gsp_ga102.c:151` for GA106) | `GSP_LOAD_EXEC_GENERIC_BOOTLOADER` (0x1026) / `GSP_LOAD_EXEC_HS_BINARY` (0x1027) → `ogkm-610: kernel_gsp.c:1579-1584` → `kgspExecuteCoreResume_TU102` (`kernel_gsp_falcon_tu102.c:455`) |
| 0x1002 dispatched? | ★ yes | **no** — zero occurrences in `ogkm-610: kernel_gsp.c`; falls to the default *"unexpected RPC event … log but otherwise ignore"* arm |
| who initiates | the **GSP** | the **GSP** |

⇒ ★★★ **The split is not "RPC vs local". On BOTH versions every route to `CORE_RESUME` begins
with a GSP→CPU event, and the CPU driver never starts one by itself.** The versions differ only
in *which* event carries it — a raw opcode buffer at 580, typed load-exec events at 610.

**Why `emit-never` is the compliant answer, not merely the cheap one.** The opcode family
(`ogkm-580: rmgspseq.h:78-89` — `REG_WRITE`, `REG_MODIFY`, `REG_POLL`, `DELAY_US`, `REG_STORE`,
`CORE_RESET/START/WAIT_FOR_HALT/RESUME`) exists so the **GSP can borrow the CPU while real
silicon is down**. An emulated GSP has no down-silicon phase, so it has nothing to ask for. And
nothing on the driver side waits for one: the boot wait is `rpcRecvPoll(… GSP_INIT_DONE)`
(`ogkm-580: kernel_gsp.c:5229`; `ogkm-610: kernel_gsp.c:6282`), and every timeout people
associate with the sequencer lives **inside** the handlers, running only if the event arrives.
Suspend waits on the falcon `MAILBOX0` processor state (`ogkm-580: kernel_gsp_tu102.c:1242`),
which we already own.

`[measured]` The C artifact is the empirical half: it contains **zero** occurrences of the
sequencer (`C: src/qemu/`, grep 2026-08-06) and a stock 580.159.04 driver nonetheless completed
cold boot → `cuCtxCreate` → matmul `bad=0` → teardown → GSP reload. So on 580 the driver
demonstrably does not need the event on any path we have exercised.

★★ **AND THE "UNREADABLE FIRMWARE" PREMISE IS DEAD** — `[measured]` 2026-08-06 against
`traces/rpctrace_ga106_boot1.bin` (real GA106, open 580.159.04): a real GSP sends **exactly one
`RUN_CPU_SEQUENCER` per bring-up** — records 2 and 481 of two sessions, `rpc_len=6328` — roughly
200 ms **before** `GSP_INIT_DONE` (records 5, 484). It is part of *normal cold boot*, not a
resume-only path, and the whole buffer is byte-decodable from a committed trace.

⊘ **That makes replay tempting and it is still the wrong move.** Replaying a captured 6328-byte
buffer would oblige us to emulate falcon halt bits, SEC2 start, the `SCRATCH_14` stage-3
handoff, SEC2 mailbox and ~104 `REG_POLL`s each with its own timeout failure mode — binding
kayfabe to one chip's boot script to serve a round trip whose purpose (reviving real silicon)
has no referent here. That is the *inverse* of the invented-encoding defect: a **too-capable**
double, which `mock_fidelity_both_directions` names as the same defect as a too-strict one.

**THE CONTRACT (version-independent):** *CPU-assist events — `0x1002` at 580, `0x1026`/`0x1027`
at 610 — are **emit-never**. The fake GSP performs their effects internally as instantaneous
state transitions. Resume and re-acquire are expressed solely through the observables the CPU
driver actually polls: the processor-suspend mailbox, WPR2, and a seqnum-correct `INIT_DONE`
re-post.*

⚠ **The one thing most likely to make this wrong,** and it is the invisible-refusal class the
ledger cannot show: a handler side effect we skip that something later *reads*. Named
candidates — the `NV_PFALCON_FALCON_OS` appVersion write and the `regSaveArea` mailbox stores
the GSP reads back after `CORE_RESUME` (`ogkm-580: kernel_gsp_ga102.c:184-186`,
`rmgspseq.h:215-217`). Inside a fully emulated device we own producer and consumer both, so this
*should* be vacuous — so S6 must assert it rather than assume it: a zero-CPU-assist boot reaches
`INIT_DONE`, **and** no guest read of `FALCON_OS` is ever answered from unwritten state.

⊘ **Not determined, and recorded as such:** what makes real GSP firmware emit the event
(closed); whether a real 610 GSP emits 0x1026/0x1027 on GA106 (**no 610 trace exists**); S3/GC6
behaviour on either version (never exercised by any bench).

---

## 12. Citation table

> ★★ **EVERY `ogkm` LINE NUMBER IN THIS TABLE IS A 610.43.02 LINE NUMBER**, and the table was
> written before `ogkm-580` existed. Per §0.1 rule 2, do **not** carry any of them into the
> 580 tree — paths moved (the whole `msgq` library is under `src/common/shared/msgq/` at 580)
> and line numbers drift even where the code is byte-identical. The rows corrected by the 580
> read are listed in §14; a row not listed there has **not** been re-checked at 580.
>
> **Re-verified identical at both tags** (so these rows hold verbatim, modulo path/line):
> the entire `msgq` layer — `msgq.c` differs only in `#include` placement and `msgq.h` /
> `msgq_priv.h` are byte-identical, so every `msgqRxLink` / `msgqTxGetFreeSpace` / `SWAP_RX` /
> `-7` claim survives — plus `g_rpc-message-header.h` (still 32 bytes), `libos_init_args.h`,
> `dev_gsp.h`, `dev_fb.h`'s WPR2 registers (0x1FA824/0x1FA828, `_VAL` 31:4),
> `kernel_falcon_tu102.c`, `rpc_common.c` (a local-variable refactor only — signature still
> written on send, still never checked on receive), `_checkSum32`, function numbers
> 1/47/65/72/73/76 and events 0x1001/0x1003, the `NV_ASSERT(0)` bootup gate, `maxRpcSize`,
> the recursive-poll prohibition, `kgspWaitForRmInitDone` polling `(GSP_INIT_DONE, 0)`, and
> the four `kgspUnloadRm` callers.

Spot-checkable index. `ogkm` (untagged, throughout this table) = `/workspace/nvidia-gpu-passthrough/research_clones/ogkm` @ 610.43.02;
`ogkm-580` = `/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04` @ 580.159.04;
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

---

## ★★ GOVERNING DIRECTIVE (owner, 2026-07-27) — port what the C proved, fix only what is bugged

> *"use regular traps. build the thing C proved work if it's not bugged."*

This outranks any cleverness in this document. Two halves:

### 1. Regular traps — no exotic memory mechanism

Guest register and mailbox accesses arrive through the **ordinary MMIO trap path**, which is what
the C does and what `kayfabe-vmm-kvm`'s memory plane already implements. Do **not** reach for
userfaultfd, `mprotect`, permanently-read-only regions, or any other page-protection scheme for
the GSP path.

This is now backed by measurement rather than preference
(`../reference/uffd_isolate_kvm_study.md`): the C **never blocked a write anywhere** — uffd has
zero implementation there, its "demand-fault" path is dead code, and its actual TOCTOU defence is
a **copy-once snapshot** (audit item P2-2, `C: src/qemu/virtio_nvgpu.c:626-663`). That design
ships, is audited twice, and runs 22 real GPU apps at host parity. Combined with
`gl11_region_arguments.md` and §3.4 above answering **NO** to the lock question for the whole boot
path, the position is: **traps + copy-once is the proven design; the region lock is a capability
held for a case we have not met.**

### 2. Port the proven behaviour; deviate only where the C is demonstrably wrong

The C is a **working implementation on real hardware** — that is evidence no amount of reasoning
replaces. The default is therefore to reproduce its behaviour, and every deviation must name the
defect it is correcting. The deviations this plan is authorised to make, each with its evidence:

| deviate | because | evidence |
|---|---|---|
| the reset/latch chain | four disagreeing reset sites; a teardown STARTCPU misclassified as a re-acquire re-latches `bootargs_dumped`/`q_ready` and wedges the next life | `[measured]` bench, and `msgqRxLink`'s `-7` has exactly one cause (§ above) |
| RPC element parsing | parses arbitrary guest RAM as RPC elements and echoes `NV_OK`; unguarded `% q_msgcount` is a SIGFPE | `[measured]`, guest-reachable by reloading its own driver |
| msgq addressing | the region is `NV_MEMORY_NONCONTIGUOUS` and self-describing via a page table in its own first page; the C addresses it **linearly** and works only because the allocation happened to be contiguous | `[src]` |
| the version key | ~15 hard-coded offsets and no version key; the element layout genuinely changed in **`(595.84, 610.43.02]`** — 575/580/590/595 are all on the 48-byte side | `[src@580]` + `[src@610]` D1/D3 |
| the `elemCount` bound | at 580 the guest's copy loop is unbounded on a field **we** write, and the staging buffer holds exactly 16 elements — `elemCount > 16` memcpys past a `portMemAllocNonPaged` allocation, hitting the live `msgq` metadata first | `[src@580]` §4.6 — a **safety** deviation, and the only one in this table that is not about correctness |
| `rpc.length = 36` | header is 32; the same file uses 32 in two other places | `[src]` |

| the sequence-number reset | preserving unconditionally hands a *reloaded* driver a `seqNum` **greater** than its own, and its receive path has no recovery branch for `>` at all — only for `<` | `[src]` `ogkm: message_queue_cpu.c:768-782, 836`; discriminator = the queue region's identity (`kgspDestruct` frees the memdesc, an idle release does not). ★ UNMEASURED — plan item O3, falsified or confirmed at S7 |

**Anything not in that table is ported, not redesigned.** If the build turns up a further defect,
add a row with its evidence — do not deviate silently, and do not deviate because a different
shape would be tidier. "It's cleaner" is inadmissible here for the same reason it is inadmissible
in the MUST-DIFFER ledger (§ trace format): without a guest-visible consequence and an independent
oracle, a deviation is an untested guess dressed as an improvement.

### Why this ordering is right

The C's bugs are **enumerable** — we found them by measurement and by reading, and each has a
citation. Its correct behaviour is **not** enumerable: it encodes hundreds of quirks nobody wrote
down, discovered over months against real silicon. Reproducing it and subtracting the known
defects keeps the quirks. Rewriting from the protocol keeps only what we thought to look for.

---

## 13. BUILD FINDINGS — S0–S5, 2026-07-27

Written by the build, against this plan. Corrections are in this section rather than edited
into the text above, so a reader who remembers the original claim learns it was wrong.

### 13.1 Errata in this plan

1. **§5's S4 row says `GET_GSP_STATIC_INFO (51)`. It is 65.** `ogkm: rpc_global_enums.h:75`
   (`X(GSP, GET_GSP_STATIC_INFO, 65)`); the two post-`INIT_DONE` RPCs at
   `ogkm: kernel_gsp.c:5152, 5159` are `SET_GUEST_SYSTEM_INFO` (1) and this one. §12's
   citation table is right; §5's parenthetical is not.
2. **§5's CI note is stale.** *"The mutation job's `-f` scope does not include the crate"*
   was true when written; `.github/workflows/ci.yml` now scopes `crates/*/src/**/*.rs`
   ("SCOPE = EVERY PRODUCTION CRATE"), so `kayfabe-gsp` is already in the denominator and
   S3 has nothing to add.
3. **§5 says "the generation-name grep gate applies". There is no such gate.** CLAUDE.md
   rule 1 describes one, and `ci.yml` does not implement it — the boundary, VMM-vocabulary,
   unsafe-surface, GPA-accessor and unsafe-containment gates exist; a
   `Ampere|Turing|Hopper|Blackwell|Ada|V5\d\d` grep does not. Worth adding; not added here,
   because `ci.yml` is not this work's to change.
4. **§4.2's check list is missing two codes.** `msgqRxLink` also returns **`-11`** (the
   backend read of the peer header failed) and **`-12`** (the backend write of the zeroed
   read pointer failed), and there is **no `-4`** (`ogkm: msgq.c:364-378, 436-441`).

### 13.2 Inferred claims this build settled

- **I4 (the mailbox pair) — settled by construction, and it was a real bug.** Keying the
  publish on `MAILBOX1` is not merely fragile: with the pair latched across a driver life,
  a new life's *first* mailbox write completes a pair whose other half belongs to the
  previous life, and the FSM publishes an address assembled from two lives. Found by the
  three-lifetimes test (two `E6`s, the first at a mixed address). The fix is a register
  *shadow* (what a read returns) separate from a *trigger* (both halves seen since the last
  publish or teardown).
- **I9 (`Running` implies not-inside-a-boot-poll) — no longer inferred.** With
  `MSGQ_FLAGS_SWAP_RX` the guest publishes its own status-queue consumption into the
  command queue's rx header, so *"the guest has consumed `GSP_INIT_DONE`"* is directly
  observable: its read pointer moves off zero after `msgqRxLink` zeroed it. The event gate
  now rests on that read instead of on the inference.
- **I1 (E3 idempotency), I8 (the region array's terminator)** — unchanged, still inferred.
  I8 is now moot in practice: the scan searches by id8 and treats a zero entry as *skip*.

### 13.3 O3 (the seqNum question) has an observable discriminator

O3 asked whether `rxSeqNum` resets on a true `rmmod`/`insmod`. It does — `GspMsgQueueInit`
zero-initialises it, reached from `kgspConstructEngine` and torn down in `kgspDestruct`,
i.e. module load and unload — while an idle release keeps `MESSAGE_QUEUE_INFO` alive. The
two are distinguishable **without** waiting for S7: `kgspDestruct` frees the shared memdesc,
so a new module load publishes a **different `sharedMemPhysAddr`**, and an idle release
publishes the same one (the C's own note at `C:3459-3470` records the reuse). The port
therefore preserves the sequence numbers across a re-acquire and restarts them at 0 when the
region changes. S7 remains the falsifier; the mechanism is now testable in-process, and is.

### 13.4 Additions to the §3.5 seam, and one subtraction

- Added `GspModel::is_swgen0_clear(value)`: transition E10 needs *"does this write clear the
  status-queue interrupt edge"*, which is a bit position (bit 6 on GA10x) and therefore
  cannot live in the logic crate. The plan's sketch omitted it.
- **Removed `status_queue_irq() -> IrqSpec`.** `IrqSpec` lives in `kayfabe-vmm`, and
  `kayfabe-arch` does not depend on it; giving it one is a lattice decision, not a side
  effect of this port. The FSM reports an abstract *"announce the status queue"* instead and
  the device shell, which already owns the VMM vocabulary, chooses the delivery.
- `decode_reg` takes a raw `u8` BAR index rather than a newtype: the repo has two
  (`kayfabe_vmm::BarId`, `kayfabe_trace::Bar`) and unifying them is the same lattice
  decision. **[open]** — worth settling before a third appears.
- `Arch::gsp()` returns `Option<&dyn GspModel>` with a provided `None`, so adding the seam
  required **zero edits** to any existing `impl Arch`. `None` is a loud
  `GspFault::NoGspModel`, never a defaulted register value.

### 13.5 The fourth axis: guest **OS**

Named by the owner while this was being built, and recorded in `kayfabe-gsp`'s crate docs:
the guest *operating system* is a separate axis from the guest *driver version*, and today
it lives nowhere. Almost nothing in the boot path is actually OS-shaped, because
`ogkm: src/nvidia/` is NVIDIA's OS-independent RM core and the per-OS layer sits above it.
Exactly three assumptions are ogkm-shaped — one queue pair per init-args struct (r535 also
declares a lockless pair), the `RMARGS` id8 as the way the init region is found, and the
falcon mailbox pair as the boot-args channel (already the *architecture* axis). Drift is
supported only across ogkm-like bootstrap sequences; anything else ends in a named refusal
(`QueueNotBound` for a guest that never binds, `GeometryRejected` for a different handshake).

---

## 14. ★★ 580 CORRECTION LOG — 2026-07-28

Written after vendoring `ogkm-580.159.04` (§0.1). Corrections are recorded here **and** at
their site, because a reader who remembers the old claim needs to learn it was version-bound
rather than simply find it silently changed. Everything below was re-read in the 580 tree
directly; where a relayed finding did not survive that re-read, it says so.

### 14.1 What changed, and where

| # | claim as it stood | what 580 says | site |
|---|---|---|---|
| 1 | element count derived from `rpc.length` (§1.1 item 5) | **read from `elemCount`@40**, and that read drives `msgqRxMarkConsumed` | §1.1-5, §4.4, §9 D3 |
| 2 | *(absent)* | ★★ **`elemCount > 16` corrupts the guest kernel heap** | **new §4.6** |
| 3 | MCTP/NVDM are "610+" constants | **no MCTP exists on the 580 GSP path at all** ⇒ `transport: None` | §1.2, §9 D2, §11-O4 |
| 4 | break interval `(570, 610]` | **`(595.84, 610.43.02]`** ⇒ predicate `major >= 610` | §4.3, §9 D1 |
| 5 | init-args may declare geometry | **4 fields at 580; geometry is compile-time** 48/4096/65536/4/12 | §1.3, §9 D8, §11-O8 |
| 6 | O7 RESOLVED: "not implemented, do not emit" | ★★ **fully implemented at 580**; 6-entry bootup allowlist vs 610's 8 different ones | §1.1-10, **§11.1** |
| 7 | init RPCs are boot step B6, inside bootstrap | **queued before bootstrap**, doorbell rung, `QueueState` still `Unbound` | **new §3.1a** |
| 8 | suspend sentinel tested with `&` | **exact equality** at 580 ⇒ write the value, never OR the bit | §1.2, §3.2 |
| 9 | *(see 14.6 — the relayed claim did not survive)* | | |

### 14.2 ★ Two consequences that are ours, not NVIDIA's

Both fall out of §3.1a and neither was in the plan or in the relayed findings:

1. **E8 fires twice on a healthy 580 boot.** The doorbell-while-`Unbound` refusal is correct
   as a *behaviour* (read no guest RAM) and wrong as a *classification*: at 580 it is normal
   protocol traffic, not the stale-binding attack GSP-D4 describes. It must not escalate, and
   the negative-trace test's exact-count arm has to be restated to distinguish "two expected
   pre-bind doorbells" from "one stale-binding refusal".
2. **E6 must drain the command queue after publishing.** At bind the guest's cmd `writePtr` is
   already **2**. A doorbell-only service path recovers only because the guest happens to send
   another command right after `INIT_DONE`; that is luck, not design.

### 14.3 What was checked and found UNCHANGED

Listed so the next reader does not re-verify them: the whole `msgq` layer (`msgq.c` differs
only in `#include` placement; `msgq.h` and `msgq_priv.h` byte-identical), `g_rpc-message-header.h`,
`libos_init_args.h`, `dev_gsp.h`, `dev_fb.h`'s WPR2 registers, `kernel_falcon_tu102.c`,
`rpc_common.c` (semantically), `_checkSum32`, function numbers 1/47/65/72/73/76 and events
0x1001/0x1003, the `NV_ASSERT(0)` bootup gate itself, `maxRpcSize`, the recursive-poll
prohibition, `kgspWaitForRmInitDone` polling `(GSP_INIT_DONE, 0)`, the four `kgspUnloadRm`
callers, `NV0000_ALLOC_PARAMETERS` (byte-identical, `NV_PROC_NAME_MAX_LENGTH` still `100U` —
which does **not** settle 575, so `alloc_param_size` keeps returning `None`).
★ **Paths and line numbers still drift** — §0.1 rules 2 and 3.

### 14.4 What is relayed, not verified here

The nine-tag probe that narrowed the break interval to `(595.84, 610.43.02]` — 575.64.05,
580.65.06, 580.173.02, 590.44.01, 590.48.01, 595.44.02, 595.84 — used trees that are **not**
vendored here. Only 580.159.04 and 610.43.02 were read directly. The two endpoints of the
interval are therefore firm on one side (580.159.04 is 48-byte, read) and relayed on the
other (595.84 is 48-byte, not read). A predicate of `major >= 610` is safe under either
reading, since it is the *610* boundary that is directly verified.

### 14.5 ★ The class fix, not just the instances

The instance was "O7 was answered from the wrong tree". The class is: **a versioned
specification was cited as if it were the specification.** §0.1 is the fix — every citation
carries its tag, an untagged claim is unverified, `[src]` unqualified means *checked at both*.
Two further things follow from it and are worth stating separately:

- The C artifact's design docs are **transcriptions**, and a transcription is not a tree.
  §9 D1 was `[inferred]` (I6) for exactly this reason and turned out to be right; §11-O7 was
  marked RESOLVED for the opposite reason and turned out to be backwards. The difference is
  not luck — it is that I6 *knew* it was second-hand and O7 did not.
- A version-split fact is a **seam**, and the plan's own premise (owner directive: the GSP
  layer must not be tied to a driver version) means the correct response is to record both
  answers, never to pick the one that matches the machine on the desk. Where a split has no
  clean seam it is named as such rather than smoothed — see the brief's closing section.

### 14.6 ⚠ One relayed finding did NOT survive the re-read — reported, not quietly fixed

The relayed finding said: *"580 has a GSP-resume handoff 610 never reads —
`NV_PGC6_BSI_SECURE_SCRATCH_14._BOOT_STAGE_3_HANDOFF == _VALUE_DONE` plus SEC2
`FALCON_MAILBOX0`, in `_kgspIsReloadCompleted`/`CORE_RESUME`."*

**The "610 never reads" half is wrong.** 610 has the identical `_kgspIsReloadCompleted`,
reading the identical register and field, at
`ogkm-610: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_falcon_tu102.c:441-452`, used
at `:471` inside `kgspExecuteCoreResume_TU102`. The mechanism is present at **both** tags.

**What is actually version-split is how it is reached**, and that is a more interesting fact
than the one claimed:

| | 580.159.04 | 610.43.02 |
|---|---|---|
| where the handoff wait lives | `kgspExecuteSequencerCommand_TU102`, `case GSP_SEQ_BUF_OPCODE_CORE_RESUME` (`ogkm-580: kernel_gsp_tu102.c:913-960`) | `kgspExecuteCoreResume_TU102` (`ogkm-610: kernel_gsp_falcon_tu102.c:455-…`) |
| how it is triggered | ★ **only** via `_kgspRpcRunCpuSequencer` ← `GSP_RUN_CPU_SEQUENCER` — **an event we would have to send** | locally, from `kernel_gsp_falcon_tu102.c:563` and `kernel_gsp_falcon_ga102.c:401` — **no RPC involved** |

⇒ the owner's standing requirement that **GPU restart (idle → back) must work without a
bolt-on** is affected, but not in the direction the finding stated. The 580 resume path is
*RPC-driven*, which means a faked GSP that never emits `GSP_RUN_CPU_SEQUENCER` cannot drive
it — whereas at 610 the same path needs nothing from us. Whether any restart scenario we
must support actually goes through `CORE_RESUME` is **§11-O7a**, and it was **not**
established here: nothing in the 580 tree was traced from a resume entry point down to a
`GSP_SEQ_BUF_OPCODE_CORE_RESUME` buffer, because the buffer's *contents* come from GSP-RM
firmware, which is not in the open tree. That is a genuine hole and it is named as one.

### 14.7 ★★ EXECUTION LOG — B1–B9 landed, 2026-07-28

`docs/design/gsp_580_correction_brief.md` items **B1–B9** are implemented and green
(`cargo test --workspace --no-fail-fast` both ways, `scripts/ci_gates.sh --all`). Every new
invariant was bite-checked by removing the fix and confirming red: **13 bites, 0
non-biters.** What follows is only what the execution *changed about this log*.

**B1 first, and it mattered exactly as predicted.** `tests/src/gspworld.rs::Guest::recv`
derived the element count for both profiles, so before it no 580 invariant could
fail-before-and-pass-after. With the oracle fixed (`elemCount` field at 580, derivation at
610), all four of B1/B2/B3's tests bite.

**§14.1 row 4 is no longer prose.** The predicate now exists:
`kayfabe_abi::versions::GspElementWire` (`Pre610` / `From610_43_02`) plus
`GspInitArgsWire` (`FourField` / `NineField`), selected by the existing `table_for`
"newest entry `<=` version" mechanism. `tests/src/gspworld.rs`'s `P580`/`P610` are now
**consumers** of that table rather than a second definition — a `Profile` carries only a
name and a `DriverVersion`. Before this, `ElementLayout::new` had no caller outside test
code and *no production path selected a layout at all*.

**§14.2 gains a third consequence, and it is a latent bug the brief did not predict.**
B4's drain-on-publish surfaced it immediately: `publish` installed
`cmd: RxCursor { read_ptr: 0 }` on **every** bind. That is right for the status queue —
`msgqRxLink` assigns `rxReadPtr = 0` — but the command queue is the mirror image: nothing
resets the guest's tx `writePtr` short of `msgqTxCreate`, which runs only from
`_gspMsgQueueInit` at module load (`ogkm-580: message_queue_cpu.c:155-161`). So an
idle-release re-acquire rebound against a producer sitting at 4 and a consumer restarted at
0, re-read four already-answered commands, and refused `SeqNumGap` forever — the `>` case
with no recovery branch (`ogkm-580: :699-714`). Fixed by carrying the command ring's
position beside `cmd_seq`, restarting both exactly when the region identity changes. **One
rule: the command stream's position and its sequence are the same instance's state.** It was
invisible before only because no test rang a doorbell after a re-acquire.

**§14.2 item 1 is resolved as `Transition::E12`.** A doorbell while unbound splits on
`phase == Halted` — the teardown-reached phase, i.e. the measured stale-binding case
(`docs/reference/mode2_bench_lifecycle.md` §4) — which keeps `GspFault::QueueNotBound`.
Every other unbound phase is the healthy 580 pre-bootstrap doorbell and returns `Ok(E12)`.
**Both arms still read zero guest RAM**; only the classification differs, which was the
whole point. The split is on observed state, not on driver identity.

**B2's bound is real but its reachability is subtler than the brief said.** The brief ranked
it first while noting it is "currently unreachable through `encode_message`". More precisely:
`elements = ceil(msgLen / element_size)` and `max = element_size_max / element_size`, so the
length bound implies the count bound **iff `element_size` divides `element_size_max`** —
true on the bench (4096 | 65536), false in general. Since `element_size` is the guest's own
published `msgSize` and `msgqRxLink` accepts any value at or above `MSGQ_MSG_SIZE_MIN`
(`ogkm-580: msgq.c:340-343`), the non-dividing case is input the crate can be handed, and the
failing-before test is written on it. Do not "simplify" the check away on the grounds that
the bench geometry makes it redundant.

**The receive side is where GSP-S1 is unconditionally live**, because there the count is
guest-written rather than derived: `service_command_queue` range-checks the declared
`elemCount` against the staging bound, then cross-checks it against the derivation
(`GspFault::ElementCountMismatch`), then falls through to the existing availability break.
Range before mismatch before availability — a count above the staging bound can never become
valid however much the ring later fills, so it outranks "producer not finished".

**§14.4's uncertainty about the MCTP words is closed.** The brief said the decomposition had
not been re-derived and might not hold. It holds:
`mctpCreateTransportHeader(som=1, eom=1, 0, 0, 0)` =
`REF_NUM(MCTP_HEADER_VERSION 3:0, 1) | REF_NUM(EOM 30:30, 1) | REF_NUM(SOM 31:31, 1)` =
**`0xC000_0001`**, and `mctpCreateNvdmHeader(NVDM_TYPE_RM_RPC)` =
`REF_DEF(TYPE 6:0, VENDOR_PCI=0x7e) | REF_DEF(VENDOR_ID 23:8, NV=0x10de) |
REF_NUM(NVDM_TYPE 31:24, 0x25)` = **`0x2510_DE7E`**
(`ogkm-610: src/nvidia/arch/nvalloc/common/inc/mctp_format.h:39-58, 79-95, 108-120`,
`.../nvdm_format.h:61`). The table carries the *derivation* as well as the constant.

★ It also carries **validated masks** — `0x0000_000F` on the MCTP word and `0x00FF_FF00` on
the NVDM word — because the receiver reads only those two bit fields
(`ogkm-610: message_queue_cpu.c:735-762`). The mock previously compared the whole words,
i.e. it was **stricter than the driver**, and a test written against it would have asserted a
rejection the guest does not perform. The masks make that structural rather than a comment.

**B9 changed no production behaviour, as the brief said it would not.** The deliverable is a
pin plus a deliberately-wrong second register model that OR-s the suspend sentinel onto the
mailbox shadow; the pin catches it, and the same test shows a 610-shaped `& sentinel` poll
would be satisfied by the wrong model — which is why the difference is invisible to anyone
testing only the mask.

**Not reached: B8, B10, B11.** B8 (version-keyed bootup allowlist) needs
`GSP_RUN_CPU_SEQUENCER`, `UCODE_LIBOS_PRINT`, `GSP_LOCKDOWN_NOTICE`,
`GSP_POST_NOCAT_RECORD` and `OS_ERROR_LOG` added to `FunctionCodes` — ids nothing currently
consumes, which is against this crate's own "each needs a consumer first" rule; the
implementation stays correct-because-narrower (`InitDone` only, a strict subset of both
tags' lists) meanwhile. B10 is §11-O7a and remains open by design. B11 (retagging the 121
bare `ogkm:` citations plus a CI grep gate) is untouched except at the sites this work
edited; it should be raised as its own task, together with the `ci.yml` ownership question
§13.1 item 3 records.

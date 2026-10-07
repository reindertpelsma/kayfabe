# The surface we present — every RPC, register and channel operation, and the plan for it

> ⊘ **DATA SUPERSEDED by `THE_SURFACE_v3.md` (2026-09-20, which says so itself); note added
> 2026-09-26.** Its per-row *data* describes the pre-v3 tree at `w749-fable-legb`. Its KEEP/DELETE
> *plans* were kept by v3 (per `THE_SURFACE_v3.md`).

**STATUS: LIVE, 2026-09-20 (w815). Companion to `THE_ARCHITECTURE_v2.md`.**

⊘ Every row below was read out of the tree at `w749-fable-legb`, with `file:line`. Where the
survey could not establish something it says so — **an unverified row is marked ⚠, never
filled in**. This document is the inventory the v2 rewrite is scoped against: the **plan**
column is the decision, and `[DELETE]` means the mechanism goes away, not that the behaviour
does.

---

## 1. GSP RPCs

The wire table is generated from `rpc_global_enums.h` (`kayfabe-abi/src/generated/rpc.rs`);
classification is `FunctionCodes::classify` (`kayfabe-gsp/src/rpc.rs:209`); the one dispatch is
`translate()` (`kayfabe-rmrpc/src/lib.rs:1073`).

### 1.1 The object-model RPCs — **KEEP, unchanged**

`GSP_RM_ALLOC` (0x67), `GSP_RM_CONTROL` (0x4c), `FREE` (0x0a), `DUP_OBJECT` (0x15).

These four are the whole guest→us object protocol: the guest tells us what it allocated, what
it freed, what it aliased, and what it wants done. We decode them into `RmEvent`s and keep a
graph of the guest's objects. **Plan: unchanged in v2.** This is the control plane, it is
version-sensitive, it is heavily tested, and none of v2's deletions touch it. An unmapped alloc
class is refused by name (`UnmappedAllocClass`) and that stays — a class we do not model is a
thing we must not pretend to have created.

### 1.2 `UPDATE_BAR_PDE` (0x46) — **KEEP, but it is the BAR2 root**

Inert to the object model; acted on at `kayfabe-device/src/bar2.rs:299`, where it latches the
root PDE that BAR2 translations are rooted at, and replies `NV_OK` with an empty body.
**Plan: keep.** BAR2 is an instance-block window and the guest legitimately re-roots it. ⚠ This
is one of the few places v2 still walks page tables *to answer a read*, rather than to produce
a diff — see §3.4.

### 1.3 `SET_GUEST_SYSTEM_INFO` (0x01) / `_EXT` (0x40) — **KEEP**

The version handshake. We answer with the version we agreed to speak; the `_EXT` form is
answered `NV_OK` with an empty body. The guest's firmware string is latched read-only. **Plan:
keep** — this is where the support matrix is enforced, and it must stay strict.

### 1.4 `GET_GSP_STATIC_INFO` (0x41) — **KEEP, forged by construction**

We synthesise an entire `GspStaticConfigInfo` from the chip and driver tables
(`staticinfo.rs:183`). There is no host value to forward: on real hardware this comes from GSP
firmware we are replacing. **Plan: keep forged.** ⚠ This is the single largest fabricated
structure we hand the guest and it is derived per-die; it is the first thing to re-derive when
adding a chip.

### 1.5 The inert set — **KEEP as inert, they are load-bearing**

`INIT_GSP_TRACE_CRASH_BUFFER` (0xe4), `UNLOADING_GUEST_DRIVER` (0x2f), `GSP_SET_SYSTEM_INFO`
(0x48), `SET_REGISTRY` (0x49), `ECC_NOTIFIER_WRITE_ACK` (0xca).

Accepted, no state moves. ⊘ Two of these matter more than "inert" suggests:
`UNLOADING_GUEST_DRIVER` **requires a reply** — a missing one hangs `rmmod` — and the two
`NoReply` ones must **not** be answered, because an unsolicited reply desynchronises the queue.
**Plan: keep, with the reply disposition as the thing under test.**

### 1.6 Events we post — **KEEP**

`EVENT_GSP_INIT_DONE` (0x1001), `EVENT_POST_EVENT` (0x1003), `EVENT_RC_TRIGGERED` (0x1004).
We send these; a guest-sent one is refused `EventFromGuest`. **Plan: keep.** The refusal
direction is a real boundary — the guest must not be able to inject its own completions.

### 1.7 Everything else — **refused by name, and that is the design**

`Other(code)` ⇒ `UnknownFunction { code }` — *replied to, never dropped*. Includes ids present
in the generated table with no dispatch arm: `NOP`, `ALLOC_ROOT`, `ALLOC_MEMORY`,
**`MAP_MEMORY_DMA` (0x0e)**, `UNMAP_MEMORY_DMA` (0x0f), `EVENT_GSP_RUN_CPU_SEQUENCER`.

★★★ **`MAP_MEMORY_DMA` is the important one.** It is a HAL stub on GSP-client parts, so the
guest never sends it — which is why `[measured w811]` a whole boot carries **zero**
`SET_PAGE_DIRECTORY` events and a client-allocated VA space is never declared to us. **Plan:
this is open problem #3 in `THE_ARCHITECTURE_v2.md` §7.** v2 needs a root to seed a walk and
this RPC will not provide it.

---

## 2. RM control commands

### 2.1 Page-directory controls — **[DELETE the concept, keep the decode]**

| cmd | name | today |
|---|---|---|
| `0x00801813` | `DMA_SET_PAGE_DIRECTORY` | → `RmEvent::SetPageDir` |
| `0x90f10106` | `VASPACE_COPY_SERVER_RESERVED_PDES` | → `RmEvent::SetPageDir` |
| `0x20800a9f` | `GMMU_COPY_..._PDES_TO_SERVER` | same params as above |
| `0x00801814` | `DMA_UNSET_PAGE_DIRECTORY` | refused `PageDirControlNotModelled` |

These tell us a VA space's page-directory base. **Plan: keep decoding them, but demote what
the answer is used for.** In v2 a root is needed **only to seed a walk** — never as an address
space's identity. **[ROT]** today `Pdb` is both, and `unwrap_or(Pdb(0))` at eight sites makes
`Pdb(0)` mean *"declared at offset 0"* **and** *"not declared"*; that collision cost four
wrong diagnoses in one session. ⚠ Note both arms refuse a **sysmem** root today
(`SetPageDirRootAperture` / `PublishedPdesRootAperture`) even though a real GA106 was measured
with one — that refusal is on the v2 review list.

### 2.2 `GPU_PROMOTE_CTX` (0x2080012b) — **KEEP**

The guest tells us where it placed a context buffer. Decoded, ≤16 entries, `entryCount > 16`
refused. **Plan: keep.** ⚠ The C rounded every promote-derived mapping **up to 64 KiB**
(`asize = (size + 0xffff) & ~0xffffull`); this port binds at the declared length and that
difference produced sub-page holes. Under v2 the mapping comes from the **walk**, not from the
promote, so this becomes an observation rather than a mapping source — which dissolves the
rounding question instead of answering it.

### 2.3 Channel controls served by `ObjectPolicy` — **KEEP**

`GPFIFO_SCHEDULE` (0xa06f0103 / 0xa06c0101), `BIND` (0xa06f0104), `SET_CTXSW_PREEMPTION_MODE`
(0x20801210), `MC_SERVICE_INTERRUPTS` (0x20801702), `GET_MMU_FAULT_INFO` (0x906f0106),
`PREEMPT` (0xa06c0105).

The channel lifecycle: schedule it, bind it to an engine, preempt it, ask why it faulted.
**Plan: keep all.** ⊘ `GET_MMU_FAULT_INFO` is **relayed to the host channel verbatim, never
synthesised**, and that becomes *more* important in v2: if the GPU faults because our diff was
late, the guest must receive the hardware's own answer.

### 2.4 The 47 forged tables — **KEEP forged, but they are a per-die liability**

`FIFO_GET_DEVICE_INFO_TABLE`, `INTERNAL_GPU_GET_CHIP_INFO`, `CE_GET_CE_PCE_MASK`,
`FB_GET_INFO_V2`, `GR` static caps, the cudart-init group, and ~40 more
(`kayfabe-device/src/inittables.rs:1085`). Each is answered with a struct built from our chip
and driver tables; a `paramsSize` mismatch is refused.

**Plan: keep, and derive per die rather than maintain per die.** ⚠ This is the largest
correctness surface we own and the one with the worst oracle: `[the C artifact]` 11 of 56
captured rows had **`dlen = 0`** and *every* empty row checked against real hardware was
**contradicted**. `CE_GET_FAULT_METHOD_BUFFER_SIZE` decoded from an empty row as **0** where a
real GA106 returns **20480** — and RM DMAs fault records into a buffer of that size, so
trusting it was a buffer overrun with a hardware writer.

### 2.5 Input-only controls — **KEEP**

`NV2081_BINAPI`, `DEBUG_SET_EXCEPTION_MASK`, `SET_TIMESLICE`, two CUDA-limit controls. The ack
is the whole verb; we echo the guest's own bytes behind a size check. **Plan: keep.**

### 2.6 Denied by capability — **KEEP DENIED**

`IMPORT_MEM`, `DISABLE_IMPORTERS`, `NVLINK_GET_PLATFORM_INFO` (fabric management);
`GPU_EXEC_REG_OPS`, `NVB0CC_EXEC_REG_OPS` (raw register access); `REPORT_NON_REPLAYABLE_FAULT`;
the SM-debugger trapping set; `ALLOC_PMA_STREAM` (performance counters).

**Plan: keep denied, permanently.** These are the guest asking for a capability the host
process does not have and must not acquire. ⚠ This is a *capability allowlist* (160 rows), not
a served set: admitted-but-undecoded still refuses `GspRuleControlUnserviced`. ⚠ The survey did
not transcribe all 160 rows — that is a mechanical dump if the full admitted set is ever needed.

---

## 3. MMIO registers

⚠ **GA106 only.** The chip table holds exactly one profile; `ad10x`, `gh100`, `gb20x` exist in
`kayfabe-chips` and are not in it. ⚠ The BAR0 aperture is 16 MiB and ~87% of its pages hold no
register at all — "everything else ⇒ Unclaimed" is the complete statement for the remainder.

### 3.1 Boot constants — **KEEP**

`NV_PMC_BOOT_0/1/42`, `NV_PTIMER_TIME_PRIV_LEVEL_MASK`, `NV_USABLE_FB_SIZE_IN_MB`,
`NV_XVE_LINK_CAPABILITIES`. Constants the driver reads to identify the part. **Plan: keep,
per-die.** ⊘ One is an advertised fiction — the access-counter notify buffer size carries an
entry count only.

### 3.2 The GSP boot FSM — **KEEP**

`GFW_BOOT_PROGRESS/PLM`, the `PGSP`/`PSEC` falcon registers (CPUCTL, HWCFG2, DMATRFCMD,
MAILBOX0/1, IRQSTAT/MASK/DEST/SCLR), `NV_PRISCV_*`, `WPR2_ADDR_LO/HI`.

This is the fake GSP's entire visible boot: the driver pokes `STARTCPU`, watches progress reach
`0xFF`, reads mailboxes for boot args, and finds WPR2 where it expects. **Plan: keep.** ⊘ WPR2
state only resets on a full QEMU restart, which is why each clean run needs a fresh boot.

### 3.3 Queue doorbells and the interrupt tree — **KEEP**

`NV_PGSP_QUEUE_HEAD(i)` (`0x110C00 + i*8`) is the command doorbell: posted to a worker, no
state touched inline. `CPU_INTR_LEAF(i)` / `EN_SET` / `EN_CLEAR`, `CPU_INTR_TOP*`, and
`CPU_INTR_LEAF_TRIGGER` (write-only, latches a vector and raises the CPU interrupt).

**Plan: keep.** ⊘ `[measured w684]` 40 of 44 completions were never announced (`no_engine`);
the interrupt **arming** model is the live design question here, not the register decode.

### 3.4 The MMU invalidate trio — **[DELETE most of what it triggers]**

`MMU_INVALIDATE_PDB` (`0xB830A0`) and `_UPPER_PDB` latch; `MMU_INVALIDATE` (`0xB830B0`)
**triggers** a publication, and the guest **spin-polls** its TRIGGER bit for completion.

**Plan: keep the register, delete the publication behind it.** In v2 an invalidate is a hint
that the guest's tables changed — a good moment to walk and diff — and **nothing waits on it**
(§1: the guest already tolerates undefined-until-complete). ⊘ Today this register is the entry
point to the epoch/dirty-gate/publication machinery that §3 of the architecture deletes. ⚠ RM
cannot express a narrow invalidate (ALL_VA is hard-coded) and its own timeout is **4 s**, after
which it proceeds with stale TLBs — so the guest is *already* designed for us to be slow here.

### 3.5 The framebuffer windows — **[DELETE]**

`NV_PBUS_BAR0_WINDOW` (`0x1700`) plus the PRAMIN moving window (`0x700000–0x7FFFFF`); BAR1
(`bus_bar::FB`) and BAR2 (`bus_bar::INST`), both GMMU-translated on access; `FbTrapPolicy`.

**Plan: BAR1/BAR2 become device views of the one store and stop being translated windows;
PRAMIN stays as the bring-up aperture it is, not a running path.** ⊘ `[measured]` vidmem CPU
reads are **~48 MiB/s flat** — any design that routes real traffic through these is slow by
construction, which is precisely why the store exists.

### 3.6 The doorbell — **KEEP, it is the hot path**

`NV_VIRTUAL_FUNCTION_DOORBELL` (BAR0 `0x00BB0090`). Rung off-lock; drives channel submission.
`NV_VIRTUAL_FUNCTION_TIME_0/1` beside it are the PTIMER: read-only, **writes refused by name**
(the guest reads the host's counter and may not move it). **Plan: keep all three exactly.**
⊘ §41 bounds what this trap may do: update a queue, wake a worker, one dword for a passthrough
doorbell, or a synchronous read-register write. Nothing else, ever.

---

## 4. Channel / pushbuffer operations

Decoder: `Ga10xPushbuffer` (`kayfabe-chips/src/ga10x.rs:1595`); vocabulary `PushMethod`
(`kayfabe-arch/src/lib.rs:949`); consumer `apply_pushbuffer` (`kayfabe-fwd/src/lib.rs:8721`).

### 4.1 EMULATED — we implement the effect

| method | addr | why it is ours |
|---|---|---|
| `SET_OBJECT` | `0x0000` | pure bookkeeping — binds a subchannel class, clears its slots |
| host-FIFO semaphore run (`SEM_ADDR_LO/HI`, `SEM_PAYLOAD_LO/HI`, `SEM_EXECUTE`) | `0x5C–0x6C` | **RELEASE only**; acquires and reduction decode `Opaque` |
| `MEM_OP_A..D` TLB invalidate | `0x28…` | recorded and censused; `PDB_ALL` and non-invalidate ops ⇒ `Opaque` |
| `LAUNCH_DMA` with `DATA_TRANSFER_TYPE_NONE` | `0x0300` | a release carrying no copy — there is no transfer to forward |

**Plan: keep all four emulated.** Each is either bookkeeping or a completion, never a byte
movement. ⊘ In v2 the TLB-invalidate record becomes a *walk trigger* and loses its publication
side effect (§3.4).

### 4.2 TRANSLATED — operands rewritten, the GPU moves the bytes

`LAUNCH_DMA` (`0x0300`) is the only true member, fed by fourteen latched operand slots:
`OFFSET_IN/OUT_UPPER/LOWER`, `LINE_LENGTH_IN`, `LINE_COUNT`, `SET_SRC/DST_PHYS_MODE`,
`SET_REMAP_CONST_A/B`, `SET_REMAP_COMPONENTS`, `SET_SEMAPHORE_A/B/PAYLOAD/_UPPER`.

Today we re-resolve `src`/`dst` through the issuing channel's **address table** and partition
into spans. **Plan: [DELETE the translation].** Under v2 the guest's VA *is* the host's VA
(route K) and the store is identity-mapped, so there is nothing to rewrite — the GPU resolves
the operands through the page tables we applied, and faults if we were wrong. ⊘ This deletes
`TableOperands`, the span partitioning, the operand gate, and `CeExecutor::Ours` with it.

⚠ `SET_SRC/DST_PHYS_MODE` is the one that must **not** be waved through: `PHYSICAL` bypasses
the MMU, and a physical-mode copy passed straight to hardware is an escape class. It stays
inspected — that is exactly what a **Translated** channel is for, and why Passthrough is not
allowed to carry it.

⊘ `SET_REMAP_COMPONENTS` is **required, never defaulted** — an unlatched remap refuses the
launch. Keep that; a defaulted remap silently writes the wrong bytes.

### 4.3 REFUSED — the fault tripwire

`GP100_UVM_SW` `FAULT_CANCEL_A/B/C` (`0x0104/0108/010C`) and `CLEAR_FAULTED_A/B`
(`0x0110/0114`) abort the whole parse with `UvmFaultMethodWithoutFaultDelivery`.
`SET_OBJECT`/`NO_OPERATION` on the same class pass through as routine.

**Plan: keep refusing — and revisit when v2 lands.** These say the guest is doing fault
*recovery*, which presumes a fault-delivery mechanism we do not implement. ★ v2 deliberately
lets the GPU fault (§3), so fault delivery moves from "not modelled" to "on the critical
path", and this refusal is the marker for that work.

### 4.4 PASSED THROUGH

Everything else ⇒ `PushMethod::Opaque`, counted, untouched. Non-`Incrementing` method forms
(`NON_INC`, `ONE_INC`, `IMMD_DATA`) are deliberately not latched. **Plan: keep.** ⊘ Parsing —
not placement — is what is forbidden on a passthrough ring; this is the rule in code.

---

## 5. ⚠ Where this inventory is knowingly incomplete

1. The 160-row capability allowlist is not transcribed (gates admission, not service).
2. Registers are **GA106 only**; three other chip modules exist and were not enumerated.
3. There is no enumerable table of *unhandled* BAR0 offsets — ~87% of pages are Unclaimed.
4. `kayfabe-rt/src/ceutils.rs` holds a **second** `PushMethod` consumer; it was confirmed to
   use the same enum but **not** audited for additional method addresses.
5. `kayfabe-isolate-host/src/rm.rs` *emits* CE pushbuffers using the same constants; classified
   as a producer, not audited as a decoder.
6. Roughly a dozen `kayfabe-device` modules were not read in depth; two of them gate on
   `RpcFunction::RmControl` and may reference control ids not listed here.

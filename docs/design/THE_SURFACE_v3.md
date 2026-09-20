# The surface, at constant level — every RPC, class, control, register and method we touch

**STATUS: LIVE, 2026-09-20 (w821).** Companion to `THE_ARCHITECTURE_v3.md`. Supersedes the
*data* in `THE_SURFACE_v2.md`; that document's KEEP/DELETE *plans* still stand and are kept.

⊘ **Written for a reader who does not know NVIDIA's constants.** Every entry gives the constant
name, its numeric value, one plain sentence saying what the thing *is*, and what we do with it.
⚠ Every name and number is cited to `research_clones/ogkm` or to this tree. Anything a survey
could not establish is marked **[UNVERIFIED]** rather than guessed.

---

## 0. The four planes, and why they are different shapes

A guest driver reaches a real GPU through exactly four kinds of traffic. kayfabe intercepts all
four, and each has a different shape because each has a different **trust** and **ordering**
requirement — not because of taste.

| plane | transport | who can drive it | our posture |
|---|---|---|---|
| **GSP RPC** | a command/status ring in guest memory, kicked by a BAR0 register write | guest **root** (RM) | we **are** the GSP: we answer, forge or forward |
| **RM control** | carried *inside* GSP RPC `GSP_RM_CONTROL`, and via ioctl on the host side | guest **root** (RM) | serve, forward, refuse — by command number |
| **BAR0 MMIO** | reads untrapped from a DRAM shadow; writes trap to the vCPU | guest **root**, except the usermode window | allowlist (§41), shadow write-through |
| **channel / pushbuffer** | a GPFIFO ring in guest memory, kicked by a doorbell write | ★ **unprivileged guest userspace** | passthrough / translated / emulated |

★★★ **The last row is the one that restructured the design.** The doorbell is the only surface
an unprivileged guest process touches directly, which makes it adversarial to the guest's own
root — see `THE_ARCHITECTURE_v3.md` §2.1 and `THE_CONSTRAINTS.md` §47.

⇒ Read the four sections below in that order: they go from most-privileged transport to
least-privileged, and the machinery gets **weaker and more paranoid** as it goes down.

---

## 1. GSP RPC — the plane we answer as the firmware

### 1.1 What this plane is, for a reader who does not know NVIDIA's constants

Modern NVIDIA GPUs run a firmware processor called the **GSP** (GPU System Processor). On such a
part the host driver does not talk to the hardware directly for most things — it sends
**messages** to the GSP, and the GSP's own copy of the resource manager (**GSP-RM**) does the
privileged work. kayfabe's central trick is that **we are the GSP**: the guest's stock, unpatched
driver sends us these messages believing we are firmware.

The transport is a pair of rings in **guest sysmem**, not a register interface:

- a **command queue** the guest writes and we read,
- a **status queue** we write and the guest reads,
- each ring made of fixed-size **elements**, each element carrying a checksum and a sequence
  number, each message an `rpc_message_header_v03_00` envelope plus a payload.

The guest tells us where those rings are once, at boot, through
`MESSAGE_QUEUE_INIT_ARGUMENTS` (`kayfabe-gsp/src/boot.rs:100`, read at `:1185`). After that the
**only** hardware event in the whole plane is a single register write that says *"the command
queue moved"* — see §3.

### 1.2 The structures on the wire

| structure | what it is | kayfabe `[CODE]` | ogkm `[ogkm]` | axis |
|---|---|---|---|---|
| `msgqTxHeader` | the ring's producer header: version, element size/count, **`write_ptr`**, flags, and the offsets of the rx header and the element array | `TxHeader`, `kayfabe-gsp/src/ring.rs:131-148` (8×u32, 32 bytes, `write_ptr` at +16) | `msgq_priv.h:37` | stable |
| per-element header | prefixes every element: a checksum and a sequence number, plus transport framing | `ElementLayout`, `kayfabe-gsp/src/element.rs:112-233` | `message_queue_priv.h:43` (580) / `:52` (610) | ⊘ **`Dg` — BREAKS** |
| `rpc_message_header_v03_00` | the message envelope inside an element: `header_version, signature, length, function, rpc_result, rpc_result_private, sequence` — 32 bytes | `RpcEnvelope`, `kayfabe-abi/src/view.rs:558-572` | — | `Dg` (additive) |
| queue pair geometry | binds the guest's command queue and our status queue, including the **swapped rx** convention where each side publishes its consumption into the *other* queue's header | `MsgqGeometry`, `kayfabe-gsp/src/ring.rs:407-570` | `MSGQ_FLAGS_SWAP_RX` | stable |

★★★ **The element header is the single best worked example of the `Dg` axis in this tree**, and
it is why §0.2 says descriptors rather than branches:

| | 580.159.04 | 610.43.02 |
|---|---|---|
| header size | **48 bytes** | **16 bytes** |
| fields | `authTagBuffer[16]`, `aadBuffer[16]`, `checkSum@32`, `seqNum@36`, `elemCount@40` | `mctpHeader`, `nvdmHeader`, `checkSum@8`, `seqNum@12` |

⇒ Size *and* every field offset changed between two versions we must both support, and the
`elemCount` field **disappeared**. kayfabe absorbs this as data — `ElementLayout` carries
`elem_count_off: Option<usize>` and a `TransportHdr` that is `None` at 580 and `Mctp{..}` at 610
— not as a version branch. **[PROPOSE]** every other axis-carrying fact should take this shape.

### 1.3 The functions

Dispatch: `FunctionCodes::classify()`, `kayfabe-gsp/src/rpc.rs:211-232`. The number↔name mapping
is **generated**, not transcribed: `kayfabe-abi/src/generated/rpc.rs`.

★ `[ogkm]` The enum (`src/nvidia/inc/kernel/vgpu/rpc_global_enums.h`) is **byte-identical between
580.159.04 and 610.43.02** for every id we use. 610 only *appends*. ⇒ On this plane the
*numbers* are `Dg`-stable and only the *framing* moves.

| # | `NV_VGPU_MSG_FUNCTION_` | what it is, in plain language | we do | `[CODE]` |
|---|---|---|---|---|
| 1 | `SET_GUEST_SYSTEM_INFO` | first message after boot: the guest states its driver version, page size and OS, and expects the agreed protocol version back | **SERVE** | `kayfabe-device/src/guestsysinfo.rs:112` |
| 64 | `SET_GUEST_SYSTEM_INFO_EXT` | follow-on carrying extra system strings; the real driver returns its status, so it must succeed | **SERVE** (`NV_OK`, empty body) | `guestsysinfo.rs:122` |
| 65 | `GET_GSP_STATIC_INFO` | the guest asks for the big fixed-fact table about this GPU — memory layout, engine list, BAR sizes — before anything else can initialise | **SERVE** from our device-data model | `kayfabe-device/src/staticinfo.rs:182` |
| 70 | `UPDATE_BAR_PDE` | the guest hands us the physical root of its **BAR2** page directory. ★ Only the firmware's directory is hardware-rooted, so this is the only fact we get about that tree | **SERVE** (latch it) | `kayfabe-device/src/bar2.rs:299` |
| 72 | `GSP_SET_SYSTEM_INFO` | a system-description blob (PCI topology and friends), sent early, fire-and-forget | **IGNORE** — ⊘ *no reply at all*; echoing would desync the guest's sequence counter | `kayfabe-gsp/src/rpc.rs:318` |
| 73 | `SET_REGISTRY` | the guest uploads its entire registry / module-parameter table | **IGNORE** (no reply) | same |
| 10 | `FREE` | destroy one object handle, or a whole client namespace — RM's teardown stream | **SERVE** from the local object graph | `kayfabe-rmrpc/src/lib.rs:1095`, `:1955` |
| 21 | `DUP_OBJECT` | alias one client's object into another client's handle namespace. ⊘ It **aliases, refcount++; it does not copy** — this is the normal UVM flow | **SERVE** (a graph edge) | `lib.rs:1097`, `:1918` |
| 103 | `GSP_RM_ALLOC` | the generic *"create an RM object of this class"* envelope. Nearly every object the guest makes — clients, devices, VA spaces, channels, memory, contexts — rides inside | **MIXED, default REFUSE** | `lib.rs:1229-1276` |
| 76 | `GSP_RM_CONTROL` | the generic *"run this RM control command"* envelope. Almost every specific query or operation rides inside as a nested `NVxxxx_CTRL_CMD_*` id + params — see §2 | **MIXED, default REFUSE** | `lib.rs:1521-1572` |
| 71 | `CONTINUATION_RECORD` | ⊘ not a function: an overflow *fragment* of a message too large for one element. Only `GSP_RM_CONTROL` fragments this way | folded into reassembly; standalone is refused | `kayfabe-rmrpc/src/reasm.rs:250` |
| 47 | `UNLOADING_GUEST_DRIVER` | the guest is about to `rmmod`. ★ **Synchronous** — the guest blocks until we reply | **SERVE (ack-only)**, reply mandatory | `kayfabe-device/src/inert.rs:119` |
| 202 | `ECC_NOTIFIER_WRITE_ACK` | the guest's own acknowledgement that it finished writing the ECC notifier buffer | **IGNORE** | `rpc.rs:320` |
| 228 | `INIT_GSP_TRACE_CRASH_BUFFER` | the guest hands us a `{physical address, size}` for a buffer the firmware is meant to fill with crash-trace records | **SERVE (ack, no-op)** — we write no trace records, so it stays zeroed | `inert.rs:119` |
| 0x1001 | `GSP_INIT_DONE` *(event)* | we tell the guest *"boot finished"*; its boot poll waits on this | **outbound only** | `kayfabe-gsp` boot FSM |
| 0x1003 | `POST_EVENT` *(event)* | the generic asynchronous-completion carrier firmware uses to notify the guest | **outbound only** | — |
| 0x1004 | `RC_TRIGGERED` *(event)* | we tell the guest a channel or engine was torn down by robust-channel fault recovery | **outbound only** | `kayfabe-gsp/src/fault.rs` |
| — | everything else (~210 ids) | any other function or event id | ⊘ **REFUSE by name** — `NV_ERR_NOT_SUPPORTED` (`0x56`), zeroed body | `lib.rs:1146` |

★★★ **The safe default is a named refusal, never an echo.** A chain of `CommandPolicy` links is
tried in order (`PolicyChain::respond`, `kayfabe-gsp/src/boot.rs:596`); whatever no link answers
falls through to `GspFsm::answer`'s refusal (`boot.rs:1719-1763`). ⊘ An `EchoOk` policy exists
(`boot.rs:541`) but is a **differential-test fixture reproducing the old C's behaviour** and is
**not installed in the production chain**. This matters because `0x56` is a status the guest
driver forgives — so a generic-ack fallback would let a wrong configuration run on, undetected,
which is precisely the failure this project has measured repeatedly.

### 1.4 ★ The reply is always built from local state

⊘ **No RPC is a verbatim forward.** Applying an `RmEvent` to the local object graph
(`Gpu::apply`, `kayfabe-core/src/gpu.rs:5582`) can *latch* a pending host-side spawn, resolved
immediately after by `materialize_pending` (`:5591`). Real host ioctls happen **asynchronously as
a side effect**, decoupled from the synchronous RPC reply.

⚠ **That decoupling is load-bearing and was paid for**: it previously ran inline under the rank-0
lock and **crashed QEMU** (`gpu.rs:3462-3471`).

⚠ **[UNVERIFIED]** whether any individual modelled `GSP_RM_CONTROL` / `GSP_RM_ALLOC` arm blocks
on a host call before constructing its reply. The top-level dispatch does not; the ~15 per-control
handler files were not exhaustively checked. ⇒ This is the same question as the synchronous-verb
list in §2.4, and it is the one the owner is waiting on.

<!--SECTION2-->
---

## 3. BAR0 MMIO — reads from DRAM, writes to the vCPU

### 3.1 The asymmetry, and why it exists

BAR0 is the GPU's register aperture: ~16 MiB of address space where each 4-byte slot is a
control register. A normal emulated device traps **both** reads and writes, and pays a VM exit
for each. kayfabe does not:

- ⊘ **Reads are not trapped.** They are served out of **ordinary DRAM** the guest reads directly,
  with **no exit and no code running**.
- ★ **Writes trap** into `nvkvm_trap_bar0_write` on the vCPU thread.

The reason is the trap contract: a vCPU inside an MMIO exit is **not preemptible**, so every
trapped read is a stall charged to a core of the customer's VM. A register the guest *polls* —
and driver init polls several thousand times — would be thousands of stalls for a value that
did not change. ⇒ Put the value in memory and let the guest read it at DRAM speed.

`[MEASURED]` **3 572 of 4 096 BAR0 pages are backable in 12 contiguous runs**
(`kayfabe-device/src/plane.rs:2560`), against a read census of **241 722** BAR0 reads.

### 3.2 The shadow — two structures, and they are not the same thing

⚠ This is the part most often got wrong: *"the shadow"* names **two** objects.

| | `RegPlane` | `ShadowSink` / `ShadowSegment` |
|---|---|---|
| what | the **model**: the source of truth that answers a *trapped* access | the **DRAM the guest actually reads** |
| `[CODE]` | `kayfabe-device/src/plane.rs:1223` | `kayfabe-qemu-raw/src/shim_unsafe.rs:1656`, `:1626` |
| shape | `ChipProfile` + `GspModel` + ROM image + two `RankedMutex` state cells + a dead-page bitmap | a `Vec` of `{off, len, base}` contiguous byte runs, linearly scanned |
| guest touches it? | ⊘ never | ★ directly, with no exit |

⊘ **It is not a flat offset-indexed array and not a page hash map.** It is a small vector of
merged contiguous runs — which is why a write to an offset no segment covers is **silently
dropped** rather than faulting (`shim_unsafe.rs:1666-1677`).

**How a page becomes readable-from-DRAM.** `bar0_backable_runs()` (`plane.rs:2579`) sweeps the
whole aperture in 4 KiB units and admits a page only if **every dword in it** is answerable
without running code: a provably dead dword, a boot-register constant, a ROM byte, a CPU
interrupt-tree leaf, the BAR0-window latch, an MMU-invalidate register, or something the GSP
model can project. Adjacent admitted pages merge into runs.

★ **The content is computed, never loaded from a capture.** `bar0_shadow_fill` (`plane.rs:2743`)
fills each run from the *same* per-arm sources the trapped read path uses, and only then does
the shim attach that memory (`:1713`). ⇒ There is no capture table to expire as a vendor
regression, which is the failure mode §0.3 forbids.

**Write-through is a single funnel.** `RegPlane::shadow_write` (`plane.rs:3144`) is the only
producer, called from the BAR0-window latch, the MMU-invalidate latches and trigger, the
interrupt-tree republish, and the GSP register-group republish (`:3933-4013`).

⚠ **[UNVERIFIED — a discrepancy worth someone's attention, not yet a confirmed defect.]** Under
`feature = "host-isolates"` a second mechanism, `Regs::back_bar0_dead_runs`
(`kayfabe-qemu-raw/src/shim.rs:17527`), independently installs a read-only KVM memslot over
`bar0_backable_runs()` backed by a memfd its own comments describe as *"never written… zero
resident pages"* — **without calling `bar0_shadow_fill` first**. Its prose still carries the
pre-w563 *dead-pages-only* meaning while calling the post-w563 *widened* range, which also
includes non-zero boot registers and ROM. The test at
`crates/kayfabe-device/tests/bar0_backable_pages.rs:143` explicitly records that w563 changed the
invariant from *"the run answers zero"* to *"the run answers what `bar0_shadow_fill` computes."*
⇒ Either the two paths are gated to disjoint builds, or one of them serves zeros where the guest
expects `NV_PMC_BOOT_0`. **The call ordering crosses the FFI boundary into the C shim, which is
not in this repository, so it could not be settled from here.**

### 3.3 The trapped-write register table

`RegPlane::write`, `plane.rs:6473`. First match wins; the order is itself load-bearing (the
doorbell is classified **before the plane lock is taken**, because its port may block).

Offsets are GA10x/GA106. **Axis** column: what moves the offset or the meaning.

| offset | register (ogkm swref) | what it is, in plain language | what we do | axis |
|---|---|---|---|---|
| `0x001700` | `NV_PBUS_BAR0_WINDOW` | a relocatable 1 MiB window latch: bits pick *which* megabyte of framebuffer the `PRAMIN` aperture below currently points at | store the raw word verbatim (a read-modify-write must see back exactly what it wrote); write through to shadow | A |
| `0xBB0090` | `NV_VIRTUAL_FUNCTION_DOORBELL` | ★ **the bell**: a work-submission token saying a command ring has new entries | §2 of Part 1 — the security-critical path. Classified with **no lock held** | ⊘ **die** |
| `0xB830A0` | `..._PRIV_MMU_INVALIDATE_PDB` | low/aperture half of *which page directory to invalidate* | latch only, not acted on | A |
| `0xB830A4` | `..._PRIV_MMU_INVALIDATE_UPPER_PDB` | high half of that address | latch only | A |
| `0xB830B0` | `..._PRIV_MMU_INVALIDATE` | ★ the TLB-invalidate **fire** register: the guest writes scope + `TRIGGER=1`, then **spin-polls this same register until `TRIGGER` reads back 0** | decode scope, publish address-space changes **here** (the guest is quiesced — this is the exact GPU boundary), then clear `TRIGGER` **unconditionally**. ⚠ A stuck `TRIGGER` hangs or resets the guest | A |
| `0xB81000`+`i*4` | `..._PRIV_CPU_INTR_LEAF(i)` | pending-interrupt bitmap, 32 vectors per leaf, write-1-to-clear | update tree, republish the whole group to the shadow, maybe raise an interrupt. No plane lock | A |
| `0xB81200/1400` | `..._LEAF_EN_SET/_CLEAR(i)` | enable mask for leaf `i`, set and clear ports | same arm | A |
| `0xB81600/1608/1610` | `..._TOP(i)`, `_TOP_EN_SET/_CLEAR` | top-level pending summary and its enables | same arm | A |
| `0xB81640` | `..._LEAF_TRIGGER` | software interrupt injection (write-only) | same arm. ⚠ tested **first**, to avoid aliasing with the `TOP_EN_CLEAR` array bound | A |
| `0x110C00`+`i*8` | `NV_PGSP_QUEUE_HEAD(i)` | ★★★ the **GSP command doorbell**: "a new command is in the ring". This is the whole hardware surface of the RPC plane in §1 | **post and return** — increment a counter, no lock, no inline drain. `[MEASURED]` this fixed a **1.79 s vCPU stall** (`plane.rs:5196`). Drained later by a worker, one command at a time | A, ⚠ see below |
| `0xBB0080/0084` | `NV_VIRTUAL_FUNCTION_TIME_0/1` | the GPU's free-running nanosecond timer, read by the driver for timeouts | ⊘ **refused by name** and counted — accepting a write would put guest and host GPU on different timebases | die |
| `0x118234` | `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05(0)` | firmware-boot-progress scratch byte the driver polls for *"GFW boot complete"* | drive the boot FSM; republish group | A |
| `0x118128` | `..._GROUP_05_PRIV_LEVEL_MASK` | privilege mask that must be fully lowered before the guest trusts the progress byte | same | A |
| `0x110100` | `NV_PFALCON_FALCON_CPUCTL` (GSP) | ★ the bit that **starts the GSP microcontroller running** | boot FSM; `is_startcpu()` classifies | A |
| `0x1100F4` | `..._HWCFG2` (GSP) | reports/gates whether the falcon's RISC-V core is enabled | boot FSM | A |
| `0x110118` | `..._DMATRFCMD` (GSP) | the ucode-load DMA command/status register | boot FSM | A |
| `0x110040/0044` | `..._MAILBOX0/1` (GSP) | the two halves of the **LibOS boot-args physical address** | boot FSM | A |
| `0x110008/0018/001C` | `..._IRQSTAT/IRQMASK/IRQDEST` (GSP) | falcon interrupt status, mask, routing | boot FSM | A |
| `0x110004` | `..._IRQSCLR` (GSP) | write-1-to-clear falcon interrupt status. ★ the guest's ISR clears here **before** draining our status queue | boot FSM; `is_swgen0_clear()` classifies | A |
| `0x111388` | `NV_PRISCV_RISCV_CPUCTL` | RISC-V core control; the `ACTIVE`/`HALTED` bits the guest polls for liveness | boot FSM | A |
| `0x111528/152C` | `NV_PRISCV_RISCV_IRQMASK/IRQDEST` | RISC-V interrupt mask and destination, ANDed for stall attribution | boot FSM | A |
| `0x840100` | `NV_PFALCON_FALCON_CPUCTL` (**SEC2**) | start register of the security co-processor that loads protected firmware | boot FSM | A |
| `0x840040` | `..._MAILBOX0` (SEC2) | the Booter's argument — distinguishes a **Load** from an **Unload** of the protected region | boot FSM; `is_booter_unload()` | A |
| `0x840118` | `..._DMATRFCMD` (SEC2) | SEC2's ucode-load DMA command/status | boot FSM | A |
| `0x1FA824/828` | `NV_PFB_PRI_MMU_WPR2_ADDR_LO/HI` | base of **WPR2**, the write-protected region where GPU firmware lives. ★ `HI != 0` is the guest's *"is WPR2 already up"* test | boot FSM | ⊘⊘ **die × `Dg`** — see below |

⊘ **Everything not in this table is silently dropped** — counted in `unclaimed_writes`, returning
`WriteOutcome::nothing()` (`plane.rs:6733-6737`). That includes writes to boot-register constants
and to the ROM window, which are read-only from the guest's side and have no write arm at all.

### 3.4 ★★★ WPR2 is the worked example of two axes crossing in one register

`gb20x.rs:196-227`: on Blackwell the register **moved** to `0x0088A824/828`
(`NV_HUBMMU_PRI_MMU_WPR2_ADDR_LO/HI`) for 610-series drivers — **while still answering the legacy
`0x1FA824/828` pair for 580-series drivers.** Both offsets decode to the same internal variant.

⇒ **One register, two axes at once**: the **die** moved it, and the **guest driver version**
decides which address that guest will use. Neither axis alone explains the behaviour, and a
design that modelled only *"per-die offsets"* would answer the wrong address for a 580 guest on a
Blackwell part. ★ This is the concrete argument for §0.2's claim that axes are not independent
and a per-die table is not sufficient on its own.

⚠ Related, and **[UNVERIFIED]**: `NV_PGSP_QUEUE_HEAD` was once believed Turing-only; `gh100.rs:51`
records the correction that **GH100's driver does write it**. ⇒ An "architecture X doesn't use
this register" belief is exactly the kind of claim that expires.

### 3.5 Whole apertures, not registers

| range | name | treatment |
|---|---|---|
| `0x700000`–`0x7FFFFF` | **PRAMIN**, 1 MiB | one **aperture**, classified before the register decode. Every dword resolves through the `0x1700` latch: `fb_addr = (base << 16) + window_off`, then reads/writes the guest framebuffer store, or **refuses by name** if untranslatable. ⚠ A bring-up aperture, not a running path |
| `0x300000`–`0x3FFFFF` | **PROM/VBIOS**, 1 MiB | one range **on read only**, served from a byte image. ⊘ **No write arm exists at all** — writes fall to the dropped path |
| `0xB80000`–`0xB8FFFF` | **usermode / VF window**, 64 KiB | ⊘ **not** a pass-through aperture — decoded **register by register** (doorbell, invalidate trio, interrupt tree, PTIMER). Any other offset inside it is dropped |
| MSI-X table / PBA | — | ⊘ **not in this tree at all.** Only a vector *count* lives here; the table and delivery are QEMU's native model on the C side. **[UNVERIFIED]** whether it sits inside BAR0's range |
| BAR1 / BAR2 | framebuffer / instance windows | separate PCI BARs, ⊘ **never trapped** — ensuring that is a requirement (Part 1 §2) |
---

## 4. Channels and pushbuffers — the plane unprivileged guest code drives

### 4.1 What a channel is, for a reader who does not know NVIDIA's constants

A **channel** is one command stream to the GPU. It has three pieces of memory:

- a **pushbuffer** — a byte array of commands ("methods"), written by the guest;
- a **GPFIFO ring** — an array of 8-byte entries, each pointing at a run of pushbuffer;
- **USERD** — a small per-channel block holding the ring cursors `GP_PUT` (producer, written by
  the guest) and `GP_GET` (consumer, advanced by the hardware).

To submit work the guest appends a GPFIFO entry, bumps `GP_PUT`, and **rings the doorbell**. The
hardware's front-end then reads `GP_PUT`, fetches entries, and executes the methods.

★★★ **`GP_PUT` lives in USERD, which is mapped to the owning process and nobody else.** That is
the whole reason a hostile doorbell is harmless on real hardware and dangerous for us: hardware
re-reads memory only the owner can write, and we *parse the owner's pushbuffer*. See Part 1 §2.1.

### 4.2 The three kinds — and an honest statement of what is built

| kind | contract | what it means |
|---|---|---|
| **Passthrough** | `RingAndReturn` — *"no inspection and no work"*; **may** run on the vCPU | the guest's ring is in real GPU memory; hardware fetches it directly. We never read a byte of it |
| **Translated** | `ScheduleAndReturn`; must **not** run on the vCPU | every entry's operands rewritten into our own VA space, then submitted on our host channel. **The GPU moves the bytes** |
| **Emulated** | `ScheduleAndReturn`; must **not** run on the vCPU | we implement the function in the VMM |

`[CODE]` `GuestChannelKind`, `kayfabe-core/src/channel_kind.rs:96-158`; contracts at `:286-315`.

⊘⊘⊘ **The classifier is a two-way switch, not a three-way one, and `Translated` is never
constructed.** `ProcBoundary::channel_kind()` (`kayfabe-core/src/project.rs:308-316`) is:

```rust
if self.anchor == SYSTEM_ANCHOR { Emulated } else { Passthrough }
```

⇒ A channel is **Emulated** iff its client namespace is the one reserved *system* component (the
guest **kernel**'s own client — UVM, the CeUtils scrubber), and **Passthrough** otherwise.
★ **No source location tests a class id to decide the kind.** The class id only matters later,
for method decode and engine routing.

⊘ `GuestChannelKind::Translated` is a fully-typed third variant with complete contracts and
exhaustive-match coverage everywhere — and **zero construction sites outside tests**
(`channel_kind.rs:355` is a test fixture array). The design doc that introduced it
(`the_three_channel_kinds.md`, STATUS: DESIGN, w803) says *"not yet implemented"*, and that is
still true. ⇒ **Real kernel-CE work the design says should be `Translated` is today classified
`Emulated`** and driven through the CPU/host-CE forwarding path in §4.5. ⚠ This is the gap behind
the scrub defect in Part 5.

| class (name + hex) | what it is | kind today | axis |
|---|---|---|---|
| `AMPERE_CHANNEL_GPFIFO_A` `0xC56F` — user CUDA context | a userspace channel a CUDA process owns | **Passthrough** | A |
| `AMPERE_CHANNEL_GPFIFO_A` `0xC56F` — guest-kernel / UVM / CeUtils | a channel the guest **kernel driver** owns | **Emulated** *(design target: Translated)* | A |
| `AMPERE_DMA_COPY_B` `0xC7B5` | the **copy engine** class object — does DMA copies and memsets | inherits its channel's kind | A |
| `AMPERE_COMPUTE_B` `0xC7C0` | the **compute** class object — kernel launches | inherits; routed `HostGr`, real hardware runs it, no in-process executor | A |
| `GP100_UVM_SW` `0xC076` | ⊘ **not a real engine.** A software placeholder so UVM's channel allocation succeeds, and a subchannel to hold fault-cancel methods | rides its (kernel) channel → Emulated | stable |
| `VOLTA_CHANNEL_GPFIFO_A` `0xC36F`, `TURING_…`, `HOPPER_…` `0xC86F`, `BLACKWELL_…` `0xC96F` | the same channel object, per architecture | kind is per-namespace, not per-class | ⊘ **A** |
| `AMPERE_USERMODE_A` `0xC361` | ⊘ not a channel — the **BAR window object** that exposes the doorbell register itself (Part 1 §2.1) | n/a | A |

### 4.3 The GPFIFO entry — 8 bytes

`[CODE]` `GpfifoEntry`, `kayfabe-abi/src/submit.rs:1836`; decode at `:1852`.
`[ogkm]` `NVC56F_GP_ENTRY0/1`, `clc56f.h:262-284`.

| field | bits | meaning |
|---|---|---|
| `GP_ENTRY0_FETCH` | `0:0` | unconditional / conditional fetch (**not modelled**) |
| `GP_ENTRY0_GET` | `31:2` | pushbuffer address, bits `31:2` |
| `GP_ENTRY1_GET_HI` | `7:0` | address bits `39:32` |
| `GP_ENTRY1_LEVEL` | `9:9` | main run, or a **subroutine** call |
| `GP_ENTRY1_LENGTH` | `30:10` | length **in dwords** |
| `GP_ENTRY1_SYNC` | `31:31` | `PROCEED`, or `WAIT` — the front-end must drain before fetching |

⚠ `LENGTH == 0` is **not an empty run** — it is a *control* entry whose low byte is an opcode, so
`GET_HI` must not be read there. We refuse it rather than decode it (`submit.rs:1822`).

★ **Axis note:** `NVC36F` (Volta) and `NVC56F` (Ampere) differ in exactly two bits across the
whole entry — C56F **dropped** `PRIV` (`8:8`) and **gained** `INVAL_SCOPE` in `MEM_OP_A`. ⇒ This
encoding is far more `A`-stable than the register offsets in §3. We model neither bit.

### 4.4 The method header — one dword

`[CODE]` `method_header_decode`, `submit.rs:2196`. `[ogkm]` `NVC56F_DMA_*`, `clc56f.h:286-360`
— bit-identical to `NVC36F_DMA_*`.

| field | bits | meaning |
|---|---|---|
| `SEC_OP` | `31:29` | which of 8 encodings this dword is |
| `METHOD_COUNT` / `IMMD_DATA` | `28:16` | dwords that follow, **or** the immediate payload itself |
| `METHOD_SUBCHANNEL` | `15:13` | which of 8 bound engine objects this addresses |
| `METHOD_ADDRESS` | `11:0` | ★ **dword-indexed** — multiply by 4 for a byte offset |

`SEC_OP` values: `GRP0_USE_TERT`(0), `INC_METHOD`(1), `GRP2_USE_TERT`(2), `NON_INC_METHOD`(3),
`IMMD_DATA_METHOD`(4), `ONE_INC`(5), `RESERVED6`(6), `END_PB_SEGMENT`(7).

★★★ **Sizing and decoding are deliberately different jobs.** We *size* (skip correctly past) all
eight forms, so the stream never desynchronises; we *decode the payload of* only incrementing
runs. ⊘ `RESERVED6` has no defined format, so it decodes to nothing rather than to a guess.
**[PROPOSE]** this split — *always know the length, decode only what you model* — is the right
shape for every parser in this system, and it is what lets an unmodelled method be harmless.

### 4.5 Every pushbuffer method we recognise

| method (ogkm constant) | offset | engine | what it does | what we do |
|---|---|---|---|---|
| `NVC56F_SET_OBJECT` | `0x000` | host FIFO | binds a class to a subchannel — this is how the stream says *"the next methods are for the copy engine"* | bind/clear per-subchannel decode state (`ga10x.rs:1762`) |
| `NVC56F_SEM_ADDR_LO/_HI`, `SEM_PAYLOAD_LO/_HI`, `SEM_EXECUTE` | `0x5C-0x6C` | host FIFO | the **channel's own semaphore release**: write a payload to a GPU address when this point in the stream retires. This is how the guest learns work finished | decoded only for `OPERATION == RELEASE`; observed on the completion queue (`ga10x.rs:1160`) |
| `NVC56F_MEM_OP_A/B/C/D` | `0x28-0x34` | host FIFO | the **TLB invalidate** transport, targeted at one page directory | decoded for `MMU_TLB_INVALIDATE`/`_TARGETED` with `PDB_ONE`; counted, and the membar honoured as a hard barrier. ⊘ **no real invalidate is issued** (`ga10x.rs:1558`) |
| `NVC7B5_OFFSET_IN_UPPER/_LOWER`, `OFFSET_OUT_UPPER/_LOWER` | `0x400-0x40C` | CE | source and destination addresses of a copy | latched (`ga10x.rs:1195`) |
| `NVC7B5_LINE_LENGTH_IN`, `LINE_COUNT` | `0x418,0x41C` | CE | how many bytes per line, and how many lines | latched |
| `NVC7B5_SET_SRC/DST_PHYS_MODE_TARGET` | `0x260,0x264` | CE | which physical aperture (coherent sysmem / non-coherent sysmem / vidmem) a **physical** operand names | latched; read only when that operand is physical |
| `NVC7B5_SET_REMAP_CONST_A/_B`, `SET_REMAP_COMPONENTS` | `0x700-0x708` | CE | the constant-fill / component-swizzle parameters — this is how a **memset** is expressed | latched, consumed by remap decode |
| `NVC7B5_SET_SEMAPHORE_A/B`, `_PAYLOAD[_UPPER]` | `0x240-0x24C` | CE | the copy engine's **own** completion semaphore address and payload | latched, consumed inside `LAUNCH_DMA` |
| `NVC7B5_LAUNCH_DMA` | `0x300` | CE | ★ **starts the copy.** Carries no operands — every operand was latched by the methods above; the method word itself is all flags | the whole execute decision (`ga10x.rs:1453`, `kayfabe-fwd/src/lib.rs:8724`). See below |
| `NVC076_SET_OBJECT`, `NO_OPERATION` | `0x000,0x100` | UVM SW | routine bind / no-op on the software class | ordinary; explicitly excluded from the tripwire |
| `NVC076_FAULT_CANCEL_A/B/C`, `CLEAR_FAULTED_A/B` | `0x104-0x114` | UVM SW | UVM cancelling or clearing a **real MMU fault** | ⊘ **refuses the whole parse** — `UvmFaultMethodWithoutFaultDelivery` (`lib.rs:8919`). We have no fault delivery, so seeing one means the assumption behind admitting this class broke. ★ A tripwire, not an error |
| `NVC36F_NON_STALL_INTERRUPT` | `0x020` | host FIFO | ⊘ **not recognised anywhere.** `grep` across all crates returns nothing | falls through to opaque |
| everything else | — | — | WFI, YIELD, NOP, CRC check, sub-device masks, legacy forms, non-incrementing and immediate runs | **sized, not decoded** — counted as opaque, stream stays in sync |

⚠ **`NON_STALL_INTERRUPT` being unrecognised is a real gap against Part 1 §5.** That section says
the interrupt is *requested in the pushbuffer*, and this is the method that requests it. We
currently do not see it. ⊘ Today that is survivable only because the channels we care about
**poll** — UVM spins, and the scrubber's blocking waits loop on the semaphore word — but it is an
assumption about the guest, not a property of our design.

★ **Why `LAUNCH_DMA` is not in the single-method decoder:** a real copy is **five separate method
runs**, and `LAUNCH_DMA` carries none of the operands. Only the stateful walker that threads an
accumulator across the whole run can produce a copy fact (`ga10x.rs:1730`). **[PROPOSE]** this is
the general shape — *a method is not a unit of meaning; a run is* — and any redesign that decodes
methods individually will reintroduce this bug.

### 4.6 What happens when each kind's doorbell rings

**Passthrough.** The host channel was adopted at **channel allocation**, never lazily
(`plan_channel_birth`, `lib.rs:5576`) — ★ mandatory, because RM **zeroes a caller-supplied USERD
at alloc time**, so adopting at first doorbell would wipe the cursor that just rang. The doorbell
resolves the guest token to the already-adopted host token, runs the ring gate, and writes the
real host doorbell. ⊘ **No pushbuffer byte is ever read** — `device.rs:4110` says so by name, and
`ring_content_is_forwardable` returns `false` for this kind. Completions are hardware's own
writes to guest memory; we observe nothing and forge nothing.

**Emulated.** Born lazily at first doorbell if needed. Handed to a worker — must not run on the
vCPU. If the bound engine routes to `CpuCe`, the ring is parsed: read the guest's pushbuffer,
decode runs into methods, then for each `LAUNCH_DMA` partition the copy into spans by address
representability, classify each as a **data copy** (a VA operand) or a **page-table write** (a
physical operand — latched and applied to the *owning* process's VA space, which may not be the
issuer). Host-CE spans go to the real copy engine and **the real engine writes the guest's
completion**. If the engine routes `HostGr`, the doorbell is forwarded but ⊘ **the ring is never
parsed** — *"no executor IN THIS PROCESS runs GR work"* (`device.rs:8930`).

⊘⊘⊘ **The measured defect, in one line:** CPU-executed spans have **zero completion writers** —
`write_completion` has no call sites (`lib.rs:8862`), so a completion on that arm is **silently
dropped**. Combined with the scrub being classified `Emulated` rather than `Translated`, this is
the cross-client leak in Part 5.

**Translated.** ⊘ Design only; no doorbell reaches it. The intended path is in
`the_three_channel_kinds.md` §4: read USERD, advance `GP_GET`, bounded-copy the pushbuffer,
rewrite every operand aperture `PHYSICAL → VIRTUAL` into one static VA space, submit on our host
channel, and translate the completion **in address only** — the real engine writes the value. An
untranslatable operand is meant to be forwarded as a **deliberate fault at a named sentinel VA**,
never emulated and never silently dropped.
---

## 5. ⚠ Where this inventory is knowingly incomplete

★★★ **This section is the point of the document, not an apology.** An inventory whose gaps are
unmarked reads as complete, and this project's most expensive recurring failure is a plausible
value that was never measured. Everything below is a place a survey **stopped**, stated so a
reader can argue about it or go and close it.

### 5.1 Gaps in what we implement

| # | gap | why it matters | where |
|---|---|---|---|
| 1 | ⊘ **`Translated` is never constructed.** The kind exists, is fully typed, has contracts and exhaustive-match coverage, and **zero live construction sites** | Everything the design says should be *translated* is today *emulated* — including the scrub. This is the gap behind the live cross-client leak | §4.2 |
| 2 | ⊘ **CPU-executed copy spans have no completion writer.** `write_completion` has no call sites | A completion on that arm is **silently dropped**. A guest waiting on it waits forever or proceeds on stale data | §4.6 |
| 3 | ⊘ **`NVC36F_NON_STALL_INTERRUPT` is not recognised anywhere** | It is the method by which a guest *requests* the completion interrupt Part 1 §5 is built around. We survive only because the channels we care about **poll** — an assumption about the guest, not a property of our design | §4.5 |
| 4 | ⊘ **No fault delivery exists.** `NVC076_FAULT_CANCEL_*` refuses the whole parse | ★ Correctly built as a **tripwire** rather than an error: it fires exactly when the assumption behind admitting the UVM software class breaks | §4.5 |
| 5 | ⊘ **The TLB-invalidate method is decoded and counted, but no invalidate is issued** | Modelled structurally with nothing behind it | §4.5 |
| 6 | ⚠ **Two BAR0 shadow-install mechanisms, possibly overlapping**, one of which maps a never-written zero memfd over a range that now includes non-zero boot registers | If both are live, one serves zeros where the guest expects `NV_PMC_BOOT_0`. Could not be settled — **the call ordering crosses the FFI boundary into the C shim, which is not in this repository** | §3.2 |

### 5.2 Questions a survey could not answer

| # | question | why it was not closed |
|---|---|---|
| 7 | Does any individual `GSP_RM_CONTROL` / `GSP_RM_ALLOC` handler **block on a host call before replying**? | The top-level dispatch does not. The ~15 per-control handler files were not exhaustively checked. ★ **This is the same question as the synchronous-verb list, and it is the one the owner is waiting on** |
| 8 | Does the status (reply) queue have its own outbound doorbell, or is it purely guest-polled? | No explicit statement found; only its absence from the code read. Consistent with the C oracle's `IrqRaise == 1` with zero interrupt-clear writes, but **not proven** |
| 9 | Is there a top-level RPC function a stock guest sends at init that we fail to classify? | Searched; **none found. Absence not proven** |
| 10 | Does the doorbell token encoding really differ per Blackwell die group? | Carried in this project's notes; **not confirmed** against `gb20x` code. The doorbell *offset* is identical Volta→Blackwell |
| 11 | Where does the MSI-X table live, and is it inside BAR0? | ⊘ **No MSI-X code exists in this tree at all** — only a vector count. The table and delivery are QEMU's native model on the C side |

### 5.3 ⊘ Axis coverage — the honest statement

Everything in this part was surveyed against **one point of the lattice**:

> **GPU:** Ampere GA106 · **guest OS:** Linux · **guest driver:** 580/610 · **VMM:** QEMU

⇒ The **axis** column in each table says what *should* vary. It does **not** certify that the
entry has been exercised at a second point on that axis. Three specific consequences:

- ⚠ **Windows guest has zero coverage.** Every guest-side inference here is Linux-shaped until
  proven otherwise. The C artifact never ran one either.
- ⚠ **Turing is not covered and is not the easy end.** NVIDIA binds Turing to a *different
  page-table format family* from the one built here; spanning Turing→Blackwell needs four.
- ⚠ **The cross-axis cases are the dangerous ones.** §3.4's WPR2 register — moved by the **die**,
  with *which* address in play decided by the **guest driver version** — is the only one this
  survey found. ⊘ It is very unlikely to be the only one that exists.

### 5.4 What would close these

**[PROPOSE]**, in the order I would do them:

1. **#2 and #1 together** — they are one defect wearing two hats, and they are the live security
   bug. A completion that is dropped and a channel kind that was never built are both *"the scrub
   does not really happen."*
2. **#7, the synchronous-verb list** — mechanical, and it unblocks judging the whole trap path.
   Until it exists, *"no RM verb on a vCPU"* cannot be checked, only asserted.
3. **#6** — needs one person to read across the FFI boundary once. Cheap, and the failure mode is
   silent.
4. **#3** — cheap to decode, and it converts an assumption about guest behaviour into a fact we
   handle.
5. The axis gaps in §5.3 — ⚠ these are not a task, they are a **standing property** of the
   product, and they want a second point on the lattice in CI rather than a one-off audit.

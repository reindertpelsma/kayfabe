# The machine, as it is — and what I propose to change

**STATUS: LIVE, 2026-09-20 (w816).** Companion to `THE_ARCHITECTURE_v2.md` (the proposal) and
`THE_SURFACE_v2.md` (the per-item inventory). **This** document is the one an engineer should
attack: it describes the system as it actually runs, from a `file:line` survey, and marks every
place I propose to change it.

## 0. How to read this

Three markers, used consistently:

| marker | meaning |
|---|---|
| `[MEASURED]` | a number from a real boot, with the run that produced it |
| `[CODE]` | a fact read out of the tree at `w749-fable-legb`, with `file:line` |
| **[PROPOSE]** | my design opinion. Argue with these. |
| ⚠ **[UNVERIFIED]** | the survey could not establish it. Not filled in with a guess. |

⊘ Two contradictions found while writing this are left **in**, unresolved, because only one
side of each can be true and I could not determine which:

1. **The BQL.** `shim.rs` and `docs/design/blocking_and_completion_model.md:16` both state *"every
   guest MMIO write arrives with the QEMU BQL held."* But `qemu/hw/misc/nvkvm/nvkvm.c:1118` calls
   `memory_region_enable_lockless_io` on **every** region, the realize-time self-check *refuses
   the device* if the flag is missing (`nvkvm.c:1568`), and `nvkvm.c:392` states as measured fact:
   *"these callbacks run CONCURRENTLY on every vCPU with NO BQL."* ⚠ If the lockless path is
   live, every "the whole VM is frozen during a trap" argument in this tree is stale, and the
   shared-lock findings in §4 get much sharper.
2. **`Vas` is keyed by `(GpuId, ResourceKey)`** `[CODE gpu.rs:1761]`, but at least three doc
   comments still say `(GpuId, Pdb)` (`gpu.rs:1760`, `:250`, `:275`). ⚠ I did not determine
   whether any reachability argument still depends on the stale key.

⚠ And a third, which I fixed while writing: `ScratchpadArm::Off` is documented *"The default"*
while `scratchpad_from(None)` returns `Measure`. That stale line made one of my own surveys
report that the single store is off by default — i.e. that the central memory design does not
run. It is the **third** stale-default of the day, after `IsolatePlane::Stillborn` and
`KAYFABE_VAS_OWNER=isolate`.
## 1. Vocabulary — what the nouns actually are

⊘ Written for a reader who does not know NVIDIA's constant names. Every term below appears in
the rest of the document and in the code; this is the only place they are defined.

**GPU, host, guest.** The *host* is the machine with the real card. The *guest* is a VM running
a **stock, unmodified** NVIDIA driver. The guest believes it owns a GPU. It does not — it owns
our emulation of one, and real work is carried out on the host's card by unprivileged host
processes we control.

**RM (Resource Manager).** The core of NVIDIA's driver. Everything the driver does is
"allocate an object of class X under parent Y", "call control command Z on object W", "free
it". Objects form a tree rooted at a *client*. This object protocol is the control plane, and
it is what we emulate.

**Client / handle / object.** A *client* is a driver-level session (roughly: one open of
`/dev/nvidiactl`). A *handle* is a 32-bit number the guest picks to name an object **within**
its client. Handles are per-client, reusable after free, and therefore not identities — two
different objects can wear the same number at different times. This is why the code keeps
"incarnations" rather than trusting handle values.

**GSP (GPU System Processor).** On modern cards, a RISC-V core **on the GPU** runs a firmware
that does most of RM's privileged work. The host driver stops touching hardware directly and
instead sends the GSP messages over a queue in memory. ★ **We impersonate the GSP.** That is
the central trick: we do not emulate a GPU register-by-register, we answer the driver's
*messages* the way its firmware would.

**RPC.** One of those messages. Each has a numeric function id (e.g. `0x67` = "allocate an
object"). The guest writes a message into a ring buffer in its own memory and pokes a register;
we read it, act, and write a reply back into the ring.

**Control command.** A sub-kind of RPC: "run command `0x20800102` against object H, here are
the parameters." These are the bulk of the driver's vocabulary — hundreds of them, each with
its own parameter struct. Most are *queries* ("how much framebuffer is there?", "what are this
copy engine's capabilities?").

**BAR0 / BAR1 / BAR2.** PCI memory windows the card exposes to the CPU.
- **BAR0** is the register aperture — 16 MiB of control registers. A guest read or write here
  traps to us.
- **BAR1** is a window onto video memory, so the CPU can read/write VRAM.
- **BAR2** is a second window, used for *instance memory* (the structures describing channels).

**Framebuffer / VRAM / vidmem.** The card's own memory. A *framebuffer offset* is a byte offset
into it. ★ In our design guest vidmem is **one** big host allocation, and a guest framebuffer
offset is simply an offset into that allocation — the identity is the entire memory model.

**Sysmem.** Ordinary system RAM the GPU can also read, via an IOMMU-ish mapping. For us this
is the guest's RAM, which the hypervisor hands us as a shareable file descriptor (`memfd`).

**GPU virtual address (VA) and the page tables.** The GPU has its own MMU. Work submitted to it
uses *virtual* addresses, translated by page tables the driver builds in memory. A **PDB** (page
directory base) is the physical address of the root of one such tree — i.e. "which address
space". The GPU's TLB caches translations, so after changing tables the driver issues a **TLB
invalidate**.

**VA space.** One page-table tree = one address space. A guest process typically has one; the
kernel driver has others. Two channels sharing a VA space see identical translations.

**Channel / GPFIFO.** A *channel* is a submission queue to an engine. The driver writes command
packets into a **pushbuffer** in memory, then appends an entry to a ring called the **GPFIFO**
pointing at that pushbuffer. Hardware consumes the ring.

**USERD.** A small per-channel structure holding the ring cursors: `GP_PUT` (how far the driver
has filled) and `GP_GET` (how far hardware has consumed). The driver writes `GP_PUT`; hardware
writes `GP_GET`.

**Doorbell.** After updating `GP_PUT`, the driver writes one 32-bit word to a register to say
"there is new work on channel N". That write is a *doorbell*. It is the hottest trap in the
system — and on a real card it is a posted write that costs the driver almost nothing.

**Engines: CE and GR.**
- **CE (Copy Engine)** — a DMA engine. It moves bytes between addresses. `LAUNCH_DMA` is its
  "go" command; operands (source, destination, length) are loaded into it by preceding commands.
- **GR (Graphics/Compute)** — runs shaders/kernels. CUDA work lands here.

**Semaphore release.** How software learns an engine finished. The driver puts a "when done,
write value V to address A" command in the pushbuffer, then **spins** reading address A until it
sees V. ⊘ This matters enormously for us: the wait is the *guest's own* spin on *its own*
memory, so if the value appears the guest proceeds, and if it never appears the guest hangs. We
must never forge V for work that genuinely had to happen.

**Scrub.** Before RM hands a page of video memory to a different client, it clears it — with a
copy-engine fill, on a kernel-owned channel. This is what stops one process reading another's
old data. ★ The guest does this itself and orders it correctly; our only job is not to lie
about whether it happened.

**WPR2 (Write-Protected Region 2).** A region of VRAM the GSP firmware is loaded into, fenced
off by hardware. The driver checks registers to see whether it is already up. It only resets on
a full device reset, which is why each clean run needs a fresh VM boot.

**CeUtils.** RM's internal helper that owns the kernel-side copy-engine channels — the ones
used for scrubbing and other housekeeping. Its channels belong to no guest process.

**UVM (Unified Virtual Memory).** A second guest driver module (`nvidia-uvm`) providing managed
memory. It has its own ioctl surface and its own channels, and CUDA uses it heavily.

**Xid.** An NVIDIA error code reported by the driver when hardware faults. `Xid 31` is a GPU
memory-access fault — the engine touched an address the page tables did not map. ⊘ For us an
Xid on the host is a signal we got a mapping wrong.

---

## 2. Processes and threads

### 2.1 The processes

A full Mode-2 boot is **one QEMU process** with our Rust archive statically linked in as a QOM
device, plus **N+1 sandboxed child processes**.

| process | count / lifetime | opens `/dev/nvidia*`? | what it is for |
|---|---|---|---|
| **VMM (QEMU)** | 1 per VM | **No** `[CODE]` | owns the guest, KVM fd, all guest RAM, the MMIO traps, the pure core, the doorbell table, the BAR mirrors, and the isolate factory |
| **Scratchpad isolate** | 1 per (VM, GPU), spawned at **PCI realize** | **Yes** | holds the ONE reserved video-memory object, the walk kernel and (when armed) a CUDA context |
| **Per-proc isolate** | 1 per (guest process, GPU), **lazy** | **Yes** | that guest process's RM client, its channels, its VA spaces |

`[CODE]` Every `/dev/nvidiactl` open in the workspace is in `kayfabe-isolate-host` (the child
side) or the standalone ladder tool. **The VMM issues no RM ioctl.** It holds descriptors the
children hand *up* — sealed memfds and armed device nodes — which it only `mmap`s, never
`ioctl`s (`export.rs:31-42`).

★ **The "birth client B" is not a process.** It is an RM client handle plus two character-device
descriptors, minted **inside a per-proc isolate** (because RM stamps `pClient->ProcID` from the
creating task) and passed to the scratchpad by `SCM_RIGHTS` `[CODE child.rs:722-740]`.

**[PROPOSE]** Isolates stay. Their founding reason is **VA identity** — NVIDIA, and UVM
especially, treats host VA and guest VA as one address, so each guest process needs its own host
address space. Forgetting this is what let bug #14 (identical-VA collision between two guest
processes) back into the C. But under the single store they stop being *memory owners* and
become **address-space containers**: `Publish`, `PublishVidmem`, `JoinFbLeaf`, `AliasFbLeaf`,
`FbJoinPeek` and `ExportBacking` all delete, leaving `MapStoreSlice`, `AdoptVaSpace`, channel
birth, doorbell and control. ⚠ **[UNVERIFIED]** Route K already births channels in the
*scratchpad's* process — if that holds for UVM's channels too, VA identity has already moved and
this conclusion changes.

### 2.2 The sandbox — two stages, forced order

**Stage A, taken by the parent at `clone`** `[CODE spawn_unsafe.rs:1009]`:
`CLONE_NEWUSER|NEWPID|NEWNET|NEWIPC|NEWUTS|NEWNS`, raw `SYS_clone`, then rootless uid/gid maps
(`setgroups=deny` first, as the kernel requires). `execveat(AT_EMPTY_PATH)` on a **sealed
memfd**, so the image cannot be substituted. `PR_SET_NO_NEW_PRIVS` before exec.

**Stage B, taken by the child in its own `main`, before any worker thread exists**
`[CODE sandbox_unsafe.rs:770-891]` — and **the order is the security property**, which is why it
is one call and not three: nested user ns → `MS_REC|MS_PRIVATE /` → 256 KiB `tmpfs` root
(`nosuid,noexec`) → bind the two device nodes → `pivot_root(".",".")` + `umount2(MNT_DETACH)` →
**only now** open the `/dev` dirfd → remount root read-only → drop every capability and **read
the result back**, a single surviving bit being a refusal.

⚠ **Three things any security claim must not overstate:**
1. **seccomp is NOT installed** `[CODE lib.rs:57]`. The fixed fd numbers exist *so that* a future
   filter can hardcode them; that filter does not exist. Any document saying the isolate is
   syscall-filtered is wrong.
2. **The CUDA scratchpad has a full filesystem view** for the duration of its CUDA bring-up,
   which runs *before* `sandbox::enter` `[CODE child.rs:355-374]`. Constraint 20 must be read as
   *"it ends with the same reach"*, not *"it never had more"*.
3. That bring-up makes the child **multi-threaded** before `sandbox::enter`, and
   `unshare(CLONE_NEWUSER)` is refused to a multi-threaded process — so the scratchpad depends on
   the fallback arm under inherited `CAP_SYS_ADMIN`.

★ `[CODE isolate.rs:20-32]` fd 4 is **deliberately vacant**. It used to carry an `O_PATH` `/dev`
dirfd, and `openat(4, "../etc/shadow")` **opened** — `O_PATH` places no restriction on `..`.

### 2.3 The threads

**Exactly two production threads are created by kayfabe inside the VMM** `[CODE]`:

| thread | loop | waits on |
|---|---|---|
| `kayfabe-doorbell-publish` | `doorbell_publish_loop` | `Condvar` on the publication queue |
| `kayfabe-completion-observer` | `observer_loop` | `epoll_wait`, 250 ms tick, woken early by an eventfd |

Plus QEMU's own main and vCPU threads, on which our trap code runs.

Inside each isolate child: a main thread, one **control** thread blocked in `recv(2)` on a
datagram socket, and `--workers` (default **4**) pool threads each blocked reading its own
socket. ⚠ **None of the child's threads are named**, so `/proc/<pid>/task/*/comm` shows
`kayfabe-isolate` for all of them.

★ The control thread is separate for one reason: a cancel must be serviced **while every worker
is busy**, which is the only state in which a cancel is ever wanted. It signals with
`tgkill(SIGUSR1)` whose handler is installed with **no `SA_RESTART`** — *"the single most
consequential line in the file"* `[CODE signal_unsafe.rs:63]` — because that is what makes an
in-flight RM `ioctl` return `EINTR`.

⚠ **[UNVERIFIED]** `observer_loop` declares no `ThreadClass`, so it defaults to `Coordinator`,
under which long blocking sections are a reported violation — yet it deliberately performs
millisecond-scale budgeted drains. The publication worker *does* declare `Worker`. There is no
comment either way.

---

## 3. The vCPU trap path, step by step

### 3.1 How a guest access reaches Rust

```
KVM vmexit
  → nvkvm_trap_write()            qemu/hw/misc/nvkvm/nvkvm.c:591   (MemoryRegionOps.write)
  → kayfabe_shim_regs_write()     shim_unsafe.rs:1371              (extern "C")
        mark_vcpu_thread()                        :1384            thread-local
        TrapGuard::enter_at(bar<<56|off)          :1453            thread-local depth
  → Regs::write()                 shim.rs:18576
  → RegPlane::write()             plane.rs:6473
```
Reads are shorter: `nvkvm_trap_read` → `kayfabe_shim_regs_read` → `Regs::read` → `RegPlane::read`.

⊘ BAR0 is **cut into pieces**: 4 KiB pages whose every dword is "dead" get a RAM-backed alias so
the guest's reads of them never exit at all; the rest stay live trap pieces.

### 3.2 A DOORBELL write (BAR0 `0x00BB0090`) — the hot path

Defaults in force: `doorbell_async = On` (defers), `doorbell_inline = Off`.

1. `RegPlane::write` — two atomics (`writes`, `mmio_in_flight` `AcqRel`), then the doorbell arm
   matches and **returns before the rank-0 plane mutex is ever reached** `[CODE plane.rs:6525]`.
2. `ring_doorbell` — bumps a counter; takes **`mmu_inval.inner`** briefly; takes
   **`plane.doorbell` (read)** across the call.
3. `SharedDoorbell::ring` — the passthrough inline short-circuit is off by default; `shadow_route`
   is pure arch math + one `Relaxed` load on an atomic slot array. **No lock.**
4. `pubqueue.offer(MapPublication::for_doorbell(token))` — takes **`pubqueue.inner`**, coalesce
   check, cap check, `VecDeque::push_back`, **drops the guard**, then `notify_one`.
5. Back out: `dbcost` (atomics), **`dbledger.seen`** (a mutex, ≤64 tokens), and the report is
   `Scheduled` — so `announce_completion` is skipped and `state`/`cpu_intr` are **not** taken.
6. The tail of `Regs::write` runs on every write: a few `Relaxed` epoch swaps, each of which may
   emit one further `offer`.

**Net on the vCPU:** ~6 atomics, **4 short lock acquisitions**, one `VecDeque` push, one
`notify_one`. No rank-0 plane lock, no device lock, no IPC, no blocking.

**Deferred to the worker:** everything — mirror revalidation, isolate spawn, ring adoption, the
GPFIFO read, the host forward or CE execution, the retired-object drain.

### 3.3 An MMU_INVALIDATE write (BAR0 `0x00B830B0`) — the one the guest spin-polls

1. `RegPlane::write` → the invalidate arm. Writes to the two PDB latches are pure latching.
2. On the **trigger**: `note_trigger` takes `mmu_inval.inner`, decodes, then
   **`pending.swap(true, AcqRel)`** — *this is the atomic the guest polls* — and bumps `issued`.
3. `shadow_write` pushes the new trigger value into the BAR0 backing page: takes
   **`plane.read_shadow` (read)** then **`ShadowSink::segments`**, then a `write_volatile`.
4. `Regs::write` offers `MapPublication::for_invalidate` onto the same queue.
5. **The vCPU returns. It never blocks.**

**Where the guest blocks: in its own loop.** ogkm spin-polls the trigger until it reads false,
with a **4 s** (graphics) / 30 s (compute) timeout. If the containing page is RAM-backed the poll
costs **no vmexit at all**; if not, each poll is a read trap served by a deliberately lock-free
arm. ⚠ **[UNVERIFIED]** whether page `0xB83000` is backed for the shipped profile — the predicate
is per-dword over the whole page and the survey did not execute it.

**What clears TRIGGER:** the worker, at the end of its pass — `complete_through_unmaps(seq, …)`,
which **withholds** completion if a newer trigger arrived or if unmaps are outstanding
(Constraint 27: completing early would let the guest reuse a page still reachable through a slice
we have not torn down), then `pending.swap(false)` and a `write_volatile` of 0 into the shadow.

⚠ `[CODE shim.rs:18726]` the completion is **no longer a `Drop` guard**. In the code's own words:
*"`complete()` is now the ONLY thing that clears the trigger, and a worker that dies without
calling it hangs the guest by construction."*

### 3.4 Back-pressure — the rule that makes deferral safe

The queue never drops work silently. A refused `offer` converts a **notification** into a
conservative **full sweep**: a dropped doorbell arms an emulated-channel sweep; a dropped
invalidate arms a full rescan; a dropped RPC bind force-releases held replies. Owner's rule,
quoted in-source: *"a lost NOTIFICATION becomes a conservative FULL RESCAN … losing one may only
ever cost time, never coverage."*

★ **Coalescing is OFF for doorbells, and that is a measured verdict.** `[MEASURED]` on real
GA106 the coalescing arm produced `cuCtxCreate → unknown error (999)` and two host `Xid 31`s
(`CE2 FAULT_PDE`, `GR0_PBDMA0 FAULT_PTE`); the `nocoalesce` arm was clean. *"Deferring is fine
and coalescing is not."*

---

## 4. Locks, and the blocking rules

### 4.1 The ranked order

`[CODE lock.rs:56]` *"The total, one-way lock order … Every lock declares its rank at
construction; a thread may acquire only in strictly increasing rank, at most one lock per rank."*

| rank | lock | protects |
|---|---|---|
| 0 | `RegPlane::state` | the GSP FSM, guest-RAM port, command policy |
| 1 | `RegPlane::mem` | the emulated framebuffer store + GMMU format |
| 2 | `SharedDevice::state` (RwLock) | the spine, routing maps, delivery pump |
| 3 | per-`Proc` mutex | one guest process's channels/VA spaces |
| 4 | leaf (`cpu_intr`, executor inbox, recorder) | |

Enforcement is real: `check_acquire` **panics before the OS acquire**, always on, not a
`debug_assert`.

★ Why the plane sorts *below* the device despite being further out: the shipping order is
plane→core and ranks must strictly increase, so the plane must sort first *"or every vCPU MMIO
trap in the device would panic."* The ABBA this fixed was **guest-buildable** — *"A guest could
build the deadlock by ringing a doorbell on one vCPU while touching a register on another."*

### 4.2 The blocking rules — what is actually enforced

| rule | mechanism | teeth |
|---|---|---|
| **R1** no blocking under a ranked lock | `assert_lock_free` | **runtime panic, always on** |
| **R0** no blocking on a vCPU | `assert_not_on_vcpu` | ⚠ **census by default** — panics only under `KAYFABE_VCPU_BLOCK_FATAL` |
| no publishing from a trap | `OffVcpu` token, taken **by value** | ★ **compile-time**; minted in exactly one place |
| INLINE-SAFE(site) predicate | prose only | ⊘ **(a) and (b) have no mechanism anywhere** |

★ The `OffVcpu` doc is the clearest statement of the principle in the tree: *"A constraint that
lives only in prose is re-litigated by every new path, and the reviewer who has to notice it is
the weakest link in the system. **The type is the reviewer.**"*

⚠ **The read path does not mark the thread as a vCPU.** `kayfabe_shim_regs_read` installs only
the `TrapGuard`; a thread that has never done an MMIO *write* is not `is_vcpu()`. A second,
independent detector does catch it — *"the trap witness is measured, a declaration is a claim,
and where they disagree the measurement wins."*

### 4.3 ⊘ The gate that is RED right now

`tests/tests/unranked_locks.rs` enumerates every **unranked** lock a vCPU can hold, because
`assert_lock_free` masks only *ranked* locks and therefore **cannot see these at all**. Its own
header is honest about what it buys: *"ENUMERATED, NOT ENFORCED … Nothing fires when one is taken
in a bad order."*

`[MEASURED 2026-09-20]` it FAILS with **21 undeclared** unranked vCPU-path locks. Two matter
most:

- `dbtable.rs` `Mutex<BTreeMap<u64,[u64;4]>>` — **the doorbell ledger, taken on every doorbell**.
  ⊘ Mine, added w801.
- `shim_unsafe.rs` `Mutex<Vec<ShadowSegment>>` — taken by the **vCPU** in `shadow_write` *and* by
  the **worker** in `publish_invalidate_trigger`. That is a lock genuinely shared between a trap
  and a worker, which is **precisely clause (c)** of the inline-safe predicate.

⚠ And if the BQL contradiction in §0 resolves toward *lockless*, two vCPUs can be in these paths
simultaneously, which makes the shared-lock case sharper still.

**[PROPOSE]** Classify all 21 with real rulings — not to turn the gate green, but because the
answer to *"can anything block beneath this?"* is a fact the v2 design needs anyway.

### 4.4 Where we block on purpose

`[CODE isolate.rs:454]`, stated at the site: *"this is a synchronous write-then-blocking-read
over a unix socket to the isolate child, and **every production caller is inside the vCPU's MMIO
trap** under the QEMU BQL ⇒ whatever the child spends here, **the guest pays**."*

⇒ So the slogan "no blocking work in any MMIO trap" is **not** what the code does. There is one
sanctioned blocking site and it is the isolate round trip. The store-map path explicitly refuses
to be one of these, declining by name on a vCPU rather than stalling.

`[MEASURED w470]` the worst recorded stall was an isolate **spawn** on a vCPU —
`Regs::write → materialize_pending → spawn_host → read_frame → __recv`, **1.62 s** inside an
MMIO exit. That is why the scratchpad is brought up at realize, before the guest runs.

**[PROPOSE]** v2 should make this structural rather than sanctioned: no RM verb on a vCPU at all.
Every such call becomes an `offer` onto the queue, and the guest waits on its own primitive —
which §1 of the architecture says it already tolerates.

---

## 5. Data structures

### 5.1 The object graph — identity vs label

The whole file is built on one distinction, and it is the right one: **a guest handle is a
label, not an identity.** RM recycles handle values with no quarantine and no free list, so the
same 32-bit number names different objects over time.

| type | what it is |
|---|---|
| `NodeKey { client, handle }` | a **handle** — per-client namespace slot. A label. |
| `ResId(u64)` | stable identity, minted at the origin alloc, **never reused** |
| `ResourceKey { origin, incarnation }` | the reportable identity; ordered origin-first so maps iterate deterministically |
| `HandleRef::{Origin,Alias}` | *how* a live handle came to reference its resource — recorded, never re-derived |

```rust
struct Resource {
    node: RmNode, refs: BTreeSet<NodeKey>, pdb: Option<Pdb>,
    pdb_aperture: Option<Aperture>, gpu: Option<GpuId>, map_refs: usize,
    owner_kind: ClientKind, owner: ClientId, owner_incarnation: u32,
}
```
Liveness is `refs` non-empty **or** `map_refs > 0`. `next_incarnation` is *"the smallest ordinal
not held by a live resource at this handle value"*, computed as a **range query**, not a scan —
explicitly to deny a complexity DoS.

⊘ `RmGraph::apply` **clones the whole graph** for rollback, because three faults are raised after
mutation. The comment records it: ~24 % of an O(live objects)-per-event control plane.
**[PROPOSE]** this is a named, deferred cost and v2 should fix it by validating before mutating.

### 5.2 The runtime model

```
Gpu { spine: Spine, system: Proc, procs: BTreeMap<ProcId, Proc> }
```
★ The concurrency contract is in the type, not a comment: a **per-proc** op takes the device
*read* lock plus that proc's mutex; a **spine** op takes the device *write* lock plus exclusive
access to every proc, expressed as a `ProcSet` argument.

**`Proc` is not a hardware concept.** `ProcId` is *"purely the label of one dup-connected
component of declared user clients in the RM graph."* The stable name is `ProcAnchor` — the
smallest client *declaration* in the component.

★★★ The distinction that earned its keep: `clients` (`ClientKey`) **labels** a component;
`client_ids` (`ClientId`) **matches** one. Matching on handle values alone let a re-declared
namespace **adopt the previous tenant's proc** — its isolate, arena, host VAS and pending-release
queue.

`Vas` carries the address plane; `Channel` carries the exec plane. ⊘ `Channel::method_state` is
the one guest-driven state and is bounded **structurally** (a fixed array), not by a check.

### 5.3 The address table — what v2 deletes

```rust
pub struct AddressTable { map: IntervalMap<Row>, owner: Option<Pdb>, generation: u64, blockage }
pub struct Binding { kind: RegionKind, phys: u64, aperture: Aperture, host: Option<HostBacking> }
```
All four `Binding` fields are private with exactly two constructors — `declared_by_guest` (no
host object) and `real_gpu_memory` (the only kind carrying one). Rows are **ranges**, not pages.

`resolve(pdb, va)` checks identity **first** (`owns`), then a predecessor lookup; a miss is a
**fault**, never a heuristic. `bind` checks identity, then host-VA identity, then extent
agreement, *before* anything enters the map.

**[PROPOSE] This whole structure goes.** Under v2 the walk produces a diff, the diff is
`mmap`/`munmap`, and the **host page tables are the model**. Every field above is already in hand
at the moment you act: the target is identity (`store offset == FB offset`), sysmem comes from a
static layout, permissions come from the PTE just walked, and "it went away" is `munmap`.

★ The machinery is **already built and unwired**: `kayfabe_mmu::walkdiff::diff` exists fully
implemented with **zero production callers** (its only caller is a `#[test]`), `MapOp::{Map,
Unmap,Remap}` exists, and `StoreMapPort::apply_ops` takes a **list** and does unmaps-before-maps
— while its one production caller always passes a single-element `Map`. The source even says
*"when the delta lands, this call site does not change."*

⊘ **Consequence today:** the only production unmap is a same-VA re-point. `release_vas` has zero
callers. So `StoreMapPort::outstanding` grows **monotonically over the VM's life** — mappings
only ever accumulate, which is the other half of the cross-client leak.

### 5.4 Crate layering

`util → arch → abi → {mmu, core, fwd, rmrpc, device, rt} → qemu-raw`, with `vmm` and `isolate`
as ports.

★ One layering fact worth arguing about: **`kayfabe-mmu` depends on `kayfabe-isolate`**, because
`Binding` carries a `HostHandle`. So the address plane sits *above* the sandbox port rather than
beside it. **[PROPOSE]** deleting the address table inverts that and makes `kayfabe-mmu` a leaf
crate — which is a good structural signal that the deletion is the right shape.

### 5.5 The isolate boundary — two layers, not one

⚠ A correction to a premise I held: `VerbPlan`/`VerbReply` are the **in-process** typed plan and
reply. The **IPC** is `Request`/`Reply`/`Envelope`.

- **Transport:** one `SOCK_STREAM` socketpair **per pool worker**, granted at fixed fd numbers.
- **Framing:** `u32` LE length then body, in **one** `write_all` — because a length written
  separately from its body permanently desynchronises a signal-interrupted channel.
- **No multiplexing, no demux, no txn correlation.** Each worker owns its socket and the channel
  is 1-deep. An explicit divergence from the C, which multiplexed 16 workers over one fd.
- ⊘ Two `RmError` variants deliberately have **no wire form**: `ForeignHandle` (our own gate
  produces it before a request is written — a compromised child must not be able to claim our
  invariant broke) and `Wedged` (a child that could send it would be reporting that it never
  answered).
- **Cancel** travels out-of-band on a separate datagram socket, guarded by a txn the worker
  publishes immediately before the verb and clears immediately after — without it a cancel races
  the completion and lands on an unrelated later operation.

---

## 6. Installing a mapping, and observing a completion

### 6.1 The mapping chain, end to end

The decision is made by a **publication pass**, not by a fault — we map ahead, we do not wait to
be asked.

```
doorbell_publish_loop                     (worker thread; mints the one OffVcpu)
 → publish_vas_rows(token, seen, OffVcpu) shim.rs:13259   — takes OffVcpu BY VALUE
 → vas_publish_census(...)                                — produces candidate (va, len, phys)
 → join_one_fb_leaf(...)                  shim.rs:15099
 → plane.fb_join_plan(phys, len) → FbJoinPlan::DeviceBacked { at: phys }
 → map_store_slice_for_leaf(...)          shim.rs:14765
      step 0  decline if on a vCPU or in a trap          :14787
      step 1  vaspace_handover → scratchpad dups the space (once per Vas)
      step 2  apply_ops(store_vas, [MapOp::Map{ va, gpga: at, len }])
 → StoreMapPort::map                      storemap.rs:559
 → iso.with_worker(|w| w.with_rm(|rm| rm.map_store_slice(...)))
 → ProxyRmBackend::map_store_slice → write_frame / blocking read_frame   (process boundary)
 → child: worker_loop → serve_one → HostRmBackend::map_store_slice
 → RmConnection::raw_map_dma_slice        rm.rs:4425
 → ioctl(NV_ESC_RM_MAP_MEMORY_DMA)        rm.rs:4509       ← THE SYSCALL
```

★ **The syscall happens in the scratchpad isolate child.** The VMM performs no RM ioctl for a
store slice; it writes and reads one socket frame.

Two details that are load-bearing and easy to lose:
- ⊘ The code takes **RM's answer, not its own request** (`done.placed.first()`). The comment
  records that substituting the requested address here regressed the guest to `NEVER RETIRED`.
- ⊘ **Constraint 28:** if RM places the mapping anywhere other than where we asked, the mapping
  is **torn down** and the call refuses — a fixed map that silently moved is worse than none.
- ⊘ `status == 0x51` on a FIXED map means *already mapped there* (occupancy, not capacity) and is
  re-reported as such.

**Guest RAM** takes a different route: the memfd arrives **at spawn on a fixed fd**, not on a
request, so a future seccomp filter can hardcode it. The child `mmap`s it and wraps it in an
`OS_DESCRIPTOR`; ⊘ RM pins those pages and releasing the `mmap` does **not** unpin them.

### 6.2 The single store

`[CODE]` Allocated at **PCI realize**, in the scratchpad, by a **bisecting probe**: halve to a
floor, then bisect to 64 MiB grain, each attempt allocated and freed. First try contiguous +
1 GiB aligned; fall back to non-contiguous + 4 KiB, latching which — and that latch is what
later picks the page-size flag for every mapping.

`[MEASURED]` **11904 MiB**. ⚠ Not a constant anywhere: the compiled advertisement is 12288 MiB
and is **rebound to whatever the probe reserved**. ★ The identity `guest FB offset == store
offset` is only safe *because* of that rebinding.

⊘ Deliberately **not** `alloc_vidmem`: that verb is contiguous-with-`alignment = len`, so an
11.8 GiB request refuses on **fragmentation** — a refusal that is not about capacity, on the one
call whose refusal is supposed to mean "there is not enough video memory."

### 6.3 Completions — two mechanisms with opposite writers

| path | who writes the value the guest polls | how we learn |
|---|---|---|
| **GR / compute**, forwarded | **the GPU** | we only *read*: `epoll_wait` at a 250 ms floor, woken early by an eventfd; the watch list is declared on the vCPU and swept on the observer thread |
| **emulated CE**, served locally | **we do** | our executor writes the guest's own literal payload, at the address the guest's own page tables resolve to, **after** the bytes moved |

★ The completion-watch module **never writes, never raises, never resolves** — the capability it
is handed is a reader, by construction. ★ The CE writer is a **single-writer discipline** with a
compile-fail test and a workspace-wide census, because the C had a second, lagging writer that
corrupted a live payload.

⊘ `AWAKEN_ENABLE = 0` in the measured `cuCtxCreate` stream: the guest asked for **no interrupt at
all**. The completion *is* a value appearing at an address. That is why polling — not interrupt
delivery — is the mechanism here.

### 6.4 Interrupts — what gets announced

Two independent gates, both in `ring_doorbell`:
- **non-stall vector:** only a **locally served** doorbell announces. A forwarded one does not —
  the work finished on a host engine at an instant this device was not standing at.
- **OS-event wake:** fires for **served or served-locally**. `[MEASURED w686]` gating this on
  locally-served alone meant **40 of 44** completions on passthrough channels announced nothing
  and the guest sat in `MC_SERVICE_INTERRUPTS`.

⊘ `LEAF_TRIGGER` is the **guest's own** self-trigger register, not our announcement path.
Masking is **recorded, not acted on**.

---

## 7. The proposal, and the arguments against it

This is the part to attack. Each item has the counter-argument I can see.

### P1. Mirror, not adopt

There are two ways to make the GPU honour the guest's translations.

- **Adopt** — point the host channel's instance block at the **guest's own** page-directory root.
  No walk, no diff, no syscalls at all. Most of v2 evaporates.
- **Mirror** — walk the guest's tables, replicate into a host VA space.

**I propose mirror, and I want this one argued.** My objection to adopt: the guest's PTEs contain
**guest** physical addresses, which are host-valid only where the backing is identity — true for
the store, false for sysmem GPAs. ⚠ If adopt can be rescued (e.g. by making sysmem identity too),
it is strictly better and I would drop mirror.

### P2. No address table; the host page tables are the model

Already argued in §5.3. The strongest evidence it is right: **the diff machinery already exists
and is unwired**, and the reconciliation bugs that dominated 2026-09-19/20 are all of the form
*"two copies of one fact disagreed."*

⊘ **Argument against:** syscall volume. N changed pages ⇒ N mappings unless runs are coalesced.
**Unmeasured.** The walk is 205.7 µs and page tables change far less often than doorbells ring,
so I expect small bursty diffs — but that is a prediction.

### P3. Lazy everywhere; strict only at recycle — and recycle is the guest's job

A doorbell **never** waits for a refresh. ogkm already accepts that addresses are *undefined*
until an invalidate completes: old page or new page, never a page neither mapping referenced.

⇒ We own no scrub policy. Our single failure is that we **forge the scrub's completion**
(`forwarded=0` on the kernel tokens), so the guest reuses a page it believes was cleared. **That
is the cross-client leak.** Fix: let the work reach the GPU. The gate already exists.

### P4. No RM verb on a vCPU, structurally

§4.4 shows the current design **sanctions** one blocking site and pays for it with a measured
1.62 s stall. **[PROPOSE]** make it impossible instead: every RM verb becomes an `offer`, and the
guest waits on its own primitive.

⊘ **Argument against:** some verbs are genuinely synchronous in the guest's eyes — a control
command whose reply the guest reads immediately. Those need an answer *now*. ⚠ I do not have a
clean answer for that case and it is the main hole in P4.

### P5. Deletion order

1. **The scrub** — live security bug, gate exists, smallest change.
2. **`Pdb`-as-identity** — still costing diagnoses; split into `VasId` + `Option<Pdb>`.
3. **The address table and joins together** — they are one mechanism.
4. **The CPU executor last** — the scrub result tells us which half of that plane survives.

### P6. What must NOT be deleted

The instruments. The doorbell ledger's per-token `forwarded=`, the censuses, refusals **by
name**, the bare-metal baseline, the hardware gate. `[MEASURED 2026-09-20]` the bare-metal
baseline is what turned nine guest failures from opinion into adjudication; without it, a rewrite
cannot be graded, and a rewrite you cannot grade is a rewrite you cannot land.

---

## 8. Open problems — named, not designed away

1. **Where the walk's root comes from.** `MAP_MEMORY_DMA` is a HAL stub on GSP parts, so a
   client-allocated VA space is never declared — `[MEASURED]` **zero** `SET_PAGE_DIRECTORY`
   events in a whole boot. Candidates, ranked: the **instance block** (which we author, and which
   is how hardware itself finds the root); `COPY_SERVER_RESERVED_PDES` (arrives, but only for
   RM-managed spaces); observing the guest's own page-table writes.
2. **Fault delivery becomes load-bearing.** v2 lets the GPU fault instead of pre-validating
   operands — but guest fault-recovery methods are currently **refused by name** precisely
   because we do not deliver faults. That refusal is the marker for this work.
3. **The BQL question** (§0). It changes the concurrency argument wholesale.
4. **The 21 unranked vCPU-path locks** (§4.3).
5. **Multi-GPU and non-GA10x.** The chip table holds exactly one profile.
6. **Syscall cost of the diff** (P2).

---
title: "kayfabe — architecture review"
subtitle: |
  The whole planned system, what is actually built, and where it is weakest.
  Written for a reader who does not know this project, for the purpose of
  hunting flaws in it.
date: "2026-07-25 · repo `kayfabe` · branch `master` · HEAD `3569d46`"
geometry: margin=1.9cm
fontsize: 10pt
mainfont: "DejaVu Sans"
monofont: "DejaVu Sans Mono"
colorlinks: true
header-includes:
  - \usepackage{pdflscape}
  - \usepackage{graphicx}
  - \usepackage{needspace}
  - \hyphenpenalty=800
---

# 0. How to read this document, and what it is for

This is a review document, not a status report and not a pitch. Its job is to describe the
system accurately enough that someone who has never seen it can find its mistakes. Four
consequences follow, and they shape everything below.

**Numbers trace to something.** Every count, score and measurement in this document comes
from a file in the tree or from a commit message, and the source is named. Where a number
is an estimate, it says so. Where a claim could not be verified, it is omitted rather than
softened.

**Unbuilt is stated as unbuilt.** Roughly three quarters of the planned system does not
exist. That is not a failure — the build order is deliberate — but a review document that
blurs the line between "designed" and "running" is worthless. Section 2 is what exists;
section 3 is where it is weak; sections on L2 and L3 say "not built" without decoration.

**The uncomfortable parts get the most space.** Section 3 is the longest section and is
the one to read if you only read one. It is assembled from the project's own contact log —
a running record, kept in `docs/design/l1_concurrency.md` §12, of every place where writing
the code proved the design wrong — plus this review's own reading of the source.

**Sections 3.2 and 7 are the findings.** Reviewing the code against the documents turned up
eleven places where they disagree — two of them substantive: an atomicity claim the code does
not deliver, and a stated resolution to a lock-discipline problem with no mechanism behind
it. Separately, a teardown audit that completed during writing found nine reclamation gaps
and six more optimistic-documentation instances. Those two sections are worth more than the
rest of this document, and the pattern they share — documentation drifting optimistic as code
moves under it — is worth more than either.

A note on constraint: this review was written without compiling or running anything. Every
statement about behaviour is read from source, from the design documents, or from measured
numbers recorded in commit messages and gate documents. Where a claim would need a test run
to confirm, it is marked.

> **★ This document describes the tree as of commit `3569d46`, and says so deliberately.**
> A teardown-and-reclamation audit of the core completed while this was being written, and
> fixes for four of its findings (G1, G3, G3b, G4 in section 3.2) were **in flight in
> parallel with this section being written** — reshaping the address binding, the reap's
> signature, the host-verb result type and the fault vocabulary. Section 3.2 records those
> findings as found-and-being-fixed, because that is what they were at the pinned commit.
> Line numbers and type signatures cited anywhere in this document are pinned to `3569d46`;
> where one has moved by the time you read this, the pin is what makes the citation honest
> rather than wrong. A separate design for the next L1 milestone's OS shell is also in
> progress and is referenced by name only.

---

# 1. What kayfabe is

## 1.1 The problem

Giving a virtual machine access to a real GPU normally means one of two things. Either you
hand the whole physical device to one guest through IOMMU passthrough — simple, and it
gives that one guest everything and every other guest nothing — or you buy hardware and
licences that support vendor virtualization (SR-IOV, vGPU), which exists on datacenter
parts and not on the consumer cards most people actually own.

kayfabe is a third option: **share one commodity GPU among several untrusted virtual
machines, with per-process blast-radius containment, using only unprivileged host
processes.** The multi-tenant claim is the point. A single cross-tenant leak, or a hostile
guest that can wedge the box, would be fatal to the thesis.

## 1.2 The trick

Some vocabulary first, since the rest of the document leans on it.

**RM** is NVIDIA's *Resource Manager* — the object model inside the driver. Everything a
GPU program touches is an RM object with a handle: clients, devices, address spaces,
channels, memory, engine contexts. Guest software allocates and frees these through ioctls
and RPCs.

**GSP** is the *GPU System Processor*: on modern NVIDIA hardware, a real CPU core on the
GPU die that runs a firmware copy of much of the resource manager. The host driver no
longer programs most of the hardware directly; it sends RPC messages to the GSP over a
ring buffer in memory.

**VAS** is a GPU virtual address space. **PDB** is its *page directory base* — the physical
address of the root of that address space's page tables, and, for our purposes, its
identity: the GPU's memory management unit keys page tables by PDB, which makes the PDB the
real memory boundary. **vChid** is a *virtual channel id*: channels are the GPU's work
submission queues, and the vChid is how a submission is routed to one. A **doorbell** is
the register write that says "channel N has work waiting"; the value written is a token
that encodes the vChid.

The trick is this. **The guest is not modified at all.** It runs the stock NVIDIA kernel
driver against an emulated PCI device that presents a plausible GPU and a *faked GSP*: we
implement the RPC ring the driver talks to, and answer as the firmware would. The guest
driver believes it owns hardware. It allocates RM objects, builds GPU page tables, creates
channels, and rings doorbells.

We are on the other side of that conversation. From the protocol traffic we **reconstruct
what the guest program was actually trying to do**, and we do the equivalent thing on a real
host GPU — through ordinary, unprivileged userspace operations, inside a sandbox.

This is called *Mode 2* in the project's vocabulary. Mode 1, the older approach, forwards
the guest's *userspace* ioctls directly and therefore needs a modified guest. Mode 2 needs
nothing from the guest, which is the whole reason it is worth the difficulty. kayfabe is
Mode-2 only.

## 1.3 The correctness criterion, and why it is not "replay everything"

By the time traffic reaches us, the guest kernel has already decomposed a userspace
intention — `cuCtxCreate`, a memory allocation, a kernel launch — into a stream of
low-level operations, many of them *privileged* steps that only exist inside the
kernel-driver-to-GSP path. Replaying those on the host is impossible: an unprivileged
process issuing one gets a permissions error, and that error means "you forwarded at the
wrong layer", not "you need more privilege".

So the correctness criterion is deliberately weak in the middle and strict at the edges:

> Only the **observable end-states** must match a real system — the host GPU's actual
> execution, and what the guest's GPU application sees. Everything in between is free.

That licence is what makes the project tractable. It is entirely correct for some guest
kernel operations to complete instantly as fakes, provided the *one* operation that carries
the real work triggers the real host-side chain. Faking an internal side effect is not the
same as faking a result.

This produces the two translation rules that recur throughout the codebase:

- **Case 1** — the RPC essentially *is* a userspace operation (allocate a channel, map
  memory, allocate an engine object). Re-issue it, roughly one-to-one, on the host through
  this guest process's sandbox. The host's own kernel driver then does all the privileged
  work internally — including, for graphics and compute contexts, building the golden
  context image on real silicon, which is something no software can fabricate.
- **Case 2** — a GSP-internal control with no userspace equivalent (context promotion,
  context buffer info). **Acknowledge the guest and do nothing on the host.** The effect
  was already achieved by the Case-1 forward.

## 1.4 The north star

The project's stated first milestone of real value is unchanged:

> `cuCtxCreate` → first compute → a matrix multiply, **numerically correct**, run by an
> unmodified guest against a real host GPU.

Numerically correct is the operative phrase. The project's own standing rule is that a
green log in the guest proves nothing; a result is only real if it can be shown to have
come from hardware.

## 1.5 The two other things that make the design work

**Trap minimization.** The only traffic that *must* trap out to us is the doorbell write,
the GSP RPC ring, and occasional control-plane RM operations. Userspace pushbuffers — the
command streams the GPU reads — and completion semaphores are ordinary shared pages: the
host GPU writes a semaphore straight into a page the guest polls, and the guest's own
processor sees it, with no thread of ours involved and no lock taken. This is why the
steady-state hot path costs approximately nothing, and it is the reason the fairly elaborate
locking described in section 2.8 is affordable at all. It is also a load-bearing bet
(section 3.10).

**Isolation by construction.** Each guest process gets its own sandboxed, unprivileged host
process — an *isolate* — per GPU it uses, with its own RM client, its own handle namespace,
its own guest-physical memory arena, and its own host address spaces. Two guest processes
that use byte-identical guest addresses and byte-identical RM handles (which the stock
driver does routinely) reach disjoint everything, because there is no shared structure for
them to collide in. This was learned expensively: the predecessor's worst multi-process bugs
were all collisions in shared state.

## 1.6 The relationship to the C predecessor

There is an earlier research artifact, written in C, in a separate repository. It proved
the approach works: it booted a fake GSP, brought up a compute context, ran a 7-billion
parameter language model at host parity on bare metal, and survived multi-process workloads
badly enough to produce a detailed catalogue of failure modes.

kayfabe is not a port of it. It is a clean-slate rewrite that implements the *settled*
design and keeps the C artifact alive as two things: a differential oracle (a second
implementation to disagree with), and the source of a regression matrix — twenty-five
catalogued C bugs, each classified against the new core as impossible-by-construction,
tested, or an honest gap (section 5.7).

## 1.7 The layer model, and the hexagonal shape

\begin{landscape}
\thispagestyle{empty}
\begin{center}
\includegraphics[width=\linewidth,height=\textheight,keepaspectratio]{l1_diagram_system.png}
\end{center}
\end{landscape}

The system is built in four layers, bottom-up, and only the bottom one and a piece of the
second exist.

**L0 — the pure logic core. Complete.** A deterministic state machine over abstract facts.
It contains no operating system calls, no syscalls, no wall clock, no NVIDIA structure
layouts, and no GPU-generation or driver-version name anywhere. The last of those is
enforced by a grep in continuous integration.

**L1 — the Linux OS layer. Milestone 1 built.** The threading, locking and sandbox-driving
shell around the core. What exists is described in section 2.8; what does not is described
in section 3.11.

**L2 — the QEMU/VMM adapter. Not built.** The register file, MMIO trap dispatch, the GSP
boot state machine and its RPC transport, memory slot management, interrupt injection. The
core does not yet implement the `Device` port at all; its adapter-facing surface today is an
event-level API, not registers.

**L3 — the per-architecture ABI and real applications. Not built.** One real GPU
generation's class identifiers, doorbell token encoding, page table formats, work-submission
region layout and command encodings; the generated wire layouts for a specific driver
version; the page-table walker; and real CUDA and Vulkan applications on real hardware.

### Why the core is OS-free

The core is a hexagon: one pure logic region, and every effect crossing a **port** — a Rust
trait — implemented by an **adapter** that lives outside. The ports are:

| Port | What it abstracts | Real adapter lands at |
|---|---|---|
| `Vmm`, `Device`, `Present` | the hypervisor and the display | L1 / L2 |
| `RmBackend`, `Isolate`, `IsolateFactory` | the sandboxed host driver connection | L1 |
| `Arch`, `GmmuFmt`, `UserdModel`, `PushbufferAbi` | one GPU generation's behaviour | L3 |
| `DriverAbi` | one driver version's wire layouts | L3 |
| `TraceSink` | structured tracing | any |

**Today, the only implementation of every one of these is a deterministic mock.** That is
not a placeholder situation; it is the design working as intended, and it buys three things
that would be very hard to recover later.

First, **the entire test suite runs with no GPU, no hypervisor, and no syscalls.** Two
hundred and three tests, in about eighteen seconds, on any machine. The project's
predecessor could only be tested on a rented GPU host, strictly serially, with a full VM
reboot between runs, because a concurrent or interrupted run would wedge the hardware. That
constraint shaped — and slowed — everything about how it was debugged.

Second, **time is a value, not a clock.** The mock hypervisor's clock advances only when a
test says so, and deferred events fire in deadline order. Nothing in the logic crates can
read wall time; there is no wall clock in the dependency graph to read.

Third, and most important for correctness: **the core is order-independent**, which makes
single-threaded testing of multi-threaded interleavings *sound* rather than merely
convenient. Section 5.2 develops this.

The seam is not a matter of style. The standing rule is that adding a GPU generation must
be an `impl Arch for <generation>` in an adapter crate with **zero edits to any logic
crate**. The mock architecture — deliberately given non-NVIDIA encodings, so that any code
secretly assuming a real bit layout fails immediately — is the standing proof that the seam
holds.

---

# 2. What is actually built

The whole of L0 and the first milestone of L1. In numbers: about 12,300 lines across thirteen
crates — of which 1,465 are the test-only mocks — plus about 14,400 lines of integration
test, 203 tests, and two standing mutation scores (99.2% on the pure core, 92.44% on the L1
surface).

> **Read this section against section 3.2.** What follows describes the *derivation* side of
> the core — building state from protocol facts — which is the part that has been heavily
> verified. The *reclamation* side, what happens to host objects when things go away, has
> nine known gaps as of this commit, four of them being fixed while this was written. Section
> 2 would, read alone, imply a completeness that teardown does not yet have.

## 2.1 The RmGraph: protocol, not trace

\begin{landscape}
\thispagestyle{empty}
\begin{center}
\includegraphics[width=\linewidth,height=\textheight,keepaspectratio]{l1_diagram_dataflow.png}
\end{center}
\end{landscape}

Everything derived by the core comes from one structure: a refcounted graph of RM objects,
built from **declared facts**. There are exactly six kinds of fact — allocate, duplicate,
set page directory, map memory, unmap, free — and they are *abstract*. They carry no wire
bytes and no NVIDIA structure layout; decoding wire format into facts is L3's job and does
not exist yet.

Three properties matter.

**Protocol, not trace.** Derived state is a pure function of the facts the guest declared,
never of the order in which they arrived, and never of timing. If a fact names a handle
that does not exist yet, it is *parked* rather than dropped, and resolves when its target
lands. This is where order tolerance comes from, and it is the property that most of the
testing strategy rests on.

**A resource/handle split.** A resource's identity is a private monotonically increasing id
minted at its origin allocation — deliberately *not* derived from the handle value, so a
freed and re-allocated handle is a genuinely new resource rather than a resurrection. Each
live handle, including every surviving duplicate alias, is a reference. Mappings are a
second class of reference. A resource dies when it has neither. That is what makes
`DUP_OBJECT` aliasing — one client handing another client a reference to an object, which
is how the unified-memory driver joins itself to a compute process — safe: the alias keeps
the resource alive past the origin handle's free.

**Every guest-growable table is bounded.** Live handles, live mappings and each parked table
cap at 2^18 entries; the outstanding-completion queue at 2^18; armed fences at 2^16. Hitting
a cap is a loud, named refusal, never an out-of-memory condition and never a panic. The
parked-map table is a *set* rather than a list specifically so that a flood is O(n log n)
and there is no quadratic-scan denial of service.

## 2.2 Projections: where "a process" comes from

The graph is projected — purely, deterministically — into two things.

**Process boundaries.** A "process" in this system is not a client handle and not a
timing observation. It is a *dup-connected component* of the client graph: clients that have
handed each other object references are one process, because they share a blast radius. The
representative label is the minimum client handle in the component, chosen by a union-find
that unions by minimum so that the representative *is* the label. This matters because the
NVIDIA driver's client handles are not process keys — values are reused across processes and
one process holds several.

**Routing.** Two maps: `(GpuId, Pdb) → address space` and `(GpuId, VChid) → channel`. Note
the key. Page directory bases and channel ids are *per-GPU* namespaces, so identical values
on two different GPUs are entirely legal and must not collide; identical values on *one* GPU
are a hardware impossibility and are a loud, contained refusal. The multi-GPU axis was
retrofitted late, and the consolidation review's most valuable finding was a non-finding:
the retrofit did not fork any concept — `(GpuId, ·)` keying is applied uniformly across
projection, runtime, routing, faults, sandboxes, arenas and delivery, with the single-GPU
case being GPU zero rather than a parallel code path.

## 2.3 The address table: forward-populated, and a miss is a fault

This is the design directive the project treats as most load-bearing, inherited verbatim
from the predecessor's hardest debugging campaign.

There is **one authoritative table** per address space that resolves a guest GPU virtual
address to a physical range. It is:

- **forward-populated only** — bound when the guest declares the mapping, read at lookup,
  never traced backwards from an address at execution time;
- **never resolved by walking the guest's page tables at lookup**;
- **free of fallbacks.** A lookup either hits the table or it is a **fault**.

The mental model is that *the table plays the role of the GPU's translation lookaside
buffer.* The guest's contract with real hardware is: write page table entries to memory,
issue an invalidate, and only then rely on the mapping. A real TLB holds stale or absent
entries until an invalidate. The guest cannot distinguish "real TLB, flushed on invalidate"
from "our table, refreshed on invalidate" — so our table's staleness between invalidates is
not a bug, it is architecturally identical to the hardware, and the guest's own invalidate
discipline is exactly what keeps us correct. We add no new requirement on the guest.

What this rules out is an entire class of bug the predecessor paid weeks for. It had a
resolution *cascade*: try the instance block, then a snooped array of address spaces, then
scan the framebuffer window, then guess. Each fallback was individually reasonable and
collectively catastrophic — one guest's address resolved under another guest's page
directory, a scrubber's semaphore release landed on the unified-memory driver's persistent
tracking page and drove it backwards into a fatal jump, a most-recently-used ring cache
evicted the page-table writer's ring under two-process load. In this core, **none of those
code shapes exists to be wrong**. There is no cascade, no most-recently-used cache, no
content scan, and no reverse resolver. A miss is a named fault with the page directory and
address in it.

`MISS = FAULT` is applied beyond addresses. An unknown or stale completion source is a
loud fault, never a guess and never a fallback onto process zero. A doorbell token that does
not decode is a refusal. Staleness after a dropped lock is a refusal (section 2.8).

## 2.4 The execution plane

The core does **not** emulate the graphics or compute engine, and the design document is
unusually blunt about it: a method emulator is "the throwaway, non-performant thing; never
build it". The golden context that a graphics context needs is produced by microcode on real
silicon and cannot be fabricated. So the execution plane is an *orchestration* layer with
four jobs: recognise context lifecycle and forward the Case-1 allocations so the host builds
its own context; decode *just enough* pushbuffer to forward the work and extract the facts
the address and completion planes need; carry an engine tag so work reaches the right
engine; and route completions.

Two structural properties are worth calling out.

**There is exactly one path that may ring a host doorbell**, and it gates first: every
address in the submission's working set must resolve *and* have been published into this
address space's own host address space. An unpublished address is a loud fault before any
host operation happens. No ungated sibling function exists — the predecessor's ungated one
was deleted, not deprecated.

That gate is the fix for the predecessor's hardest bug, and the reason it is load-bearing is
worth stating: when two processes used identical guest addresses, the loser's addresses were
never published into its *own* host context's address space. The host GPU then faulted past
the shared prefix, with a real hardware error code. The completion the guest was waiting for
legitimately never existed, because the work never ran. That is a bug that looks like a
delivery problem and is actually an execution problem — and a delivery-side fix would have
been a plausible, wrong, and expensive detour.

**Scheduling state is per-process.** The predecessor had a device-global one-shot flag; the
second context to arrive found it already set, skipped its own scheduling, and ran off the
runlist. Here the scheduled set lives inside each process, keyed by that process's channel
ids, and there is no scalar for a second context to trip over.

## 2.5 The completion plane

Per guest process: a completion queue and a set of armed mapped fences. Delivery composes
across processes into a per-GPU batch, gated so that one batch is outstanding per GPU's
message queue at a time.

The gate is a **transport** limit, never an **observation** gate — and the distinction is
the fix for another catalogued failure. The predecessor drove delivery only from the
doorbell handler, behind a global "did anything complete" check, into a single cross-process
batch. A process that polled but did not submit starved as soon as its neighbour went quiet.
Here, re-delivery is driven off **the owner's own poll**, so a process that only polls
re-posts its own unacknowledged events regardless of what anyone else is doing.

The mapped-fence arm exists for video encode, where completion is a coherent fence the GPU
writes and a worker reads with no syscall. Its observation is wrap-correct and carries a
backwards-jump guard: a step larger than 2048 is refused with the arm unchanged. That number
is not decorative — the predecessor's fatal unified-memory failure was a backwards jump
caused by a misrouted semaphore write.

## 2.6 Per-process isolation and multi-GPU

Every guest process owns four planes and nothing is shared:

- **address** — one table per address space, one address space per `(GpuId, Pdb)`;
- **execution** — channels keyed by vChid, plus the per-process scheduling record;
- **completion** — the queue and the fence arms;
- **sandbox and arena** — one isolate and one disjoint guest-physical arena per
  `(process, GPU)`.

Arena release takes the arena **by value**, which means releasing an arena a live process
still owns does not type-check. That is not a stylistic flourish: the predecessor leaked
guest-physical window space across process churn until it exhausted, and the same leak was
silently reintroduced in this rewrite and caught by a regression test written for the C bug
(section 5.7).

## 2.7 The completion-source reactor: a pure port

The newest piece of the core, and the clearest example of where the hexagonal boundary is
drawn.

Something has to watch for completions arriving from the host — file descriptors the sandbox
armed, sandbox worker death, a wake primitive, future cross-sandbox signalling. The naive
placement puts all of that in the OS layer. The design instead splits it:

- **The core owns the model, and the model is pure.** Opaque completion-source handles,
  minted monotonically and **never reused** — so a handle that outlives its registration is
  permanently unresolvable rather than re-bindable, which makes an entire use-after-free
  class unrepresentable rather than guarded. Four source kinds, from day one, so that adding
  the fifth is a list entry and not a redesign. A registry, and the dispatch logic: *"source
  S signalled → which process → what to do"*. And an abstract notifiable source — the core
  can *request* a wake and does not know what a wake is.
- **L1 owns the adapter**: the table mapping a source to a host descriptor, the real wait
  loop, the real wake primitive.

The vocabulary rule is absolute and mechanically enforced: the words `eventfd`, `epoll`,
`timerfd`, `rawfd`, `libc` and `O_NONBLOCK` may not appear anywhere in the eleven pure
crates, **in code or in comments**, and a CI job greps for them and fails the build on a hit.
The gate was verified in both polarities — a clean tree passes, a planted comment fails.

The abstraction earns its keep for a reason beyond tidiness: it is what makes completion
interleavings **deterministically testable**. A test drives dispatch directly with zero
syscalls, holds sources pending, and fires them in adversarial orders with no timing
dependence at all. Without the port, testing the completion flow would mean real
descriptors, real readiness and real races — the thing the strategy exists to avoid.

## 2.8 The L1 shell as built

\begin{landscape}
\thispagestyle{empty}
\begin{center}
\includegraphics[width=\linewidth,height=\textheight,keepaspectratio]{l1_diagram_runtime.png}
\end{center}
\end{landscape}

The L1 milestone that landed in this campaign is the threading and locking shell. It is
about 1,780 lines. Four ideas.

### Ranked locks with always-on assertions

Three lock ranks: **device = 0**, **process = 1**, **leaf = 2** (the executor inbox and the
verb recorder). Wrapper types maintain a per-thread *bitmask* of held ranks — a bitmask, not
a counter, because a second acquisition at a rank already held is a violation, not recursion.

Two invariants have teeth:

- **R3 — acquire only in strictly increasing rank, at most one lock per rank.** Checked
  *before* the underlying acquisition, so an inverted order panics deterministically instead
  of sometimes deadlocking first. The check is a single shift-and-test, which enforces both
  halves of the rule at once.
- **R1 — no blocking call under any lock, ever.** Asserted at the host-verb entry point
  itself.

Both are **always-on panics, not debug assertions** — verified in the source. The argument
is recorded in the contact log and is worth quoting in substance: a thread-local read costs
far less than the lock acquisition it guards, and the entire reason these invariants exist is
that their violation is *invisible until the unlucky interleaving*. Compiling the detector
out of the build that actually runs in production inverts the point of having it.

The rank state is restored on unwind, so a panicking test does not leave a phantom bit
behind — which is itself pinned by a test.

### `SharedDevice`, and both lock modes from day one

The device state is a ranked reader-writer lock over `{ the pure spine, the system process
in its own cell, a map of per-process cells }`. Device-global operations take the write lock;
per-process operations take the device read lock plus that one process's lock.

Two mechanics are worth understanding.

**Device-wide operations acquire zero process locks.** The naive reading of "exclusive access
to every process" would lock each process cell in turn — which would hold N rank-1 locks at
once and violate R3 on the first two-process device. It does not have to: under the write
guard the caller already holds a mutable reference to the whole state, and a mutex hands out
its contents mutably *without acquiring anything*, because a mutable reference already proves
the exclusivity the lock would establish. A test asserts that the thread's rank-1 acquisition
counter stays flat across every device-wide operation — and notably, injecting a spurious
lock into that path does **not** trip R3, because rank 0 to rank 1 is a legal increasing
order. That test is the only thing standing between this design and a silently reintroduced
convoy.

**Both lock modes ship, and are tested, from day one.** A `Degenerate` mode where every
operation takes the write lock, and a `Sharded` mode with real per-process locking. The
earlier review flagged the classic trap — a granularity flip scheduled for "later" is exactly
where retrofit races appear, and the sharded mode is then the untested one at the moment it
matters. So both modes run in every threaded tier, and a differential test asserts a
bit-identical end state and an operation-by-operation identical log across them. The two
modes are genuinely different, not the same code with a bigger lock: in degenerate mode rank
1 is *never entered at all*, and even read-only probes take the write lock.

### The plan / execute / commit verb seam

This is the shape R1 forces, and it is the most consequential piece of the milestone.

A guest operation that needs the host runs in three phases: **plan** under the device read
lock and the owning process's lock, emitting a typed description of the verbs to run —
identifiers only, never a held reference — and calling nothing; **execute** with **no lock
held**, doing the blocking round trip on the calling thread against a worker it checked out;
**commit** under the same locks re-acquired, re-resolving every identifier through the graph
and re-validating before applying the reply field by field.

The enforcement went further than the design asked for. The method that handed out a
reference to the host connection **no longer exists**. A backend lives in a pool slot, and
checkout *moves* it out to the calling thread. A locked phase therefore has nothing to call:
the old violating shape does not merely panic at runtime, it does not type-check. The runtime
assertion covers only what types cannot.

**R5 — re-validate after re-lock** — is the third always-on invariant, and it is subtler than
it first looks. Section 3.4 is about why.

### The worker pool

Each sandbox holds a bounded pool of workers (four by default), each strictly one verb in
flight on its own one-deep request-reply channel. Concurrency comes from **channel count,
never from multiplexing a channel** — so the predecessor's shared in-flight slot table and
transaction demultiplexer, itself a proven bug source, has no home to return to. A
checked-out worker is owned mutably by exactly one thread, so single-in-flight is still the
borrow checker's guarantee, just N times over.

Pool exhaustion is backpressure, not a hang: the caller releases *every* lock, parks on a
condition variable, and re-enters from the very top — which means a full re-plan and a full
re-validation, because the world may have changed while it waited.

### The condemned component

The last thing the milestone added, and it was found by a test rather than by review, so it
belongs in section 3 as much as here.

When a sandbox worker dies out of band, the design says: retire the process loudly, its
completions die with it, **never a respawn**. The implementation retired the process — and
then the very next protocol event from any client re-derived that component, matched no live
process, took the "new process" branch, and minted a fresh identity with a **freshly spawned
sandbox**, a new handle namespace and a new arena. In other words, *a guest able to crash its
own isolate worker got a clean new isolate on demand.*

The fix carries a set of **condemned components** in the device spine, and the choice of
identity key is the whole design. Three candidates were on the table and two are wrong. The
process id is minted per derivation and dies with the process — that *was* the defect, so it
cannot key the thing that must survive re-derivation. The anchor is only the smallest client
handle, so a guest that frees that one client silently re-labels its component and slips the
condemnation. The **client set** is exactly what the refresh already matches boundaries on,
so condemnation and process-matching agree by construction and survive every re-derivation a
guest can provoke: re-labels, growth, and splits (freeing a duplicate edge splits the
component; both halves intersect, both stay condemned — they shared the blast radius).

Two further holes turned up while building it. Condemnation must be **monotone over growth**,
or "duplicate into a fresh client, then free the old root" resurrects the process cleanly. And
a *live* process duplicating into a condemned component is a third branch with no good answer:
absorbing resurrects a corpse around a working sandbox, and condemning the live process lets a
guest kill a healthy neighbour by duplicating into a corpse. So the **event** is refused,
atomically, and the refusal lands on the process issuing the duplicate — never on a victim.

An operation against a condemned component resolves *forward* to a named fault, using maps
rebuilt from the same projection that fills the live routing maps. No reverse lookup was
invented in order to produce a prettier error message, which would have violated the address
table's founding directive.

## 2.9 What is not built

Stated plainly, because the rest of the document depends on it being clear.

| Not built | Where it lands | Consequence today |
|---|---|---|
| Real wait loop, wake primitive, source-to-descriptor table | L1 | the reactor has a model and no OS half |
| Real sandboxed isolate processes and their wire protocol | L1 | every "isolate" is an in-process mock |
| Verb interruption (the signal handshake) | L1 | section 3.7 — genuinely untested |
| Memory-mapping / guest-RAM plumbing, the one audited raw module | L1 | the only future `unsafe` in the workspace |
| Register model, MMIO dispatch, `Gpu: Device` | L2 | the core has no register surface |
| GSP boot state machine, mailbox latches, RPC transport | L2 | **the largest un-modelled behaviour in the project** |
| Wire-layout codegen for a driver version | L3 | no wire decode exists anywhere |
| GMMU page-table walker | L3 | page-table-write capture records pages but cannot decode them |
| A real `Arch` implementation | L3 | the only architecture is a deliberate fake |
| Graphics pipeline, video decode, MIG | later | seams typed, pipeline absent |
| **Device reset of any kind** | unassigned | §3.2 G5 — a driver reload leaks the entire device |
| **Eager host-object reclamation** | L1 | §3.2 G1/G2 — not merely deferred; not representable in the current shapes |

---

# 3. Risky areas, uncertain parts, and open questions

This section is the point of the document. Most of it is assembled from the project's own
contact log — `l1_concurrency.md` §12, fifteen entries recording every place where building
the thing proved the design wrong — plus this review's reading of the source. Where the
project has already conceded something, it is quoted rather than paraphrased, because the
concessions are more useful than any restatement of them.

The stance the design takes about itself is worth stating first, since it explains the shape
of what follows:

> This design is not trusted. It is a pure-logic argument plus the predecessor's scar tissue;
> the harsh simulated harness, with its mean assertions, is what the architecture actually
> converges to. Where harness and design disagree, the design moves.

## 3.1 The single biggest risk, in the project's own words

> The design commits to a core ownership refactor, a lock discipline, a re-entrant verb
> shape, **and** an N-worker sandbox protocol on the strength of pure-logic reasoning and
> mock testing, **before any real host operation latency distribution has been measured under
> this architecture.**

If real latencies are much worse — or much *weirder* — than the C artifact's measurements
(roughly 5.4 ms allocations, microsecond controls), the confinement story stays *correct* but
per-operation latency could disappoint, and pressure will mount to widen concurrency further:
bigger pools, dynamic scaling, per-process executors. Each widening re-opens exactly the race
classes the predecessor bled on.

The discipline the design asks for is that widening happens **only through named seams**,
each as its own reviewed change, and **never in a debugging session**. That is a
social control, and social controls decay. It is partly backed by mechanism now — the R1, R3
and R5 assertions and the mean test fail loudly when a widening breaks the rules — but only
partly.

## 3.2 ★ The reclamation story is incomplete — a teardown audit, landed during writing

**Status note.** A read-only audit of the core's teardown and reclamation completeness
finished *after* this document was started. Its findings are **not** in the contact log, and
fixes for four of them were **being implemented in parallel with this section being
written**. Everything below describes the tree at HEAD `3569d46`. Do not read "being fixed"
as "fixed"; do not read "not yet fixed" as "unknown".

The framing that matters: **the derivation logic is strong and heavily verified; the
reclamation logic is not, and is now known not to be.** Section 2 describes what is built and
would, read alone, imply that teardown is complete. It is not. Nine gaps, in descending order
of consequence.

### The four being fixed as this is written

**G1 — a published backing's host memory handle is recorded nowhere.** The commit phase
receives a reply carrying the host address space, the host memory object and the host
address, but on the **success** path it stores only the address into the binding. The
binding type is `{physical, aperture, host address}` — there is no field for the memory
object's identity. The object identity appears only in the *refusal* path's orphan closure.
So any future reclaim path can unmap the address and can **never free the memory** — and this
is the majority of the host bytes the system allocates. Being fixed by putting the backing
identity into the binding.
`kayfabe-fwd/src/lib.rs:514-518`, `:592-607`; `kayfabe-mmu/src/lib.rs:36-46`.

**G2 — the refresh silently drops live host state.** When re-deriving, the process's address
spaces and channels are filtered by retaining the ones the graph still supports. The
discarded values still hold host address spaces, host channels and host engine objects — and
the **process is still live**, so sandbox teardown never covers them either. This is
reachable by an ordinary guest freeing one address-space or channel handle. It is **not yet
fixed**, and the reason is a real design tension: the fix needs a deferred-release queue,
because the no-blocking-under-lock rule forbids issuing host verbs inside the refresh, which
runs under the device write lock.
`kayfabe-core/src/gpu.rs:848`, `:883`.

**G3 / G3b — the reap is both unsafe and violates R1.** Reaping retired processes drops them
*including their sandboxes* in place, and it is called under the device **write** lock. A real
sandbox's destructor waits for the child process and tears down namespaces — so it blocks
under a rank-0 lock. That is a live R1 violation **with no assertion to catch it**: the
lock-free assertion guards *verbs*, not *drops*. This is precisely the shape of the honest gap
the contact log recorded at §12.6 and that stage 3 already fixed once, reappearing in a place
the fix did not cover. Separately, the reap can run while a worker is checked out on another
thread, tearing the sandbox down under a live connection. And the "quiesce point" the design
promises is **defined nowhere**. Being fixed.
`kayfabe-core/src/gpu.rs:1087-1099`; `kayfabe-rt/src/device.rs:411-413`.

**G4 — there is no cancellation vocabulary.** The host error type has no interrupted variant
despite the design specifying one on the wire, and the forwarding fault type has no cancelled
variant. So a cancelled verb surfaces as a generic host error and is re-resolved into a
generic host fault — **the exact wrong-reason conflation** the contact log's §12.10 was
written about. Separately: if a worker dies mid-chain, the internal unwind cannot run, so
everything allocated before the failure is in no orphan set and in no core state. Being fixed
as vocabulary and shape only; the actual cancel mechanism belongs to the next L1 milestone
(section 3.7).

### The five not yet addressed

**G5 — there is no device reset.** There is no reset event and no reset entry point. On a
driver reload in the guest — or a guest panic, or a VM reset — the guest sends no free events
at all. The graph, every live process with its sandboxes and arenas and host handles, both
routing maps, the condemned set, the retired list, the registered completion sources and the
guest-physical window base **all survive into the new driver's life while nothing is
reclaimed.** The C artifact's known limitation — that its emulated firmware state only reset
on a full restart of the emulator — is inherited here **by omission** rather than by decision,
which is worse, because nothing names it.

**G6 — the per-process arena is bump-only.** ★ **FIXED** (`l1_concurrency.md` §12.20).
There was no intra-process free: structurally the same leak the predecessor hit at process
granularity, reproduced at the *intra*-process granularity, so a long-lived process that
mapped and unmapped repeatedly exhausted its own arena (measured: dead at map/unmap cycle
128 on a 512 KiB arena). `GpaArena` now has a coalescing free list and a move-only
`GpaBlock` token — `free` takes it by value, so a double free does not compile — and
`kayfabe_fwd::unpublish_backing` returns the GPA to the proc's own arena together with the
host `Orphans`. Deliberately a free list driven by declared graph facts, **not** a
collector.

**G7 — arena release only checks its range in debug builds.** ★ **FIXED** (§12.19). The
window check was a `debug_assert!`, compiled out in release, so releasing an arena into the
*wrong* window was representable. It is now a loud `Result<(), ForeignArena>` that hands the
arena back, arenas are stamped with their owning target at carve time, and
`Spine::reap_retired` routes each one home **by its own owner** rather than by a map key a
caller supplies. `GpaArena` also lost its `Clone` (a clone is two releases of one range), and
the reap's silently-dropped arena is now reported on `Reclaimed::orphaned()`.

**G9 (security) — unbounded, guest-driven device-global growth.** ★ **FIXED** (§12.21). The
GPU target id derived from a **guest-supplied 32-bit value** and first touch minted a fresh
window and delivery plane, uncapped and unvalidated. The cap is now the **entitlement** — the
roster the device was realized with (`Gpu::realize`) — enforced at the `Device` alloc where RM
enforces it, with `RmGraphError::InvalidDeviceInstance` standing in for `NV_ERR_INVALID_CLASS`.
Deliberately *not* `NV_MAX_DEVICES`: RM already bounds the field to `< 32`, so that cap would
still allow 31 windows on a single-GPU box. Our own `unwrap_or(0)` default-to-GPU-0 guess is
gone with it — an undeclared instance is unroutable, never GPU 0.

**G10 (security) — the condemned set and the retired list are unbounded**, and the condemned
set is scanned as (boundaries × condemned) on every apply. ★ **FIXED** (§12.22), and the scan
was worse than reported: the *carry-forward* was O(n² log n) per apply (measured 55 s at the
cap), now near-linear via union-find over entry indices. Both lists have named caps, and the
refusal lands on **deriving a new `Proc`** — never on the condemnation itself, because
refusing that would un-condemn a component whose isolate is already dead.

### ★ The methodological finding, which is worth more than any individual gap

The audit found **six places where a document asserts something is fine and it is not.**
Three examples:

- The consolidation document's deferred list says eager host-side reclamation is *"fine for
  correctness, wired for footprint later"*. **Both clauses are false**: G1 and G2 mean it is
  not merely deferred, it is unrepresentable in the current data shapes, and the consequence
  is a correctness-relevant leak of host objects, not a footprint optimization.
- The orphan type's own documentation claims *"both dispositions are decided, neither is a
  leak"*. A worker dying mid-chain is an unnamed **third** disposition, and it does leak.
- The arena module says its release is *"safe by construction"* — a claim about a destructor
  in an adapter that the core neither performs nor checks.

The general point deserves stating on its own: **this project's documentation has a
systematic tendency to drift *optimistic* relative to its code.** Section 7 of this document
records eleven more instances of the same drift found independently. That matters more than
any single one of them, for a specific reason: this project's principal defence against its
own mistakes is that its design documents are unusually honest, and that its contact log
records where the design was wrong. Both of those instruments lose their value in exact
proportion to how often a document says "handled" about something that is not. The drift is
not carelessness — every instance above was true when written, and became false as the code
moved underneath it. That is precisely why it needs a mechanism rather than an intention.

### What this changes about the assessment

Nothing in section 2 is wrong, but its emphasis is. The core's *derivation* — graph,
projection, routing, address table, isolation — is the part that has been fuzzed,
property-tested, mutation-gated and red-teamed. The core's *reclamation* — what happens to
host objects when things go away — has had one campaign (the predecessor's window-exhaustion
regression, section 5.7) and that campaign found a live bug. This audit is the second
campaign, and it found nine more.

That is not a surprising ratio, and it points somewhere useful: **teardown paths are
under-tested relative to construction paths across this whole codebase**, and the reason is
structural rather than accidental. Construction is what the mean test drives 21,000 times.
Teardown is what happens once, at the end, when the assertions have mostly already run.

## 3.3 R1's cost: re-entrancy everywhere a verb is issued

R1 — no blocking under any lock — resolved the sharpest problem the previous review found.
Under the earlier design a process's operation held the device *read* lock across its
blocking host call, and the completion pump is a *writer*; so one process's 5.4 ms allocation
(or its multi-second worst-case interrupt unwind) delayed **every** process's completion
delivery and all control-plane progress. That was the predecessor's de-facto global
serialization, rebuilt at the lock layer.

The fix is real, and it is not free. Every verb-issuing path is now a three-phase, re-entrant
shape. The costs, named:

- **Each rank is taken twice per operation**, where the in-lock shape took it once — and more
  than twice on the error and retry paths (a verb error re-enters to return the worker and
  again to check liveness; a refused commit re-enters again; a retry runs the whole thing
  again). A test asserts that *count*, not merely the absence of a panic, because collapsing
  it back to one is precisely the regression R1 exists to prevent. **Note:** the design
  document states "twice" as an invariant; it holds only for the uncontended success path
  (section 7.10).
- **Every gap is now a staleness window.** That is section 3.4, and it is the trade.
- The hot path pays it too. The design's staging plan hoped the doorbell would not need the
  split; the build checked, and it does — because the sandbox is a *separate process*, so even
  ringing a doorbell is an inter-process round trip rather than a store. What survives is that
  a steady-state doorbell is a single-verb chain whose commit records one field, and a re-send
  needing no host work emits no verbs at all and never touches the pool. So the cost on the
  hot path is two microsecond-scale locked phases around one round trip, not a second round
  trip.

One structural simplification is load-bearing and was **checked rather than assumed**: no
verb site needs to consult core state *between* two verbs. Each chain is purely
verb-output-to-verb-input, with every core-derived value known at plan time. That is what
makes the seam a short typed struct instead of a resumable continuation machine — and if a
future verb site breaks that property, the seam's complexity changes category.

## 3.4 R5 and staleness: the risk deliberately accepted

The design names this as bet B5, and it is the most interesting risk in the project:

> The drop-lock discipline trades convoy and deadlock risk for **staleness** risk. R1 removes
> lock-holding across blocking calls; the price is that every gap is a window where the world
> changes, and every commit is a site that must re-validate. **A forgotten re-validation is a
> use-after-retire — *quieter* than the deadlock it replaced.**

Quieter is the operative word. Deadlocks announce themselves. A commit that writes a reply
into freed state does not.

Two contact-log entries show this is not hypothetical.

**The rule as written was half a rule, and the missing half was the dangerous one.** R5
originally said: if re-validation finds the target gone, refuse. Building it that way
immediately broke a multi-thread test, and the reason is worth stating precisely. Lazy
materialization — of a host address space, a host channel, an engine object — reads "is it
there?" under the plan lock and writes "here it is" under the commit lock, with a lock-free
gap in between. That is a **compare-and-swap**, and two sibling threads of *one* guest process
racing it is the *ordinary* case for exactly the workload this design exists to serve.
Refusing the loser turns a legal concurrent submission into a spurious guest-visible fault:
worse than the use-after-retire R5 was written to prevent, and visible only under
concurrency. So staleness now has two shapes — *converging* (a sibling got there first;
release the duplicate and re-plan against the winner) and *divergent* (process retired,
channel torn down, route rewritten; refuse loudly).

Note the shape of that failure. **The first symptom of getting this wrong was a hang, not an
assertion** — a worker thread panicked on a staleness variant its expectation did not
tolerate, poisoned the device lock, and left the executor spinning.

**R5 applies to the failure path too.** A verb held in flight across a retire returns a host
error and never reaches the commit at all. Loud and mutation-free — but the fault the guest
saw said "the host failed" when the truth was "your process was torn down". A canary
asserting only *that it refused* would have passed for the wrong reason. That incident is the
origin of the house rule described in section 5.5: **assert the exact fault variant, never
`is_err()`**.

### The named, untested residual

The converging retry is bounded at eight passes, because each pass observes a strictly more
materialized world — one pass is the expected worst case, and the bound exists so a bug
cannot turn a race into a spin.

**That bound is not tested.** The mutation gate found it and left it open, named:

> `retries += 1` → `*= 1`, and `retries < MAX` → `<=`. These bound the converging-staleness
> re-plan loop; the first pins the counter at zero and makes the bound vacuous, so a
> pathological cycle becomes a spin instead of a surfaced fault. Killing them needs a mock
> that can force a *repeated* rebound — today's recorder injects one-shot host errors, not a
> persistent commit race — so the test would be new **machinery**, not a new assertion.

This is the honest core of the L1 mutation score: the untested *logic* on that surface is two
mutants wide, and it is the retry bound. Two mutants is not much; a spin under a pathological
race is not nothing.

## 3.5 The worker pool re-admits concurrency (bet B6)

The original decision was one single-threaded worker per sandbox, strictly one verb in
flight. It came free from the type system and it deleted the predecessor's entire thread-pool
and slot-mapping apparatus, itself a proven bug source.

The revision re-admits threads into the sandbox process, **deliberately**, because
single-in-flight-per-sandbox cannot satisfy the intra-process invariant: a multi-threaded
guest process is *one* process with *one* sandbox, so thread B's verb would queue behind
thread A's.

The bet, stated:

> Channel-COUNT concurrency is categorically safer than channel-MULTIPLEXED concurrency —
> believed strongly, argued from the type system, proven only by the mean test.

The argument is good. There is no shared in-flight table and no transaction demultiplexing;
each worker owns one channel end to end; the only thing workers share is the driver file
descriptor, which is kernel-mediated, and concurrent operations on one RM client are ordinary
host behaviour that multithreaded CUDA does all day. But the honest form of the claim is that
a bug *class* was structurally excluded and the *conclusion* is proven only against mocks.
Real sandbox processes with real threads and real signals do not exist yet.

## 3.6 The system-process stall (bet B3) — an explicitly open owner decision

Kernel and copy-utility traffic routes to a distinguished *system* process, whose sandbox is
as stall-prone as any other. A host operation stuck in uninterruptible sleep on a *user*
process stalls that process's calling thread — contained, by design. On the **system**
process it stalls system-traffic forwarding.

R1 materially improved this: the stalled system verb now holds no lock, so device-wide
operations and the completion pump proceed freely, and sibling system verbs proceed on other
pool workers. What stalls is only the specific operation whose worker is wedged.

The residual mitigation is a watchdog that interrupts and retires a verb exceeding its
budget, converting an unbounded stall into a bounded, *loud* failure. That is the best
available — a host thread in uninterruptible sleep is un-killable by anyone; the predecessor
wedged an entire GPU on these.

**Two problems.** First, this is recorded in the decision ledger as **still open**: accept as
residual, or demand a stronger story. It has not been closed. Second, and more concretely:
**the watchdog and the interrupt mechanism do not exist** (section 3.7). The mitigation for
the open residual is itself unbuilt.

## 3.7 Interrupt-mid-verb: genuinely untested, and in fact absent

The design specifies verb interruption in detail — the predecessor's hardest-won lesson, since
it had both a guest task that could not die while blocked in a forwarded operation *and* a
use-after-free in the naive fix. Every request carries a transaction id; the worker installs a
non-restarting signal handler; an interrupt makes the blocked operation return early; the
worker replies on the normal path; and **the requester never abandons the reply buffer** — it
blocks (holding no lock, bounded by the unwind, measured at roughly 3.5 s worst case in C) for
the reply, then surfaces the refusal.

The project states the test status plainly:

> §5.4's interrupt-mid-verb has no cancel API yet, so it is **genuinely untested**.

This review's reading of the code makes it stronger than untested. In the sandbox port, the
word "transaction" appears exactly once — in a comment saying there is *no* transaction
demultiplexing. There is no interrupt method, no interrupted reply variant, and no interrupted
error variant. Retiring a sandbox is documented as "stop accepting new operations, begin
quiescing in-flight work", and against a mock there is nothing to quiesce.

So section 5.4 of the design is not a lightly-tested feature. **It is a specification with no
implementation.** That is legitimate for an unbuilt layer — but it means three things
simultaneously depend on unwritten code: the interruptibility story, the teardown-under-load
story, and the mitigation for the open B3 residual.

## 3.8 The single executor as a completion funnel

All interrupt-route completions serialize through one thread that also runs deferred reaps
and lock faults. A slow event delays every other process's delivery — an echo, in miniature,
of the starvation shape the design otherwise kills.

The stated mitigation is that observation edges also fire from the guest's own threads (its
own poll, and the interrupt-clear path), so every completion pattern has a non-executor edge
or a bounded executor hop. Per-process executor sharding is named as a future seam if a
measured workload ever shows executor latency — and explicitly not before.

The mitigation is sound in design. **In the code it is partly unwired**: of the four pump
edges the design names, the shell wires the poll edge and the deferred backstop. The
observe-then-pump edge and the drain edge are absent, and the backstop pumps a hardcoded GPU
zero. Section 7.1 and 7.3 are the details; the risk is that the funnel's mitigation is one of
the pieces still missing.

## 3.9 Mutation score: 92.44% on L1 against 99.2% on L0

The L1 surface scores lower than the pure core, and the project argues the gap is
composition rather than negligence. The thirteen survivors are: **two proven equivalent**,
**six diagnostics-only** (they change only the text of a panic that is already firing, or a
method with no production caller), **three spin-versus-park** on the pool backpressure gate,
and **two real** — the retry bound of section 3.4.

The reasoning for accepting the six diagnostics-only survivors is worth endorsing: pinning
the text of a panic message would couple the invariant suite to a diagnostic, and *"hollow
tests to raise a number are worse than an honest gap."*

The three spin-versus-park survivors are more interesting. They degrade the pool's
backpressure from *parking* to *spinning*: the waiter returns immediately and re-enters from
the top, which is still correct and still terminates. What changes is CPU burn, and the suite
asserts backpressure as progress, never as wall clock — deliberately, because timing
assertions are how concurrency suites become flaky. So the gate cannot see the difference
between a well-behaved park and a busy-wait. **Given that "never busy-poll anywhere" is one
of this project's founding rules — its predecessor's worst performance bug was a poll storm —
having a blind spot exactly there is uncomfortable**, even if each individual decision that
produced it was right.

The CI threshold is 91%, not 92.44%, because that cluster is scored by timeout and measurably
moves by about two mutants between identical runs. The reasoning is explicit and good: a bar
set at the measurement goes red on jitter, and a gate that cries wolf gets muted — *which is
exactly how this surface reached master with no mutation run in the first place*.

## 3.10 Performance is entirely unmeasured under this architecture

There is no performance number in this repository. Not one. Every performance claim in the
project's documents is inherited from the C artifact.

Two specific reasons to be careful with the inherited numbers.

**The trap-minimization premise (bet B2)** — that contended lock paths stay off the
steady-state hot path because the hot path has almost no traps — was proven for the C artifact
*on bare metal*. The Rust system will first run under nested virtualization, where the cost of
exiting the guest dominates everything. Conclusions from that bench must be read through that
filter, or **a correct design will look falsely slow**. The predecessor has a documented
instance of exactly this: a correct trap-elimination showed zero win under nesting.

**The lock design has never been under real load.** Both modes are exercised by a mean test
that runs about 21,000 operations in under a second against in-process mocks. That proves
progress and correctness. It says nothing about contention profiles when a lock section calls
a real memory-mapping syscall or a real inter-process round trip.

One property change is already known and named: the core advertises that any number of
threads may share the device and resolve or route in parallel, lock-free. That survives the
device-global spine, but once each process lives in a mutex cell, a per-process *read* must
take that process's lock — and in degenerate mode it takes the device *write* lock. The reads
are microseconds and usually uncontended, but the property as advertised is now false in both
shipping configurations (section 7.9).

## 3.11 The whole L2 and L3 surface is unbuilt

Section 2.9 lists it. Three items deserve individual attention as risks rather than as
missing work.

**The GSP boot and reboot lifecycle is the largest un-modelled behaviour in the project.**
The crate is a 34-line placeholder enum. The C artifact fought this hard: firmware unload,
protected-region teardown and re-establishment, security-processor mailbox latches, a
sequence-number-preserving initialization re-post, and a genuinely nasty
reload-detection bug. The regression matrix classifies this as its **one honestly-open
lifecycle exposure**, and it is right to: no test here can guard it, because a mock of an
unwritten state machine tests the mock. Its planned oracle is trace replay against the C
emulator's recorded boots — which is a good plan, and is a plan.

**The page-table walker is a 41-line skeleton.** The pushbuffer parser already captures
dirtied page-table pages per address space; decoding them into bindings needs the walker.
The C artifact's most expensive single bug was a walker that did not know about one leaf
page size on one architecture and therefore **silently dropped** the page table writes it
could not decode. The requirement is codified — an un-enumerated leaf size must be a loud
fault, never a silent drop — but codified is not implemented.

**The memory-safety surface does not exist yet, and neither does its audit.** The core's
threat model is explicit that memory safety, out-of-bounds access and host breakout are out
of scope *because the pure core has no pointer to get out of bounds and no `unsafe` to be
unsound*. That is honest, and it means the entire memory-safety story is deferred to the one
audited raw module — memory-mapping guest RAM into sandboxes, volatile access to pages the
GPU writes concurrently, hypervisor ioctls — that has not been written. The debt starts
accruing the moment it is.

Worth recording precisely, since it is often stated loosely: `unsafe_code = "forbid"` is
declared once as a workspace lint and every member opts in. **There is no `#![forbid]`
attribute in any source file**, and there are **zero `unsafe` blocks anywhere in the tree**,
including the fuzzing workspace — the unsafety there lives inside the libfuzzer dependency.

## 3.12 No real host GPU has ever run this Rust code

Stated without qualification because it is the fact that contextualizes everything else.

Nothing in this repository has ever touched a GPU. There is no adapter that could. The
203 tests run against deterministic mocks; the mocks were written by the same people who
wrote the code they check; the only architecture that exists is a deliberate fake with
non-NVIDIA encodings.

What that leaves the project standing on is genuinely substantial — order-independence
properties, differential oracles, a 25-row regression matrix built from a *working* C
implementation's real failures, hostile-input fuzzing, two mutation campaigns, a race
detector run, and a composed adversarial integration test that has already found a real
defect. That is a lot more than most pre-hardware code has.

But it is all *one kind* of evidence. Every one of those gates checks the implementation
against the team's own model of NVIDIA's behaviour. **None of them can catch a wrong model.**
The C artifact's expensive bugs were overwhelmingly wrong-model bugs: a walker that did not
know a leaf size existed, a channel put on the wrong runlist because an engine tag was
dropped, a golden context that could not be fabricated. A mock cannot surface any of those,
because the mock encodes the same belief the code does.

The mitigation named in the plan — trace replay against the C emulator's recordings — is the
right one, because the C artifact's traces are the only ground truth available without
hardware. It has not been built.

## 3.13 What the earlier review flagged, and where it stands

For continuity: the previous version of this document listed five pressure points. Their
status today is a reasonable proxy for whether review pressure actually moves this project.

| # | Pressure point | Status |
|---|---|---|
| P1 | The device read lock held across a blocking verb couples every device write — including the completion pump — to the slowest verb in flight | **Resolved** by R1. This was the sharpest finding and it produced the largest design change (the plan/execute/commit seam). |
| P2 | The observe-then-pump lock transition is a convention that one convenience refactor could deadlock | **Resolved** by R3 as an always-on ranked assertion — the panic fires the first time someone pumps under a held process lock. Note the edge itself is not yet wired (§7.1). |
| P3 | The route-then-act window is a staleness gap with ad-hoc re-checks | **Resolved in kind** by R5 as an asserted invariant with canaries — and, in resolving it, discovered that the rule as written was half a rule (§3.4). |
| P4 | The single executor is a completion funnel | **Accepted as residual** with named mitigations; partly unwired (§3.8). |
| P5 | A late lock-granularity flip means the sharded mode is the untested one when it matters | **Resolved** — both modes run in every threaded tier from day one, with a differential assertion. |

Three of five produced structural changes rather than reassurances. That is the most
encouraging signal available about how this project responds to being told it is wrong.

## 3.14 Risks the project has not named

Everything above is the project's own list plus this review's reading. Three additions.

**The mocks and the code share an author and a model.** Section 3.12 covers the wrong-model
risk; the sharper version is that the mocks encode *design intent*, so a test failure means
"the code disagrees with what we meant", never "what we meant is wrong about NVIDIA". The
differential oracle against the C artifact is the only planned instrument that can produce
the second kind of failure, and it is unbuilt.

**The always-on assertions are a production panic policy, not just a test tool.** R1, R3 and
R5 panic in release builds. That is argued well and this review agrees with the argument. But
it means a lock-discipline bug reaching production is a *crash* of the process serving every
guest on that host — which is the right trade for a correctness bug and a poor one for a
false positive. Nothing in the tree yet exercises what a panic does to the guests attached to
a device, because there are no guests.

**Unbounded source registration is a named boundary gap.** The completion-source registry has
no cap on registered sources, recorded in the source as an honest deferral. It becomes live
the moment source arming becomes guest-driven — which is exactly what L1's real reactor will
make it. Every other guest-growable table in the core is capacity-bounded; this one is the
exception, and it is the one that arrives with the untested layer.

---

# 4. Design decisions

The numbered ledger, with the cost of each stated as plainly as the rationale. Decision
numbers are the project's own.

### D1 — OS threads with fixed roles; not async, not a second state machine (#34.2, confirmed)

**Decision.** A small fixed set of thread roles: guest-processor trap threads, one reactor
loop, one serialized executor, and worker threads inside each sandbox. No async runtime. No
atomics, no lock-free structures, no hand-rolled synchronization in L1 logic — only the
standard library's locks, condition variables and queues.

**Rationale.** Four arguments, in the design's order of weight. The core is synchronous pure
logic and the ports are synchronous traits, so async infects every port signature for zero
core benefit. The genuinely blocking things do not become asynchronous under an async
runtime; they become a thread pool in disguise, with a scheduler we do not control between us
and determinism. The predecessor's lesson set contains **zero** "not enough concurrency" bugs
and six "concurrency wrongly shaped" bugs. And a hung guest is diagnosed from thread
stacks — thread-per-role makes a hang *legible*; a task soup does not.

The no-atomics rule has a second purpose: it keeps a race detector meaningful as the ceiling
and keeps exhaustive concurrency model-checking unnecessary *by rule*. The standing note is
that a model checker becomes mandatory if a lock-free path is ever introduced — so the rule
is: do not introduce one.

**Cost.** Thread count scales with guests. The blocking round trip occupies a real thread for
its duration. And a single-threaded event loop — which would be maximally deterministic —
was rejected because a guest exit is a *synchronous upcall*, so a single loop would put a
cross-thread round trip on the one mandatory hot-path trap. That loop does exist in this
design, as the deterministic test harness, where it belongs.

### D2 — A hexagonal core with the logic OS-free (#1/#2)

**Decision.** One pure logic region; every effect crosses a trait. OS code lives only in
adapter crates.

**Rationale.** Deterministic testability without hardware (section 1.7), plus the seam that
makes a new GPU generation an `impl` rather than a refactor.

**Cost.** Real. Ports must be designed before their adapters exist, which means designing
against an imagined implementation — and three port groups are still unwired because their
adapters do not exist. The core cannot implement the hypervisor's device trait yet, so the
adapter-facing surface is provisional. And a boundary maintained by review decays, which is
why the vocabulary rule is a CI grep rather than a convention.

### D3 — Protocol, not trace (#4/#27)

**Decision.** Derived state is a pure function of declared facts, never of arrival order.
Facts naming absent targets park rather than drop.

**Rationale.** It is what real hardware semantics allow, it is what makes concurrent delivery
safe, and it is what makes single-threaded scripted testing of interleavings *sound* rather
than merely convenient.

**Cost.** Parked-fact machinery is the richest bug cluster the core has produced. The mutation
gate found eight distinct predicates in parked-map cleanup and free-subtree cascade that no
existing test pinned exactly; the security red-team found two *wedge* bugs whose common root
was a parked fact that resolves during an unrelated operation, produces a fault, and is
restored by that operation's rollback — so it re-fires forever, turning a contained refusal
into a device-wide denial of service. Both instances are fixed at their resolution sites; a
systemic guard is named as a candidate hardening if new parked-fact kinds are added.

### D4 — MISS = FAULT, everywhere (the address-table directive)

**Decision.** Forward-populate only. No reverse resolution, no heuristic pick, no
most-recently-used fallback, anywhere. Applies to addresses, routing, completion sources,
doorbell tokens and post-relock staleness alike.

**Rationale.** Section 2.3. Every fallback in the predecessor was individually reasonable and
collectively catastrophic.

**Cost.** The system is loud. Any place the model is incomplete surfaces as a guest-visible
fault rather than a degraded guess — which is correct, and which will make early hardware
bring-up noisier than a forgiving design would. The project treats that as the feature.

### D5 — Per-process sharding, with both lock modes shipped (#34.1/#37, decided)

**Decision.** A device reader-writer lock over the spine plus one mutex per guest process,
governed by three asserted invariants (no blocking under a lock; ranked acquisition;
re-validate after re-lock) and two structural rules (locks bracket bookkeeping only; mixed
paths are route/act split). Both the degenerate one-lock configuration and the real sharded
one ship and are tested from day one.

**Rationale.** The lock layout maps one-to-one onto the ownership layout the core already
has, so there is no impedance mismatch to maintain. Device-wide operations are rare and
coarse by design. Containment becomes a lock property: a hostile guest spamming refusals
contends only the write slot and cannot occupy another process's shard.

**Cost.** A core ownership refactor was required before any L1 code could be written — the
route/act split of every mixed entry point, and separating the device-global spine from the
process set. It landed behaviour-preserving with the suite green throughout, but it is the
clearest case in the project of L1 concerns reaching into L0's shape.

### D6 — The verb seam is plan / execute / commit (#37, revising #34.3)

**Decision.** A locked phase emits a verb description; the calling thread executes it
lock-free; a re-locked phase re-validates and commits.

**Rationale.** R1's direct consequence. The alternative — keeping verbs in-line under the
process lock — was the P1 hole and an intra-process violation.

**Cost.** Section 3.3. Re-entrancy at every verb site, double lock acquisition, and staleness
as a standing hazard. The design concedes the trade in its own words: the seam was originally
held in reserve as an emergency exit and is now the standard shape — *applied uniformly by
design rather than retrofitted per-verb under debugging pressure, which was exactly the
failure mode the original text feared about its own fallback.*

### D7 — A bounded pool of single-in-flight workers (#37, superseding #34.4)

**Decision.** N workers per sandbox, each strictly one verb in flight on its own one-deep
channel. Interface designed for N from day one; pool statically bounded first; dynamic
scaling only on measured need.

**Rationale.** Section 3.5. The intra-process invariant is unmeetable with one worker.

**Cost.** Bet B6. Premature dynamic scaling is explicitly named as a complexity trap — a
spawn and reap policy, thundering-herd wakeups on growth, worker-lifetime races: all cost, no
demonstrated benefit. Pool exhaustion is deliberately left as backpressure.

### D8 — Edge-driven completion delivery; a periodic sweep is forbidden (#34.5)

**Decision.** The pump runs on edges only, plus a deferred backstop armed *only while
completions are outstanding*. A periodic redelivery sweep is forbidden.

**Rationale.** The predecessor's worst performance failure was a poll storm; the forbidden
thing is specifically *"the harmless safety poll that historically creeps in during a
debugging session."*

**Cost.** Every completion pattern must have an edge, and if one is missed the failure mode is
a lost wakeup rather than a slow one. Two of the four named edges are not yet wired (§7.1).

### D9 — The reactor's purity boundary, enforced by a CI grep (#37.7)

**Decision.** The core owns the source model, the registry, the dispatch and an abstract
notifiable source. L1 owns descriptors and the wait loop. The core says "notifiable source"
and "completion source", full stop.

**Rationale.** Section 2.7 — hexagonal fit, and it is what makes completion interleavings
mock-testable with zero syscalls.

**Cost.** An extra indirection on every completion, and a vocabulary rule strict enough that
comments have been reworded to satisfy it. The grep gate is coarse — it matches six tokens —
and section 7.7 records that the module's own absolute claim about itself is stronger than
what the gate enforces.

### D10 — Condemned components keyed on the client set (#37, from the mean test's finding)

**Decision.** An out-of-band retire condemns the component, keyed on its client set;
condemnation is monotone over growth, clears only when the guest frees the client root, and a
live process duplicating into a condemned component refuses the *event*.

**Rationale.** Section 2.8. Without it, "no resurrect" lasted microseconds.

**Cost.** New device-global state that must stay canonical and disjoint, a new fault variant,
two new routing maps, and a third branch in the merge logic whose refusal must land on the
attacker rather than the victim. Its residual is named: a condemned component's reported label
moves if the component grows a client with a smaller handle, because the label is derived by
definition.

### D11 — arm64 as a guarded seam, not a bolt-on (#36)

**Decision.** Build nothing for arm64. Instead, cross-compile-check the whole workspace for
aarch64 on every push, and write down the one binding rule now: **the host page size is
queried at runtime, never a hardcoded 4096** — arm64 hosts run 16 KiB or 64 KiB base pages,
and the guest-RAM windowing machinery aligns and slices by page size.

**Rationale.** arm64 plus NVIDIA is a real and growing configuration. `cargo check`
type-checks without linking, so the gate needs no cross-toolchain and no emulator: it is
nearly free, and it structurally proves the core stays architecture-portable — the moment a
CPU-architecture assumption enters a core crate, the gate fails on the push that introduced
it.

**Cost.** Almost none today. The real cost is deferred and is a *discipline*: the page-size
rule must be honoured when the memory-mapping layer is first written, and this document is
the only thing carrying that requirement until an L1 mapping design exists.

The same pattern governs MIG — with a useful piece of honesty attached. The tempting shortcut
was "a MIG slice is just another device node, so multi-GPU gets it for free." That premise is
wrong twice: MIG is datacenter silicon that the target commodity hardware does not have at
all, and a MIG instance is not a device node but a partition subscription plus capability
files. So the target abstraction is left MIG-*accommodating* (a routable target, not
"a physical device") and **none of MIG is built or tested** — no pretend tests on absent
hardware.

### D12 — `forbid(unsafe_code)`, with one future audited raw module (#16/#16b)

**Decision.** The workspace forbids unsafe code. Exactly one future module will hold the
operations that cannot be safe — memory-mapping guest RAM, volatile access to pages hardware
writes concurrently, hypervisor ioctls — and it will expose only a bounded-object API with
no raw pointer escapes and compile-fail tests asserting that the dangerous patterns do not
compile.

**Rationale.** The core has nothing to be unsound about. Concentrating the unsafety makes the
audit finite.

**Cost.** The fuzzing harness needs unsafe (its entry point and FFI), so it lives in a
**separate workspace** — which means its own format check in CI, its own toolchain, and no
lint coverage. And the audit debt is real and unstarted: the memory-mapping plumbing is the
memory-safety breakout surface the threat model defers, it is *born* in L1, and its API review
is named as part of L1's exit gate rather than a later cleanup.

### D13 — Testing decisions

Grouped, because they are one policy. Each is developed in section 5.

- **The mean integration test is the arbiter** (#37.8). Where design and harness disagree, the
  design moves. It is L1's exit gate.
- **Progress under pending, never wall clock** (§8.3/§8.4). No sleeps, no thresholds, no
  "finished within X".
- **Assert the exact fault variant, never `is_err()`** — learned from a canary that passed for
  the wrong reason.
- **Mutation testing and the race detector are standing CI gates, not remembered ones** — the
  change that produced this rule is that about 9,100 lines of L1 code reached master with
  neither.
- **One slow-test flag**, and gated tests print how to run themselves rather than vanishing.
- **Bite-checking**: a new gate is not trusted until it has been made to fail on purpose.

---

# 5. What the tests cover, and how

Two hundred and three tests is not the interesting fact. How they are constructed is.

## 5.1 The harness: no GPU, no OS, no syscalls

Every port has exactly one implementation and it is a deterministic mock. There are no files,
no sockets and no wall clock anywhere in the test system.

| Mock | What it fakes | What makes it deterministic |
|---|---|---|
| `MockArch` | a complete GPU generation, "Mockingbird" | its encodings are **deliberately not NVIDIA's**, so code secretly assuming a real bit layout fails immediately |
| `MockVmm` | the hypervisor | guest RAM is a sparse byte-keyed map; **time is a value** that moves only on `advance()`, with deferred events firing in deadline order; every interrupt, memory slot and trap is recorded in order |
| `MockRmBackend` | one host driver connection per worker | every minted identity is namespaced by `(sandbox, GPU)` in disjoint bit lanes, so cross-sandbox reach is *visible* rather than merely refused |
| `MockIsolate` | the sandbox and its pool | slots are idle / busy / **dead**; a dead slot is never resurrected; a retired sandbox refuses checkout |
| `VerbHold` | ★ the pending-verb latch | a condition variable pair: the backend parks inside a verb, the test observes the *edge* that it parked |
| `RmRecorder` | the audit log | every verb in global order, with an injectable one-shot host error |

Two properties of this harness do the heavy lifting.

**Identities are namespaced so that leaks are observable.** Handles, work-submission tokens
and mapped addresses are minted in disjoint bit lanes per sandbox and per GPU, with the
carry-free arithmetic argued in the source and pinned by a unit test. That turns "no handle
minted for process A is ever observed in process B's state" from a hope into a direct
equality assertion.

**Time is a value.** Deferred events fire in deadline order when a test advances the clock —
the semantics a real adapter must match. There is no timer anywhere.

There is one documented harness artifact worth knowing, because it propagates: the mock's
guest RAM is a byte-per-node tree, which makes hostile pushbuffer ranges expensive to read.
That is the entire reason one test takes 73 seconds, which is the reason a slow-test flag
exists, which is the reason a nightly job exists, and which — via section 5.11 — briefly
hollowed out a security gate. **The 73 seconds is a mock cost, not a core cost**; fixing the
mock would ungate the test.

## 5.2 Why single-threaded testing of concurrency is sound here

This is the argument the whole strategy rests on, so it is worth stating carefully.

The core is order-independent: derived state is a function of the facts, not of their
arrival order. Therefore **any real interleaving's facts can be presented in any serial
order and must yield the same derived state.** Multi-processor interleavings can be driven as
*scripted call orders* from one thread, and that is not an approximation — it is exact, given
the property.

The property is not assumed. It is proven by a differential suite that snapshots everything
an adapter can observe off the device — address resolutions, per-channel engine routing,
per-address-space backing, refcount liveness, both routing maps, per-process grouping — and
asserts the snapshot is identical across permutations, interleavings and benign duplication.
The snapshot is canonicalized on **stable identities** (anchors, page directory bases, channel
ids, node keys) and deliberately **not** on minted counters, which are order-dependent by
design and would leak arrival order into the comparison. It uses ordered maps throughout, so
no iteration-order artifact can make the *test* flaky rather than the *core* wrong.

The tiers follow from this:

- **T1 — deterministic single-threaded.** The default and the vast majority. A scripted event
  loop, a virtual clock, an inspectable and permutable queue.
- **T2 — real-thread stress.** Real locks, real threads, mocks underneath, sixteen threads,
  watchdogs, bounded iteration counts, and a race detector as the ceiling. This is where the
  *shell* is validated — lock acquisition order, condition-variable wakeups — the one thing
  T1 cannot reach.
- **T3 — exhaustive model checking: not applicable by rule.** Atomics and lock-free
  structures are forbidden in L1 logic, and if that rule is ever revisited, a model of the
  new path is the mandatory toll.

The bet here (B4) is named honestly: scripted-order testing is a faithful proxy *given* the
order-independence property plus the thin-waist rule that all L1 logic lives in plain
synchronous functions with no thread, clock or descriptor types in their signatures. Its
blind spot is the shell itself, covered only by T2 and the race detector. Which is why
**keeping the shell small is a correctness strategy, not a style preference** — shell growth
in review is a smell.

## 5.3 The mean test, and progress-under-pending

The arbiter. One composed harsh run, not a pile of isolated cases. Its mandate is worth
quoting because it explains the design:

> Assume you make mistakes; do not trust yourself during testing. If you have real,
> production-like, harsh simulated workload tests with strong assertions, the architecture
> will converge to what passes.

**The world.** Six guest processes, two mock GPUs, three identity lanes, eight threads at
peak, five host verbs parked simultaneously. Roles: a *witness* process with four of its own
threads (one parked in a held verb, three running mixed workloads); a *peer* making full
progress on the other GPU, also multi-threaded; a process retired mid-flight; a process whose
channel is torn down in the gap; a process whose routing is rewritten in the gap; and a
process whose sandbox worker dies out of band.

**The identity-lane trick.** Each lane is a `(page directory base, graphics channel id, copy
channel id)` triple, and each lane's byte-identical values are handed to one process on GPU 0
*and* one process on GPU 1. That is legal — those are per-GPU namespaces — so any routing map
that lost its GPU key mis-routes immediately. On top of that, all six processes share
byte-identical guest RM handle values, so handle keying is under load at the same time. And
process identities are never assumed from mint order; they are resolved back through the
routing map.

**Three workload shapes run concurrently**: a control-plane thread doing allocation and
mapping churn — which crucially drives device **writes**, described in the source as *"the
sharpest probe there is, because they prove the parked verbs hold no lock at all, not merely
no process lock"*; a submission thread doing 4,000 doorbells including the ring gate's
*negative* probe with an exact-variant assertion; and a poll-only thread that is the sole
driver of its GPU's completion plane.

### ★ Why progress-under-pending, and why timing thresholds are not sound

This is the most transferable idea in the project's testing.

The obvious way to test "A's blocking work does not stall B" is to time it: start A, start B,
assert B finishes within some bound. **That test is unsound in both directions.** It passes on
a fast machine even when B genuinely did queue behind A, because the queue was short enough to
fit inside the threshold. It fails on a loaded machine, under a sanitizer, or in CI, when
nothing is wrong. A concurrency suite built on thresholds becomes flaky, and a flaky suite gets
muted — at which point it is worse than no suite, because it launders the absence of a signal
as a green tick.

The alternative asserts an **ordering of events**, not a duration:

1. A scripted latch **holds thread A's verb pending inside the mock backend**, and the test
   blocks until the verb has *genuinely entered* the backend. That edge — observed, not
   waited-out — replaces the sleep.
2. While it is held, the other threads run their full workloads and their joins return.
3. **The assertion is that after every workload thread has joined, every latch is still
   pending.** That is a single boolean over the latches: entered, and not yet released.

The logic is exact. If the workloads only completed *after* a parked verb was released, the
latch would no longer be pending at the moment of the assertion. So the test proves the
*ordering* "workloads finished, and then latches released", which is precisely the invariant.
It **cannot pass because the box was fast**, and it cannot fail because the box was slow. It
is deterministic on any machine, and it survives a 20× sanitizer slowdown unchanged.

Delivery is asserted the same way: each process's own synchronization thread observed its own
completion *while a sibling's verb was parked* — which exercises the structural claim that the
completion path needs no worker at all.

The failure mode is a hang rather than a wrong answer, and a watchdog converts it: if the
no-blocking-under-lock rule regressed, or the pool regressed to one worker, the sibling
threads block and the joins never return. **This was verified by falsification** — shrinking
the pool to one worker makes the test abort on its watchdog rather than pass slowly.

### Non-vacuity, which is the part usually skipped

The invariant assertions are always-on panics, so "they ran" is only as strong as how many
times the guarded path was crossed. The test therefore asserts the recorder logged more than
10,000 verbs — a run that stopped exercising the verb path would otherwise pass everything
else *vacuously*. And after **every operation of every workload thread** it asserts the
thread's held-lock depth is zero, so a leaked guard is caught at the operation that leaked it
rather than at the end.

There is a matching lesson about the harness itself: **a held latch must release on unwind,
or a failed assertion reads as a hang.** The first composed run panicked inside the window; the
panic was real and correct, but the threads parked in the mock were never released, so the
scope's join-all waited forever and the failure *presented* as a wedge with its message
swallowed. The latches are now a drop guard. This is the same species as the staleness lesson
in section 3.4 — the first symptom of getting a concurrency thing wrong is a hang, not an
assertion — and it applies to the harness as much as to the design.

### The conservation sweep

At the end, on the unwrapped device after a final refresh and a reap, seven properties:

1. **Per-GPU routing never collapses.** For each lane, the two GPUs' owners differ, both
   routing maps agree, and each is asserted *equal* to a three-valued expectation (live /
   condemned / absent) rather than probed for membership. Two lanes each have one condemned
   and one healthy member **on opposite GPUs carrying identical values**, so a condemnation
   keyed on anything numeric would take the wrong process down and be caught here.
2. Torn-down channels are gone from routing; rewritten ones resolve to their new channel.
3. **No resurrect.** For both out-of-band-retired processes: the original identity is gone,
   the component is condemned, and **no process under any identity holds the condemned
   client**. Exactly two condemned entries, exactly four live processes, none of them
   condemned — so a *false* condemnation, which would be a self-inflicted denial of service,
   fails here too.
4. **Completion conservation.** Nothing landed in the system process's queue; processes that
   armed nothing have nothing outstanding; processes that did are acknowledged for exactly
   the events they owned and must then have nothing left.
5. **Arenas pairwise disjoint globally** — across all live processes and all their targets,
   plus the system process, not merely per-GPU.
6. **Provenance.** No host handle minted for one process is observed in another's state; no
   process's GPU-0 sandbox leaks into its GPU-1 state; every published guest-physical address
   lies inside that process's own arena for that very target.
7. **Routing agrees with the graph** — every route names a live process, an existing channel,
   and that channel's key.

Then the whole run happens **twice**, once per lock mode, and the two reports are asserted
*equal* — the mode must not be observable through the API. The report type deliberately
excludes genuinely nondeterministic quantities (which thread won a materialization race,
exact verb counts, minted handle values) rather than fudging them into a false determinism
claim.

**And it found a real defect on its first composed run** — the resurrect bug of section 2.8.
The methodological point is exact: the isolated test for that behaviour was green *only
because it never issued a protocol event after the worker died*. The composed run caught it
because it puts a worker death and an allocation-heavy workload **in the same run**. Isolated
cases test what you thought of; composed runs test what you did not.

## 5.4 Property and fuzz testing

Sixteen property tests across six files, with case budgets from 128 to 3,000.

The graph-level properties feed arbitrary hostile event streams — dangling parents,
double frees, freeing before children, duplicating a freed object, duplicate cycles,
use-after-free, handle reuse, unknown classes, out-of-order and duplicated events, events for
non-existent clients — and assert that every step returns a result rather than panicking, and
that the boundary invariants hold **after each event**, so intermediate hostile states are
checked too. One property runs the whole device spine and additionally requires that live
processes' arenas stay pairwise disjoint throughout. One oracles reference counting against
an independently written reference model.

The isolation invariants (section 5.8) are properties with differential oracles: an injective
page-directory-to-physical map for address isolation, and an independent fence model for
completion integrity.

A separate coverage-guided fuzzing workspace targets the pushbuffer parser — *the one place
raw adversarial guest bytes reach the core*. Its input is structured, not random bytes, so
the fuzzer reaches deep decode paths instead of bouncing off early rejects, and it steers
addresses at four windows including one near the top of the address space to probe range caps
and wrapping. It lives in its own workspace because its harness needs unsafe code, which must
never live in a core crate. **CI only checks that it compiles**; running it is a manual gate,
so its 114-input corpus is never re-run automatically — a documented, honest limitation, and
the reason the gated property test's coverage of that decoder mattered so much in section
5.11.

## 5.5 The staleness canaries, and the exact-variant rule

The re-validation invariant is checked by canaries that mutate the world **inside the
lock-free gap**: hold a verb pending, spawn the operation, wait for the edge, retire the
process or tear down the channel or rewrite routing, release, and read the outcome.

They exist in two places deliberately — focused, one per case, *and* composed into the mean
run — because the design is explicit that the standing scripts must compose rather than live
as isolated cases.

**The rule is: assert the exact fault variant, never `is_err()`.** It is stated in the code
in four places, and it was learned the hard way: a canary once passed for the wrong reason,
reporting "the host failed" when the truth was "your process was torn down". A loose assertion
would have shipped that hole.

In practice the rule holds essentially everywhere. Across all nineteen test files there are
forty-two `is_err`/`is_ok` occurrences and **exactly one** bare `assert!(… .is_err())`; it is
benign (the same condition is asserted as an exact pattern eight lines above) but it is the
one place a reviewer would want tightening.

The canaries also assert **mutation-freedom**, not merely refusal, and one goes further and
pins the *disposition*: a refused commit's orphaned host objects must be released on the same
worker, still lock-free, in reverse allocation order — child before parent — with an explicit
non-vacuity check that the held verb really did allocate something to orphan. The reason is
stated in the source and is a good general principle: *"we refused" and "we refused and
leaked" are otherwise indistinguishable from the outside.*

## 5.6 The determinism differential

Described in section 5.2. Three axes: permutation with benign duplication, two processes'
fact streams woven every which way, and the data-plane dimensions (published backing, armed
fences) projected order-invariantly. This is the suite that makes the T1 tier sound, so it is
the one whose failure would invalidate the most other tests.

## 5.7 The C-bug regression matrix

Twenty-five catalogued bugs from the C artifact, each classified against this core with a
citation of the *structural property* or the *named test* — never an opinion. "Impossible"
means the buggy code shape is unrepresentable: the fallback, the scalar or the gate it lived
in does not exist here.

The tally: fourteen impossible by construction, five already tested, three converted from gap
to tested in that pass, five honestly deferred to the milestone that will model the missing
subsystem. The deferred ones are named — the page-table walker's leaf-size discipline, method
encoding semantics that belong to a real architecture adapter, the ABI layer's whole bug
family, and the GSP reboot state machine.

**The matrix earned its keep immediately.** Writing the regression for the predecessor's
guest-physical window exhaustion exposed *the same leak in this core*: the allocator never
recycled and retired processes were never reaped, so the device died after a handful of
teardown generations. Verified both ways — with recycling disabled, both lifecycle tests fail
with window exhaustion. The fix is the by-value arena release of section 2.6.

That is the argument for the whole exercise: a regression matrix built from a *working*
predecessor's real failures is the closest thing to hardware feedback available before
hardware.

## 5.8 The security invariants

An independent red-team pass states four isolation invariants as checkable properties over a
three-tier attacker model, and audits every place a guest handle becomes a typed object.

- **I1 — cross-process address isolation.** If one process's address resolves, it yields that
  process's own backing, never another's. Mechanised over a multi-process world with identical
  addresses and identical handles, woven with generated hostile junk, checked against an
  injective oracle **both ways** — each resolves to its own, and provably not to any other's.
- **I2 — completion integrity.** A completion fires only from a genuinely armed source in the
  owning process, at most once, at or after its target. Forged or backwards values cannot
  forge a completion or cross-signal another armed source. Differential against an
  independently written reference fence model.
- **I3 — refcount soundness.** Over deep trees with cross-client duplicates and interior
  frees: no free-while-referenced, no double free, no leak. Freeing every root drains the
  graph to empty.
- **I4 — denial-of-service containment.** The device is *always* projectable; a bystander's
  routing and resolution are never corrupted by a storm; every hostile event earns only its
  own loud refusal.

Alongside these, a boundary suite of named adversarial examples covers six things a hostile
guest must not be able to do, including six audited unbounded-allocation paths each driven to
its cap and asserted to fault with the **exact** capacity variant *exactly at* the cap — never
by running out of memory.

**The pass found two real bugs**, both reachable by an ordinary hostile guest process, and
both of the same shape: a *parked* fact that resolves during an unrelated operation produces a
device-level fault whose rollback restores the parked fact — so it re-fires on every
subsequent allocation. Both were **device-wide control-plane wedges**: every other process's
allocations refused, forever. Both fixed at their resolution sites, each with a regression
verified to fail before the fix. It also hardened a confused-deputy inconsistency where two
resolvers disagreed about what a memory handle was.

One assumption is documented rather than fixed: which of two colliding claimants to a
hardware identity is refused is order-dependent. Under the ordinary attacker model this is
unreachable (userspace cannot choose its own page directory base or channel id); it becomes
reachable only to a compromised guest kernel, which already owns its whole VM. The
cross-tenant boundary is untouched and the effect is a contained refusal, not a leak.

The scope honesty is worth repeating: memory safety, out-of-bounds and breakout are
explicitly **out of scope** for this pass because the pure core has no pointer to get out of
bounds and no unsafe code to be unsound. *"Asserting them here would be theatre."*

## 5.9 The race detector

ThreadSanitizer over the four threaded suites. First run, 2026-07-25: **28 tests, 0 races,
exit 0.**

Two details that make it real rather than ceremonial. It rebuilds the standard library
instrumented — without that, the standard library's own synchronization is invisible and every
lock handoff reads as a race. And it runs with the slow-test flag *on*, because the gated
16-thread soak is the whole reason the job exists and a run that skipped it would be the race
ceiling in name only.

Measured inflation is about 20× (the stress suite 290 s, the shell suite 69 s, the mean test
20 s), which is why the watchdog timeout is overridable: the suites' wedge detectors must
measure wedging, not instrumentation tax. One suite was missing that override and would have
aborted on sanitizer overhead rather than a real hang — found and fixed while wiring the job.

The design had called this "the race ceiling" since it was written. Until that date it had
never been executed against the built code. It is now a nightly job, because — per the
standing lesson — *a gate a human has to remember is not a gate.*

## 5.10 The mutation gate, and the two ways it silently lied

### What a mutation score means

Breadth is not meaningfulness. A suite can have hundreds of tests and still not *notice* when
the code changes behaviour. Mutation testing measures that directly: mechanically introduce
small changes to the logic — flip a comparison, replace a return with a constant, change an
operator — and run the whole suite against each one. A mutant the suite still passes is, by
construction, **a change to the logic that no test detects: an objective, reproducible test
gap.** The score is killed divided by viable, where "viable" excludes mutants that do not
compile (a tool artifact) and a mutant detected by a hang counts as killed.

It answers "do we have enough tests?" with a number and a triage of every survivor instead of
an opinion. On the pure core it found **24 real gaps** that a 113-test suite — already
containing order-independence properties, fuzzing, security invariants and a determinism
differential — had missed. The richest cluster was the parked-fact and free-subtree
bookkeeping: eight distinct predicates that no test pinned to their *exact* boolean. For
example, a dead-address-space mapping teardown must fire on `touched AND dead`, never
`touched OR dead`, which would tear down a *live* address space's mappings whenever a
duplicate kept it alive. The fuzz suite's structural invariants did not localize any of them.

Current standing: **L0 — 99.2%** (245 of 247 viable killed; both residuals proven equivalent
or documented). **L1 — 92.44%** (159 of 172), with the residual triaged in section 3.9.

### ★ The first way it lied: it tested nothing

Two of the mutated crates have no unit tests of their own; every test covering them lives in
a separate workspace test crate. Without the flag that tells the tool to run the *workspace*
suite, it ran only the mutated package's own tests — an empty suite — and dutifully reported
**everything** as surviving.

The fix is the flag. The *durable* fix is that the scoring step now fails outright when the
viable count is zero, so the failure mode is loud rather than a plausible-looking bad score.

### ★★ The second way: compiler crashes counted as "unviable" and vanished from the denominator

This is the one worth the whole section.

The first L1 campaign reported **24 caught of 36 viable = 67%**, with 256 mutants filed
"unviable". Both halves were wrong.

The tool copies the source tree once and rewrites one file per mutant, reusing a single build
directory — and therefore a single **incremental compilation cache**. That cache does not
survive the churn: the compiler aborts with an internal assertion failure. The tool cannot
tell a compiler crash from a type error, so it files the mutant **unviable** — which removes
it from the denominator **silently**.

Measured: **136 of 293 mutant builds crashed the compiler.** The reported 67% was scoring
about a fifth of the surface, arbitrarily selected by which compilations happened to crash.
As the commit puts it: *it was not a low score; it was not a score.*

The tell had been visible and was missed: one method's `-> false` mutant was reported caught
while its sibling `-> true` was reported unviable — and `-> true` obviously type-checks.

Disabling incremental compilation removed **every** crash (0 of 292) and moved 88 mutants
into the real denominator. Honest numbers on the same tree: **88.95% before, 92.44% after**
the gap-closing tests.

Three hardenings followed, and they are the right lessons. The job now **fails outright if any
mutant's build log contains a compiler crash**, before the threshold is even read — because if
"unviable" ever again means "the compiler crashed", the denominator is fiction and so is the
score. Viability is a property of the *mutant*, never of the run, so when two runs disagree the
crash-free one is authoritative. And a documented sanity check exists: grep the logs for
compiler panics and require zero before trusting any campaign's number.

**The transferable lesson: a metric that can be wrong *upward* is worse than no metric.** Both
defects made the surface look better-tested or worse-tested than it was, silently, and neither
was visible in the headline number.

### The gap-closing tests, and what they say about the suite

Each was verified by hand-applying the exact mutation and watching the named test flip from
pass to fail. Four are worth reporting because of what they reveal:

- **The three re-validation guards.** Killing them requires committing one process's plan
  against a *live* second process — a term no whole-device canary can reach, because the shell
  always re-locks the plan's own process. Every assertion names the exact staleness variant.
- **The ring gate through the shell.** The gate function was pinned; the shell's wrapper around
  it was replaceable with "everything is published". That is the isolation gate of section 2.4,
  and it mattered.
- **A completion identity fold.** A lossy operator in composing a completion's identity is a
  completion **collision** — a lost completion, arriving through the untrusted pushbuffer path.
  Every prior test asserted only *that* a completion was observed, never *which*.
- **A read budget term**, killed by a 12-range submission ring whose budget edge lands *inside*
  a range — in 0.35 seconds. This **supersedes the pure core's own verdict** on the same term,
  which had rated it acceptable on the grounds that killing it would need an unreasonably large
  hostile ring. It did not, and it was not acceptable.

That last one is the healthiest thing in the mutation documents: a previous, carefully argued
"this gap is acceptable" was revisited and found wrong.

## 5.11 The slow-test flag, and how gating a test hollowed out a security gate

The flag is one environment variable, `KAYFABE_SLOW`, gating exactly two tests. Environment-only
because Rust's test harness accepts no custom command-line flags — an unknown flag is a hard
error — so an environment variable is the only channel reaching every test binary uniformly.

Three things about it are well done. **Membership was measured, not guessed**: only two tests
exceeded about three seconds, and together they were roughly 85% of wall clock. The fast path
went from 122.7 s to 17.1 s. Second, one of the tests hidden behind the old "long-running"
marker turned out to run in **0.6 seconds** — it had never been slow, it was invisible for no
reason, and it now always runs. Third, a gated test **prints how to run itself**, to standard
error, bypassing the harness's output capture — because otherwise it is swallowed on the
passing path and silently vanishes, which was the entire failure mode of the marker it
replaced. There is now **zero** use of that marker anywhere in the repository.

### ★ And then it broke something

The mutation job was set up to run with the flag *off*, "because the fast path is what makes
mutants times suite tractable". Entirely reasonable in isolation.

Except: the 73-second gated test is the **only** coverage of the untrusted, guest-controlled
pushbuffer decoder. Skipping it let decoder mutants survive that the property test kills —
verified on that scope: **2 missed became 1 with the flag on**. So the flag introduced to keep
the *push* gate fast had, as a side effect, **hollowed out the mutation gate exactly where the
input is hostile.**

The job now sets the flag. The cost is honest: about 90 seconds per mutant, roughly doubling a
weekly job. That trade is not close.

The generalizable lesson is stated in the commit itself and is the sharpest thing in this
whole section: **two individually-reasonable decisions composed into a security regression that
neither one contains.** Nobody weakened a security gate. Someone gated a slow test, and someone
else made a job fast, and the composition did it. There is no code review that catches that —
only a measurement.

## 5.12 Bite-checking, as a standing practice

Worth naming separately because it appears in nearly every commit: a new gate is not trusted
until it has been made to fail on purpose.

The boundary grep was verified in both polarities — clean tree passes, planted comment fails.
Neutering the rank check makes both inversion tests fail. Injecting one spurious lock into a
device-wide operation fails the acquisition-count test — and *notably does not* trip the rank
assertion, because rank 0 to rank 1 is a legal order, which is what makes that test the only
thing between the design and a silently reintroduced convoy. Shrinking the pool to one worker
makes the mean test abort. Removing the condemnation line reproduces the original bug's output
**byte-identically** and fails exactly six tests — and exactly the two that should still pass
without the fix do.

That last form is the strongest: not "the fix works" but "the fix is load-bearing, and here is
the precise blast radius of removing it."

## 5.13 What the tests do not cover

Collected honestly.

- **Anything about real NVIDIA hardware or the real driver.** Section 3.12.
- **Host-object reclamation, broadly.** Section 3.2: nine gaps, found by a targeted audit
  rather than by the suite, in a codebase with 203 tests and two mutation campaigns. The
  suite drives construction tens of thousands of times per run and teardown a handful. That
  asymmetry is the most useful thing this document can point a future test campaign at.
- **Device reset of any kind.** No test exercises a driver reload, a guest panic, or a VM
  reset, because no mechanism exists to exercise (G5).
- **Verb interruption and cancellation.** No mechanism exists to test (section 3.7).
- **The converging-staleness retry bound.** Needs new mock machinery (section 3.4).
- **Performance, of anything.** Section 3.10.
- **The distinction between parking and spinning** in pool backpressure — deliberately, since
  the alternative is a timing assertion (section 3.9).
- **The reactor's OS half, the sandbox wire protocol, memory-mapping, registers, the GSP boot
  machine, wire decode, the page-table walker** — all unbuilt.
- **Corpus rot in the fuzzing workspace** — CI compiles it but never runs it.
- **Panic behaviour under production conditions.** The always-on assertions crash the process;
  nothing exercises what that does to attached guests, because there are none.

---

# 6. Directory structure and where to find things

## 6.1 Crates

| Crate | Lines | Job | State |
|---|---:|---|---|
| `kayfabe-core` | 4,003 | ★ the spine: `rmgraph` (source of truth) · `project` (boundaries + routing) · `gpu` (runtime, `Spine`/`Proc`/`Vas`/`Channel`, transactional apply, condemnation) · `gpa` (arenas) · `reactor` (the pure completion-source port) | full |
| `kayfabe-fwd` | 1,945 | intent to host operations: the one doorbell ring path, publish, the one pushbuffer parser, Case-1/Case-2 split, fence arm, present. All mixed entry points route/act split; all verb-issuing ones plan/execute/commit | full for the core slice |
| `kayfabe-rt` | 1,783 | ★ the L1 shell: `lock` (ranks + assertions) · `device` (`SharedDevice`, both lock modes, the verb driver, the pool gate) · `inbox` · `executor` | L1-M1 |
| `kayfabe-mocks` | 1,465 | one deterministic fake per port, plus the verb recorder and the pending-verb latch | full, test-only |
| `kayfabe-completion` | 756 | per-process queues, delivery policy with the drain gate, fence arms | full |
| `kayfabe-isolate` | 631 | the sandbox ports: `RmBackend` verbs, `Worker`, `Isolate`, `IsolateFactory` | traits only |
| `kayfabe-arch` | 508 | identity newtypes + the per-generation port set | traits only |
| `kayfabe-util` | 493 | interval map, virtual clock, the send/sync build assertion, and the thread-local lock witness | full |
| `kayfabe-vmm` | 334 | the hypervisor and display ports (eight capability groups) | traits only |
| `kayfabe-mmu` | 241 | the per-address-space table (miss = fault) + a walker skeleton | table full; **walker is a 41-line skeleton** |
| `kayfabe-abi` | 53 | per-driver-version wire layouts; the only future home of C-layout structs | **stub** |
| `kayfabe-gsp` | 34 | the GSP boot state machine | **stub — a placeholder enum** |
| `kayfabe-trace` | 21 | a one-method trace sink | **stub — no call sites** |

## 6.2 Tests — `tests/tests/*.rs`

Nineteen files, 14,383 lines, 156 integration tests. The full count of 203 is 200 test
functions (156 integration plus 44 unit tests inside crates) plus 3 documentation tests.

| File | Lines | Tests | Pins |
|---|---:|---:|---|
| `l1_mean.rs` | 2,011 | 8 | ★ the mean test + the six condemnation property tests |
| `l1_verb_seam.rs` | 1,379 | 19 | plan/execute/commit; the assertion at the verb itself; intra-process progress; the three staleness canaries; pool backpressure; worker death |
| `security_boundary.rs` | 1,276 | 19 | named adversarial examples for six boundaries; six capacity paths driven to their caps |
| `security_invariants.rs` | 1,172 | 10 | I1–I4 as properties with differential oracles; the confused-deputy audit |
| `object_model.rs` | 1,103 | 13 | memory objects, mapping refcounts, parked maps, namespace-confined cascade |
| `rt_shell.rs` | 877 | 5 | the lock-mode differential; zero process locks in device-wide operations; bounded threaded smoke |
| `c_bug_regressions.rs` | 872 | 14 | the C-bug matrix's gap rows, plus four mutation-gate kills |
| `concurrency_stress.rs` | 753 | 4 | 16-thread soak; per-process parallelism; lock-free shared reads |
| `fuzz_rmgraph_invariants.rs` | 744 | 5 | all five are properties: hostile streams, spine safety, order-independence, refcounting |
| `weird_order_regressions.rs` | 643 | 6 | one spec-legal weird order per C incident |
| `engine_context.rs` | 608 | 12 | Case-1/Case-2; the fence arm; "the verb surface does not grow per engine" |
| `multi_gpu.rs` | 584 | 8 | per-GPU keying, cross-GPU isolation, identical ids across GPUs |
| `determinism.rs` | 582 | 4 | ★ the whole-device order-invariance differential |
| `pushbuffer_parser.rs` | 546 | 9 | the one parser; the ring gate both ways; the gated 73 s hostile-byte property |
| `soak_llm_like.rs` | 295 | 3 | inference-shaped soak, invariants every iteration |
| `sim_14_two_process.rs` | 271 | 5 | the two-process collision shape end to end |
| `reactor.rs` | 229 | 2 | the registry wired into the real retire path |
| `rmgraph_order_independence.rs` | 225 | 5 | deterministic full permutation |
| `present_seam.rs` | 213 | 5 | export → present → vblank, both halves |

`tests/src/lib.rs` (330 lines) holds the scenario builder — which *scripts the guest's RM
protocol as abstract events* and executes nothing — plus the slow-test flag. Its most
important shape is the identical-handle constructor: NVIDIA-shaped handle values shared by
every process, with only channel ids varying, which is what puts the collision case under
test rather than a sanitized version of it.

## 6.3 Design documents — `docs/design/`

**Read first, in this order:**

- `core_state_and_consolidation.md` — the reviewed per-crate state and the L1 hand-off
  contract. The best single entry point to the core.
- **`l1_concurrency.md` — the design of record for L1**, and the most valuable document in
  the repository. Sections 0–10 are the design; **section 12 is the contact log**, fifteen
  entries recording where building it proved the design wrong. Section 11 is the bets.
- `execution_plane.md` — how real GPU work is orchestrated, and the anti-emulation boundary.

**Gates and evidence:** `core_mutation_gate.md` (both campaigns, the triage of every
survivor, and the two measurement defects), `core_security_threat_model.md` (the attacker
model, I1–I4, and the two wedge bugs found), `c_bug_regression_matrix.md` (25 rows),
`core_completeness_gate.md`.

**Scope and portability:** `multi_gpu_and_mig.md` (the routable-target axis and the honest MIG
reality check), `portability_arm64.md` (the deferred-but-guarded seam),
`gr_multigpu_seam_audit.md`.

**This document:** `l1_architecture_summary.md` (source) and `l1_architecture_diagram.py`
(the three figures). The rendered PDF and PNGs are gitignored and regenerated.

Additional settled design lives in the C repository's `docs/design/` — notably the forwarding
model (translate intent, never replay privileged internals) and the address table (the one
table of truth). This repository implements those; it does not re-derive them.

## 6.4 Continuous integration — `.github/workflows/ci.yml`

Two tiers, six jobs. The split exists because the every-push gate must stay fast and the heavy
checks must still be *standing* rather than remembered — which is how roughly 9,100 lines of
L1 code once reached master with neither a race-detector run nor a mutation campaign.

**Every push and pull request:**

| Job | Runs | Gates |
|---|---|---|
| `stable` | build; the fast suite; lint with warnings denied; format check on both workspaces; **the hexagonal boundary grep** | the bar for landing anything |
| `aarch64` | cross-`cargo check` of the whole workspace for aarch64 | no CPU-architecture assumption may enter the core — it fails on the push that introduces one, and needs no cross-toolchain because checking does not link |
| `nightly-fuzz` | builds the fuzzing harness | the harness keeps compiling as the core evolves; running it stays manual |

The **boundary grep** deserves its own note. It searches eleven pure crates for six tokens
(`eventfd`, `epoll`, `timerfd`, `rawfd`, `libc`, `O_NONBLOCK`), case-insensitively, **in code
and in comments**. Its exit-code polarity is documented in the file because grep returns
success on a *hit*, which is the failure case here. Its message ends: *"Reword the comment;
never weaken this gate."*

**Scheduled, standing:**

| Job | When | Runs | Gates |
|---|---|---|---|
| `slow` | nightly | the whole suite with the slow flag set | the two measured-slow tests actually run, without a human remembering them |
| `tsan` | nightly | the race detector over the four threaded suites, with an instrumented standard library, the slow flag on, and an inflated watchdog | the race ceiling (§5.9) |
| `mutants` | weekly | the mutation campaign over the L1 surface, with incremental compilation off, a compiler-crash guard, a zero-viable guard, and a hard 91% threshold | the mutation gate (§5.10) |

Three details in that last row are the section-5.10 lessons encoded as mechanism: the slow
flag is on because turning it off hollowed out a security gate; incremental compilation is off
because its crashes silently inflated the score; and the crash guard runs **before** the
threshold is read, because a fictional denominator makes any threshold meaningless. The
threshold's own comment: *"Raise it when a campaign measures higher and the new number holds
across a run; never lower it to make a red night go away — a survivor is a test gap, and this
file is where that gets argued."*

Note that the CI file **deliberately refuses to name a test count** ("it rots on every PR") —
a lesson the README and architecture map have not yet learned (section 7.11).

---

# 7. Discrepancies found between the code and the documents

Reviewing the source against the design documents turned up eleven disagreements. They are
ordered by how much they matter. Two are substantive.

These are the same species as the six the teardown audit found independently (section 3.2),
and the pattern is the finding: **documentation in this project drifts optimistic relative to
its code.** Seventeen instances across two independent passes is not a run of bad luck. Each
one was true when written and became false as the code moved; what is missing is a mechanism
that notices. The continuous-integration file already models the right instinct — it
deliberately refuses to name a test count because *"it rots on every PR"* — and the two
documents that do name one are both wrong (§7.11).

## ★ 7.1 The atomicity claim is stronger than what the code delivers

The device's transactional apply is documented — in the source and in the architecture map —
as all-or-nothing: *"the offending event is refused atomically and no other process's state is
disturbed."*

The rollback restores **only the graph**. The process set is left as the failing refresh
mutated it. Because the refresh absorbs and retires matched processes *while iterating
boundaries*, and separately retires vanished ones, a fault raised later in the same refresh —
a merge refusal on a later boundary, or an arena failure during target materialization —
leaves those processes permanently retired and deregistered. The re-derivation from the
last-good graph then mints them **fresh identities with freshly spawned sandboxes and freshly
carved arenas**.

There is a mitigation in the code: absorbed processes must be untouched, so no host state is
lost. But identities, registered completion sources and arena identity are, and the claim as
written is broader than that.

`crates/kayfabe-core/src/gpu.rs:586-590` (the claim), `:608-612` (the rollback),
`:800-815` and `:893-902` (the retires), `ARCHITECTURE.md:104`.

## ★ 7.2 A reachable panic on a guest-driven control path

Same path. The rollback re-derivation is followed by an expectation that the last-good graph
re-projects. That holds for projection errors, which are deterministic on the same graph. It
does **not** hold for target materialization: if a process was absorbed or removed earlier in
the failing refresh, the re-derivation must re-mint it, and carving its arena can fail with
window exhaustion — because the absorbed process's arena is on the retired list and is not
recycled until the next reap.

That propagates as an error into an `expect` and **panics the device**. It is narrow — it needs
a near-exhausted guest-physical window — but it is a panic on a guest-driven control path, and
the project's stated posture everywhere else is "a loud fault, never a panic".

`crates/kayfabe-core/src/gpu.rs:610`, `:922`; `crates/kayfabe-core/src/gpa.rs:64-66`.

## ★ 7.3 The two-phase sandbox spawn has no mechanism

The design identifies spawning a sandbox under the device write lock as *"a real R1 tension,
resolved by making spawn two-phase: the factory under the lock only reserves; the actual
fork/exec and namespace setup run lazily at the first checkout."* It concludes: **"L1 chooses
reserve."**

There is no reserve-and-realize split anywhere. The factory's spawn is called once, inside
target materialization, which runs under the device **write** lock. Checkout has no lazy
bring-up hook. The port's documentation hedges ("spawn, or lazily reserve") but nothing
enforces the reserve-only half.

So **a real forking factory would be a live R1 violation with no assertion firing** — the exact
shape the contact log's honest entry §12.6 was written about, in a different place. The lock
witness cannot see it, because process creation is not routed through the blocking-section
wrapper or the verb entry point.

This is the most actionable finding in this section: it is a hole that opens the moment the
real sandbox lands, and the assertion designed to catch that class does not cover it.

`crates/kayfabe-core/src/gpu.rs:570`; `crates/kayfabe-fwd/src/lib.rs:357-367`;
`docs/design/l1_concurrency.md:758-765`.

## 7.4 Two of four completion pump edges are unwired, past their stated deadline

The design names four edges on which delivery is pumped. The contact log records the
observe-then-pump edge as a stage-2 gap and promises *"the pump edge lands with the hypervisor
seam in stage 3."* Stage 3 landed; so did stage 4. It is still absent, and its own source
comment still says so. The drain edge has no implementation either. Of four named edges, the
shell wires one and a half — the deferred backstop, and the poll edge via the core.

`crates/kayfabe-rt/src/device.rs:843-863`, `:402-407`; `docs/design/l1_concurrency.md:532-542`,
`:1135-1142`.

## 7.5 The two "pumping without a deliverer wedges the plane" arguments contradict each other

The reason given for *not* pumping on the observe edge is that opening a batch nobody drains
would wedge the delivery plane. The deferred backstop edge does exactly that — pumps, closing
the gate, and hands the batch to a caller that may or may not deliver it. Either the observe
edge could have used the same surface-it-to-the-caller contract, or the backstop shares the
hazard. One of the two comments is wrong and the design does not distinguish them.

`crates/kayfabe-rt/src/device.rs:845-848` versus `crates/kayfabe-rt/src/executor.rs:88-92`.

## 7.6 The multi-GPU backstop still pumps a hardcoded GPU zero

Recorded in the contact log as a real ABI gap, with the fix scheduled for stage 3. At HEAD the
deferred redelivery event still carries no target, the executor still pumps GPU zero, and the
comment still describes it as "stage 2" and defers the policy to "stage 3" — two stages later.
On a two-GPU device the backstop cannot reach the second target.

`crates/kayfabe-rt/src/executor.rs:84-92`; `crates/kayfabe-vmm/src/lib.rs:172`;
`docs/design/l1_concurrency.md:1111-1118`.

## 7.7 Module documentation that describes a world the same file refutes

The shell's device module header still carries a section titled "R1 status in stage 2
(honest)" explaining that act phases *"still run the backend verbs inline under the process
lock"* and that the real split *"is stage 3's job"*. That is false of the file it heads: the
phases below it are plan emitters, the verb runs with no lock held, and the contact log
declares the gap closed. A reviewer auditing the lock invariant from that header would
conclude it is still unenforced.

Related, smaller: the reactor module claims there are no descriptor or syscall words in the
crate *"not in the code, not in the comments"* — in a sentence containing both words. The
actual enforced rule is narrower (six specific tokens), so nothing fails; but the module's own
absolute wording is self-contradicted.

`crates/kayfabe-rt/src/device.rs:53-60`; `crates/kayfabe-core/src/reactor.rs:38`.

## 7.8 Stale concurrency-bound documentation naming a method that no longer exists

Three places still describe the host-connection trait as the workspace's one send-only
exception, *"reachable only through `Isolate::rm(&mut self)"* — a method that no longer exists
anywhere (verified: zero matches). One of them sits thirteen lines above the compile-time
assertion that refutes it. A fourth place gives it as the canonical example in a macro's
documentation. And the worker type's own documentation says it is not shareable, while the
same file compile-time asserts that it is.

`crates/kayfabe-isolate/src/lib.rs:617-618`, `:355-356`, `:631`;
`crates/kayfabe-core/src/lib.rs:56-60`; `crates/kayfabe-util/src/lib.rs:53`.

## 7.9 The core still advertises lock-free shared reads without qualification

The contact log records that sharded mode costs this property and says it is flagged in the
relevant method's documentation — and it is. But the claim is still made **unqualified at its
source**. Under sharding a per-process read takes that process's lock; under the degenerate
mode it takes the device write lock. The advertised property is false in both shipping
configurations.

`crates/kayfabe-core/src/lib.rs:54-55`; `crates/kayfabe-rt/src/device.rs:782-801`.

## 7.10 "Each rank exactly twice" is a success-path-only number

Stated in the contact log as an invariant and pinned by a test. It holds for the uncontended
success path. A verb error re-enters twice more; a refused commit re-enters again, up to eight
retries; and the control path takes rank 0 once more in its classification prologue before the
driver ever runs. No correctness violation — everything is in rank order, one per rank — but
the number is not invariant.

`crates/kayfabe-rt/src/device.rs:565-641`, `:729-770`;
`docs/design/l1_concurrency.md:1234-1238`.

## 7.11 The README and the architecture map do not know about a whole crate, and their counts are stale

The README states that *"no real adapter (Linux, QEMU, or NVIDIA arch) exists yet"* and that
the project is *"paused before descending to L1"*. The L1 shell is 1,783 lines, is a workspace
member, describes itself as the L1 threaded shell, and appears in neither crate table. Four L1
test suites are likewise unlisted.

Both files say **192 tests**; the number is **203**. Both say the test directory holds
**14 files**; there are **19**. The architecture map adds "~120 integration tests"; there are
156. The mutation figure quoted (99.2%) is the pure core's; the L1 surface is 92.44%. The
consolidation document still says "143 tests green, plus one long soak marked ignored" — and
nothing is marked ignored anywhere in the repository.

Two smaller documentation errors of the same species: the graph's resource documentation states
liveness as "references non-empty" while the code correctly requires references *or* mapping
references (the architecture map has it right); and the read-only query form of the ring gate is
described as "the same predicate" as the enforcing form, but it is strictly stricter — it
refuses a case the enforcing path accepts, so a caller pre-checking with it gets a refusal for a
submission the ring path would allow.

`README.md:33-34`, `:72`, `:114`; `ARCHITECTURE.md:58`, `:123`;
`docs/design/core_state_and_consolidation.md:290`; `crates/kayfabe-core/src/rmgraph.rs:223-232`;
`crates/kayfabe-fwd/src/lib.rs:1725`, `:1748-1760`, `:841`.

---

# 8. Provenance

Written read-only at `master` / `3569d46`, 2026-07-25, without compiling or running anything.

**Design sources:** `l1_concurrency.md` (all sections, including the fifteen-entry contact
log), `core_state_and_consolidation.md`, `core_mutation_gate.md`, `core_security_threat_model.md`,
`c_bug_regression_matrix.md`, `execution_plane.md`, `multi_gpu_and_mig.md`,
`portability_arm64.md`, `README.md`, `ARCHITECTURE.md`, `CLAUDE.md`, `.github/workflows/ci.yml`,
and — for sections 1.2 to 1.5 — the C repository's forwarding-model and address-table documents.

**Code read:** all thirteen crates' sources and all nineteen test files, plus the mock crate and
the fuzzing workspace.

**Numbers:** 203 tests = 200 test functions (156 integration across 19 files, 44 unit) plus 3
documentation tests, counted statically and corroborated by `core_mutation_gate.md:239`. Fast
path *about 18 seconds*, per the same line; the 122.7 s to 17.1 s improvement is from commit
`6195b7c` and was measured at 192 tests, before eleven more landed. Mutation: 99.2% (245/247)
on the pure core and 92.44% (159/172) on the L1 surface, both from `core_mutation_gate.md`;
threshold 91% from `ci.yml:274`. Race detector: 28 tests, 4 targets, 0 races, from
`l1_concurrency.md` §12.15 and the commit that made it a standing job. Constants (pool of 4,
retry bound of 8, the capacity caps, the fence jump guard) read from source.

**Attribution of judgement.** Sections 1, 2, 4, 5 and 6 describe the project's own design and
its own recorded findings. Section 3 combines the project's named bets and contact log — which
are quoted or closely paraphrased and attributed — with this review's own reading; §3.14 is
entirely this review's. **Section 3.2's nine gaps come from a separate teardown-completeness
audit** that completed during writing and whose findings are not yet in the contact log; the
commentary around them, and the optimistic-drift argument, are this review's. **Section 7 is
entirely this review's**, and each item cites the file and line it is drawn from.

**Concurrency note.** This document was written while other work was in progress in the same
tree — the G1/G3/G3b/G4 core teardown fixes, and a design for the next L1 milestone's OS
shell. This document owns only itself and its diagram generator; it changed nothing else, and
it is pinned to `3569d46` precisely so that the moving files do not make it quietly wrong.

Nothing in this document amends the design. Discrepancies found in review should be filed
against `l1_concurrency.md` and the affected sources.

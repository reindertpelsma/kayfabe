# THE WRITE-TRAP CONTRACT — what a vCPU may do inside an MMIO exit

**STATUS: LIVE AS A REFERENCE, 2026-09-21 (w822).** Cited by `THE_DESIGN.md`, which is the
current architecture. ⊘ Where this file's *architecture* disagrees with that one, **that one
wins** — this predates it. ★ What stands here regardless: its **measurements**, its **ogkm
findings**, and the **reasoning** behind a constraint.


> Status: **LIVE, 2026-09-13.** Owner ruling, recorded verbatim below. Supersedes the reading of
> `the_three_synchronization_points` (2026-09-09) that treated the TLB invalidate, RPC map calls
> and UVM setup as *"blockable"* — ★ **a ruling's DATE is part of its citation**, and this one is
> later and narrower.

## ★★★★★ THE RULE

**Owner, 2026-09-13:**

> *"I don't think on bare metal the CPU assumes that something synchronous has happened during
> the MMIO trap. Only `BAR0 window latch` seems to me one that can do the most expensive one,
> which is also cheap and almost instant, a mmap remap of fake fb on pramin. The rest all seems
> to me something that uses simple queues, with only a simple lock, that's held for microseconds
> guaranteed, hard to mess up, and maybe an atomic table (atomic table only possible if all
> entries are u64 atomic integers, if not possible, use a lock over it)."*
>
> *"Since read is completely untrapped and bar1/bar2 are also untrapped, means that we cannot
> block the vcpu thread for any other CPU work on the guest basically which is what makes this a
> useful product. The reason is MMIO traps cannot be interrupted by the guest."*

## ⊘ WHY THIS IS A PRODUCT ARGUMENT, NOT A LATENCY ONE

**A guest spinning on a backed page is PREEMPTIBLE. A vCPU inside an MMIO exit is NOT.**

The guest's own scheduler can interrupt a spin loop and run something else on that vCPU. It
cannot interrupt a VM exit: until we return, that vCPU is unavailable to *every* process in the
guest, not merely to the thread that touched the register.

⇒ Blocking in a trap does not slow down one CUDA thread. **It freezes a core of the customer's
VM.** That is the difference between a slow device and an unusable one, and it is why this rule is
about the product rather than about microseconds.

★ And the corollary that makes almost every trap convertible: **the same wall time, spent
spinning, costs nothing.** A guest that polls a shadow we update later waits exactly as long as a
guest we made wait inside the trap — and stays schedulable throughout.

## ⊘ BARE METAL ASSUMES NOTHING SYNCHRONOUS

A PCIe memory write is **posted**: it completes at the root complex, not at the device. The CPU
retires the store without waiting for the device to act on it. The only thing a driver can
actually observe is a subsequent **read** — and in this port every read is either backed memory or
a shadow, both untrapped.

⇒ There is no register whose *write* the guest can observe completing. Anything we do
synchronously in a write trap is work we chose to do, not work the hardware contract requires.

## The permitted shapes, in order of preference

1. **An atomic table.** One naturally-aligned `u64` per entry, `Relaxed` load/store, no lock.
   `kayfabe_device::dbtable::DoorbellTable` is the worked example: the whole vCPU read path is a
   bounds check, one load and a two-bit decode. ⚠ Only legal when every entry fits ONE atomic
   access — 8 bytes. A struct no CPU can load atomically needs a lock, and pretending otherwise
   is a torn read.
2. **A simple queue under a leaf lock held for microseconds.** Push a `u64`, notify, return.
   `PublicationQueue::offer` is the example: take the leaf mutex, insert, notify. No host verb, no
   isolate IPC, no plane lock, no guest-RAM read.
3. **A shadow write-through.** Record the value where an untrapped read will find it, so the guest
   polls memory instead of exiting.
4. ⊘ **Nothing else.** No page-table walk, no publication, no host ioctl, no `mmap` (except §
   below), no lock that another thread holds across blocking work.

## ⊘ THE ONE SANCTIONED EXPENSIVE TRAP

**The BAR0 window latch / PRAMIN re-point.** It is an `mmap(MAP_FIXED)` over one slot —
`[measured w653a]` `move_ns[worst=296558 mean=74698]`, i.e. **297 µs worst**. Owner:
*"297us for a thing that only happens at boot is not bad, that's fine for that mmap."*

⚠ **The load-bearing half of that ruling is "only happens at boot", not the number.** PRAMIN is a
bring-up aperture, so the ~22 moves are spent before the guest is doing work. The ruling **expires
if the moves stop being a boot-time set**; the census prints `moves` beside `move_ns` so the
precondition is checkable rather than remembered.

⊘ It is also genuinely un-deferrable: the guest writes the window register and reads *through* the
aperture immediately after, so a deferred move shows it the OLD framebuffer. That is a correctness
break, not a latency trade — the only trap on this list with that property.

## What this makes of the current write-trap table

| trap | today | under this contract |
|---|---|---|
| BAR0 window latch (PRAMIN re-point) | inline `mmap`, 297 µs | ★ **stays inline** — sanctioned, boot-time, un-deferrable |
| USERMODE doorbell | table load + inline dword store, or queue | ★ **already correct** — the worked example |
| TLB invalidate PDB/upper latch | record + shadow write-through | ★ already correct |
| **TLB invalidate TRIGGER** | refresh + publish + premap, **inline** | ⊘ **convertible**: write through *pending*, queue, worker refreshes then writes through *done*. The guest spin-polls an untrapped read either way — it just stops owning the vCPU while it does |
| CPU interrupt tree | update + publish shadow | ★ already a shadow write-through |
| PTIMER | refused by name, counted | ★ already correct |
| Framebuffer / PRAMIN body | `fb_write` into fake FB | ★ a memory write |
| **GSP queue head** | records, queues the forward — **but `observe`/`apply` decodes and mutates the object graph inline under `state.write()`** | ⊘ **convertible**: record the written value, queue it, decode and apply on the worker. See below |
| unclaimed | counted no-op | ★ already correct — bare metal ignores it too |

## ★★★★★ ARM THE COMPLETION SYNCHRONOUSLY, OR THE NEXT REQUEST READS THE LAST ONE'S ANSWER

**Owner, 2026-09-13**, on deferring the TLB invalidate:

> *"the thing that can race is that the next tlb invalidate sees a completed one immediately.
> This means … when the TLB invalidate is written in the register, is synchronously clear the
> wait bit before returning from the trap, which is just one memory write still
> nanoseconds-level."*

⊘ **This is the correctness half of every deferral the guest POLLS, and it is not optional.**
Without it the sequence is:

1. worker finishes invalidate **N**, writes *done* through to the shadow;
2. guest issues invalidate **N+1** — the trap records it and queues;
3. guest spin-reads the shadow **before the worker has dequeued**, sees **N**'s *done*, and
   proceeds;
4. ⇒ the guest runs with **stale translations** and nothing anywhere reports a fault.

⚠ That is silent and it is a correctness break, not a latency one. It is also exactly the shape
that has bitten this tree before — a completion observed that belongs to an earlier request.

★ The fix costs one store. In the trap, **before returning**, write *pending* through to the
shadow (clear the wait/done bit). After that store the bit can only read *done* if **this**
request's worker pass set it, so step 3 is unrepresentable. No lock, no allocation, a single
naturally-aligned store — the contract's shape (3), and the cheapest thing on the permitted list.

⇒ **The general rule: a deferred completion that the guest polls must be ARMED synchronously at
request time.** And the test for which deferrals need it is exactly *"does the guest read anything
back?"*:

| deferral | guest reads back? | needs the synchronous arm |
|---|---|---|
| USERMODE doorbell | no — fire-and-forget by the GPFIFO contract | ⊘ no |
| TLB invalidate trigger | **yes** — `kgmmuCheckPendingInvalidates` spin-polls it | ★ **YES** |
| GSP queue head | no at the register; the reply lands in guest RAM + MSI-X | ⊘ no |
| CPU interrupt tree | yes — the ISR reads the shadow | ★ yes (already write-through) |

⊘ The mechanism already exists and must be **kept, not invented**: w564 made the trigger a
write-through precisely because *"the guest spin-polls this register"*. The deferred design
changes only *what* is written at trap time — **pending**, rather than the completed value.

## ⊘ THE GSP QUEUE HEAD IS THE BIG ONE, AND THE CODE ALREADY SAYS SO

`RegPlane::service_deferred_commands` prints, in the tree today:

> *"the plane lock is held for this long, and a vCPU's queue-head write **waits on it** — compare
> with `worst_trap … at=bar0+0x110c00`"*

`[measured, every boot]` that trap is **15–19 ms**, the single worst in the system. It is not the
vCPU doing the decode — it is the vCPU **blocked behind the worker** which holds the plane lock
while draining. ⇒ Deferring the *forward* already happened; what remains is that the vCPU still
decodes the command and mutates the graph under a write lock, and so still contends.

**Owner's target shape:** *"The vcpu only registers what value was written to the GSP submit
registers and queues that in a super simple queue, the same way how the doorbell page table
works."*

### ⚠ The three questions that must be answered before building it

1. **Read-back.** Is any value the guest reads back dependent on the decode? If the queue-head
   write is fire-and-forget (posted, reply arrives asynchronously in guest RAM with an MSI-X),
   record-and-queue is safe by the same argument as the doorbell.
2. **Order.** `observe`/`apply` runs in trap order today, so the graph sees commands as the guest
   issued them. A FIFO single-consumer queue preserves that — ⊘ but it must **not coalesce**, or
   two distinct commands become one. (`PublicationQueue` coalesces on equal token; GSP seq tokens
   are distinct by construction, which is why `coalesced=0` there.)
3. **A lagging graph.** Today the graph is current before the trap returns. If apply moves to the
   worker, a later trap can observe a graph that has not caught up — the same class as the
   doorbell's `exec.scheduled` race, and the one that fails **silently**.

⊘ (3) is the real design question and is unanswered. It is not a reason to keep the work inline;
it is the thing the design must state an answer to.

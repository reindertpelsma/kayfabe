# The scratchpad address space — where OUR work lives

> ### STATUS — 2026-08-11
> **LIVE, with one correction folded in below (§0).** Owner design, 2026-08-10; corrected
> 2026-08-11 after I shelved it on a misreading. ⇒ **The scratchpad is NEEDED** — for throughput,
> which is a product requirement — and it is **not next**: it is the kernel road, while the compute
> road is stuck at `RING-VA-UNBOUND`.
> Companions: `mode2_channel_ownership_split.md` (who owns which channel),
> `s1_what_does_it_protect.md` (why the old placement was wrong).

---

## 0. ⊘ CORRECTION, folded in — I read a CORRECTNESS answer as a PERFORMANCE one

`w232` found an owner design of **2026-08-07** (`ce_executor_tree.md`): *"★ It is **NOT** needed for
the scrubber — the CPU branch **reaches** both operands."* ⇒ I read that as *"no customer"* and
shelved the scratchpad. ⊘ **That line answers CORRECTNESS. The owner's question was PERFORMANCE:**

> *"How do you do a kernel-initiated CE with one operand in physical RAM and one in real GPU RAM,
> since most performant is doing real CE? Same for scrub in GPU space if the page is still
> referenced. **There must be a channel with a VA on unprivileged to execute that.**"*

★★★ **"Reaches" is not "reaches fast enough"**, and the standing bar is *a product ready for
production, not a research paper.*

**The C measured this exact cost and named this exact fix:** emulated CE **~95–107 MB/s** vs **~7 GB/s**
real — **≈70×** — every byte through a 4-byte CPU loop (`nvkvm_gpu_emul.c:3755-3844`). Its stated fix:
*"(B) forward the CE to the host GPU."* ★ And the sharper case — **scrubbing GPU memory that is still
referenced** — cannot be discarded; zeros must be written through something that can reach it, and
CPU-side means the aperture window: slow **and** size-limited.

★★★ **And it does NOT violate the boundary — two different things were collapsed.** The rule
(`ce_executor_tree.md`, `[measured]` E10) forbids a **CPU memcpy inside the isolate**, because that
would pull guest bytes into the sandbox. ⊘ **It says nothing about a GPU channel in the isolate** —
driving real GPU channels *is* the isolate's job, and under a real CE the bytes never enter its
address space: **the GPU moves them.**

| shape | verdict |
|---|---|
| scratchpad + **real CE** | ★ compatible with the boundary |
| scratchpad + **CPU memcpy** in the isolate | ⊘ exactly what the boundary forbids |

⚠ **Sequencing: right, needed, NOT next.** The compute road (`cuCtxCreate` → first arithmetic) is
stuck at `RING-VA-UNBOUND`. This is the **kernel** road and a **throughput** requirement. ★ One
falsifiable exception, cheap to measure from a boot we already take: **if the CPU-side scrubber is
slow enough to stall init itself**, it becomes a blocker rather than a performance item.

---

## 1. The rule, in one line

> **Guest userspace → passthrough, in the guest's own address space, opaque to us.
> Guest kernel → emulated, in OUR scratchpad.**

★ This is the channel-ownership split expressed in **addressing** terms rather than policy terms,
which is the more durable form: it is checkable by looking at where an object lives.

---

## 2. The scratchpad

- **One scratchpad address space**, belonging to the **system proc** (the guest kernel's client).
  ⊘ It is **not** per-guest-process, and that costs nothing: its customers are **kernel-initiated
  operations** — scrubs, copies, the driver's own housekeeping — and **there is only one guest
  kernel**. Guest userspace never needs it, because userspace is passthrough.
  ⇒ **Per-process isolation is untouched by this.**
- **Map all of GPGA at a fixed offset**, sparse — holes where nothing is backed.
  ⇒ ★★★ **scratchpad VA = offset + GPGA.** The address is a **pure function** of the thing
  addressed, so there is **no allocator and nothing to track**. This is
  `mode2_address_table_of_truth`'s forward-population rule applied to our own memory.
  ★ And laziness buys little: a scrubber reaches most of GPGA anyway, so lazy mapping would add a
  tracking set for no gain — and mapping everything up front removes the idempotence hazard, since
  there is nothing to re-map. (⚠ Re-mapping an occupied address returns `NV_ERR_NO_MEMORY`, which
  cannot be distinguished from genuine exhaustion — see `s1_what_does_it_protect.md`.)
- **An object may live in several address spaces at once.** The same page can be at the guest's VA
  in the guest's space *and* at `offset + GPGA` in ours. Routine, and unobservable to the guest.
- The scratchpad holds **our** ring, pushbuffer, USERD and completion semaphores — room for several.
  ⊘ **None of it is guest-visible, so its shape is ours to choose.** We are not obliged to give our
  own machinery the layout a guest would use.

**A kernel-initiated operation is then:** map (no-op if already), do the work, return.

---

## 3. Completion — the waiter that does not exist yet

An emulated operation that needs real GPU work puts it in the scratchpad and registers its
semaphore with **one waiter over many semaphores** — an epoll-shaped loop, not one blocking wait per
operation. Spin briefly; if it takes too long, arm an OS notification for the outstanding semaphores
so the loop can sleep and resume. On completion, complete the **emulated** semaphore in the guest's
managed buffers and raise the interrupt if one was requested.

★★★ **If an interrupt is registered after the work already completed, it fires immediately** — as a
real GPU does. ⇒ Check-then-wait must be atomic against the wake, the same discipline as a futex
guarding a sleep. ⚠ **This is not a refinement: a lost wakeup on an event that could never be posted
was 12.6 s of a 13 s `cuCtxCreate` — measured.**

⊘ **This architecture does not exist today.** Audit `026374c`: *"there is NO completion-wait
architecture — synchronous inline on the vCPU thread; the whole reactor subsystem is UNREACHED."*
⇒ The owner's design is the missing piece, named.

---

## 4. ⊘ What this does NOT solve — recorded so it is not read as closure

**Two memories.** The framebuffer objects backed at `w228` are **blank and have no CPU view**: the
host GPU reaches the real object, the guest's own accesses still reach the emulator's fabricated
aperture. Under execution the engine would read **zeros** where the guest wrote, and write where the
guest cannot see.

⚠ **Silent in both directions** — no fault, no `Xid`, no status. And ★★★ **the missing CPU view is
simultaneously the defect and the reason the defect would be invisible**: a run in which the engine
dereferenced a blank pool would log identically to a correct one.

★ The C's answer was a **double mapping** — CPU side at the framebuffer address, GPU side FIXED at
the guest VA, **both sharing one host object** — plus a one-time seed copy. `kayfabe` deliberately
took the `gpu_only` shape and named the join as successor: `Request::ExportBacking`, *"already on
the wire"*, not routed to this path.

⇒ **This is the open question, and it is an ordering decision the owner owns:** the CPU view is the
**falsifier** as much as the fix, so it arguably belongs **before** the first boot in which an engine
executes guest methods, not after.

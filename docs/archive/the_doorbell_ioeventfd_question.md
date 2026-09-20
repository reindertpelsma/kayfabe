> ⊘⊘⊘ **ARCHIVED — THIS DESCRIBES A SUPERSEDED ARCHITECTURE. IT IS REFERENCE, NOT CURRENT.**
> The live design is `docs/design/THE_DESIGN.md`. This file is kept because its *measurements*,
> its *ogkm findings* and its *reasoning* remain useful — its **architecture does not**. Do not
> implement from it, and do not cite it as current. Archived 2026-09-21 (w822).

# The doorbell as a KVM ioeventfd — asked, measured against, and deferred again

**STATUS: DESIGN-ONLY, 2026-09-19 (w794). Not built. Reopening condition stated in §6.**

> **Owner, 2026-09-19**, proposing it: *"Register the doorbell with `KVM_IOEVENTFD` using
> `DATAMATCH`, one eventfd per valid token, created and destroyed with the channel lifecycle."*
>
> **Owner, same day**, arguing against his own proposal: *"Well it makes the trap itself cheap,
> but I am worried about the forward. It requires a context switch to a worker returning from
> epoll on those, to then send a dword. Now doorbell is just inline and one ring intentional,
> no context switch."*
>
> **Owner, on placement:** *"also for ioeventfd, idea is worker(s) handle it, they already have
> an epoll loop that could fit it."*

## 1. ⊘ This question already has a decision of record

`l2_qemu_adapter.md:1328`, **Q5**: *"The doorbell is a regular trap (Q-D1). ioeventfd-without-
datamatch is rejected on our core's own shape; **ioeventfd-with-datamatch is deferred behind a
named measurement.**"*

★ The 2026-09-19 brief re-derived the same architecture *and the same gating measurement*
independently. That is a good sign for the reasoning and a
`check_whether_the_question_is_already_answered` hit: the answer was *"deferred pending a
measurement"*, and **that measurement still has not been taken.**

## 2. What is actually true of today's path

- The doorbell page is a **read-only memslot on purpose**, so the guest's store exits and the
  token can be translated at all: *"One mapping, two permissions: the VMA is WRITABLE so the
  host doorbell can be rung with a translated token inline on the vCPU; the slot is READ-ONLY
  so the guest's doorbell store still exits"* (`shim.rs:5440-5445`).
- A **passthrough** doorbell is then served **inline on the vCPU**, by owner ruling:
  *"passthrough doorbells are inline in vcpu, no queue, no worker"* — `try_ring_passthrough_inline`
  returns before anything else runs (`shim.rs:6728-6731`, `:6909-6946`). It is one aligned
  volatile dword into a mapping already held.
- An **emulated** doorbell already **enqueues and returns** (`DoorbellAsyncArm::On` is the
  default since the 2026-09-09 owner ruling, `shim.rs:21505-21522`).
- §41 licenses exactly this and nothing more: *"a **passthrough** doorbell is ONE DWORD WRITE
  to the real register; an **emulated** doorbell puts the token in a queue."*

⊘ **So there IS a userspace round trip on the passthrough path, and ioeventfd would remove it.**
An earlier draft of this analysis claimed otherwise and was wrong.

## 3. What the brief gets right, and is worth keeping whatever happens

- **`DATAMATCH` carries the value in *which fd fired*.** Plain ioeventfd discards the written
  value, which would be fatal — the token must survive. Bounded token space makes this legal,
  and it is bounded: `VCHID_SPACE = 1 << 12`, sized *from the encoding*
  (`NV_CTRL_VF_DOORBELL_VECTOR` is bits 11:0), not from a channel count (`shim.rs:6565-6572`).
- **Coalescing is safe here, and that is not generic.** An eventfd is a counter; five rings
  collapse to one wakeup. Correct *because the operation is idempotent* — the host inspects
  current USERD, not a log of rings. ★ Most notification paths do not have this property, and
  saying so explicitly is what stops a later reader treating the collapse as a bug.
- **Measure first.** The brief makes its own premise falsifiable. That is the right shape.

## 4. Why the owner's counter-argument is the one that survives

ioeventfd moves the forward **off the vCPU and after it resumes**: exit → KVM match → eventfd →
(vCPU runs) → epoll wake → context switch → *the same dword*.

For a fire-and-forget doorbell that is free. ⚠ **For submit-then-poll it may not be.** The guest
rings and immediately begins polling for completion; today the dword has already been written
before the vCPU resumes, and after this change it has not. The trap gets cheaper and the
*completion* gets later. Whether that trade is positive is a workload question, not an
architectural one — and CUDA is submit-then-poll.

★ The owner's placement note removes the one structural objection: the workers already run an
`epoll` set (`kayfabe-shell/src/reactor.rs`, `sources.rs`), and §37 exists precisely because
*"a raw client yields an `eventfd`, so the worker polls its own semaphore inside its own
`epoll` set"* (`THE_CONSTRAINTS.md:882-893`). There is no new thread and no new loop — the
wakeup lands where the work already dispatches.

## 5. The doorbell is not where the time goes — measured TODAY, not cited from 2026-08

⚠ An earlier draft cited `worst_trap = 1.79 s at bar0+0x110c00` (w447). **The owner corrected
that as stale and was right** — `a_rulings_date_is_part_of_the_citation`, committed by the
person who wrote that lesson down. Re-measured from this session's own boots:

| boots | worst_trap | site | slow_traps(>1000us) |
|---|---|---|---|
| ×10 | **9 107 us** | `bar0+0xb81608` | 6 |
| ×3  | **1 275 us** | `bar0+0x110c00` | 1 |
| ×1  | **13 879 us** | `bar0+0xb81610` | 3 |

⇒ Worst traps are **1.3–13.9 ms**, not 1.79 s — 130–1400× better. Not sub-millisecond yet, but
the point that survives on *this* evidence rather than the stale one: **`bar0+0xbb0090`, the
doorbell aperture, does not appear in these boots at all.**

## 6. ⊘⊘⊘ THE BLOCKER IS THAT NOBODY KNOWS WHAT ONE DOORBELL COSTS

`c_vs_rust_per_launch_path.md:261`: *"**No timing instrumentation on our doorbell path at all.**
No histogram, no span, no per-doorbell duration counter — only counters… **NOT A COST — A CAUSE
OF NOT KNOWING.**"*

Every number we have is the wrong shape:
- `worst_trap` is **per boot** and names *the site, never the cause* (§41) — the register the
  guest touched, not what the trap waited on.
- `DBL_RATIO_X` (`w386`) measures **guest p50 ~512–683 us against a native ~9 us floor** — but
  that is the whole submit round trip, not the trap.
- The only ioeventfd delta in the tree is from a **synthetic QEMU spike device**, not ours:
  doorbell p50 **50 us trapped → 29 us ioeventfd** (`qemu_bql_spike.md:183-190`).

### Recommendation

**Build the histogram, not the ioeventfd.** A per-doorbell span (passthrough vs emulated,
p50/p90/p99) is cheap, is missing, is §41-compatible (it measures, it does not add work to the
trap), and is the brief's own stated precondition. It converts the owner's counter-argument and
the proposal from two opinions into a comparison.

### Reopening condition

Reopen when the histogram shows the **passthrough** doorbell's userspace round trip is a
material share of a launch-heavy workload's time. Then the ten-line test the brief names is the
next gate, and it is worth doing early because the pairing is unusual: **does an ioeventfd fire
for a write to a `KVM_MEM_READONLY` memslot at the same GPA?** Tracing KVM says it should —
a write to a read-only slot goes through emulation into `vcpu_mmio_write()` → `kvm_io_bus_write()`,
where ioeventfds are matched — but the read-only mapping is load-bearing for the passthrough
reads of the counter page and cannot simply be dropped if the combination fails.

⊘ Also unmeasured and cheap to check first: fd count vs `ulimit -n`, the KVM io-bus device cap
against 4096 tokens, and registration churn if channels are short-lived.

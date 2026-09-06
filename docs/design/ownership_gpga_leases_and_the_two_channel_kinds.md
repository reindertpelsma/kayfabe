# The two channel kinds, and who owns memory — the ruling that closes w296

**STATUS: LIVE, 2026-09-06. OWNER RULING.** Closes the ruling request opened by `da86fc26`
(w296, 2026-08-14), which named five red tests as *"a real product decision"* and stopped at
them. ⊘ Nothing here is implemented yet; this is the decision, its reasoning, and the gates it
obliges. Read `the_nine_red_tests.md` for the failures this explains.

**Self-contained on purpose.** It restates its own premises rather than citing a conversation,
because the thing it settles will be asked again after everyone involved has forgotten it.

---

## 1. The blocker, as w296 established it

A guest copy-engine doorbell **cannot reach the host-completion observer on either channel
kind.** Not by oversight — by two decisions each correct alone.

```
GuestChannelKind::Emulated  ⟺  anchor is SYSTEM_ANCHOR  ⟺  routes to SYSTEM_PROC
                                                        ⟹  §12.26 refuses host backing
                                                        ⟹  no operand can be host-backed
                                                        ⟹  CeExecutor::HostCe unreachable
                                                        ⟹  await_semaphore unreachable
```
and on the other kind, `8cca3502` (w287) scoped ring-content forwarding **off** passthrough
channels — rightly, because `[measured, w283d]` one CE doorbell rang the adopted channel *and*
decoded that same guest ring onto a different host channel. *"`ce_copy` **drives** a channel;
adoption means the **guest** drives it; both cannot hold."*

⇒ **On the kind whose ring we read, no operand may be host-backed; on the kind whose operands
may be host-backed, we do not read the ring.**

Citations: `SYSTEM_ANCHOR` `kayfabe-core/src/project.rs:312`; `GuestChannelKind`
`kayfabe-core/src/channel_kind.rs` (`56256793`, 2026-08-11); `ring_content_is_forwardable`
`kayfabe-rt/src/device.rs:5202`; the three `Binding::host` sites — `publish_backing`,
`plan_pin_guest_ram` (`kayfabe-fwd:1687`), `plan_back_fb_leaf` (`:2397`).

---

## 2. ★★★★★ THE RULING, PART ONE — WHAT THE TWO KINDS ACTUALLY ARE

### 2.1 `Emulated` means **implement**, not intercept-and-forward

An emulated channel is **not** a channel we inspect, rewrite and pass along. Everything about
it is ours: **operands, GPFIFO buffers, USERD, and the ring.** A guest submission on it is a
**remote procedure call into kayfabe**, and we answer it with **our own function body**.

★ The governing analogy, and it is exact: a guest kernel does not forward syscalls to the VMM.
The syscall lands in ring 0 and the kernel runs *its own implementation*, which may internally
do asynchronous I/O, cache, batch or elide — and then returns. QEMU serves a guest's disk from
a qcow2 file through ordinary userspace calls while the guest sees a real disk. Nothing is
"forwarded"; something is **implemented**.

⇒ Consequences that follow directly, and are the parts most easily got wrong:

- **There is no host doorbell for an emulated channel.** Only a guest doorbell exists. If the
  function body needs real GPU work — a copy, say — it calls a kayfabe library function that
  runs against our **scratchpad**. A scratchpad doorbell is an implementation detail *inside a
  function body*, never a forwarded data structure.
- **Only a pointer is ever not-copied.** In a CE copy the data pointer — a GPGA address or a
  VA — is what the body resolves; the body handles it to VAs in whatever scratchpad it uses.
- **Many privileged-command bodies may be entirely fabricated.** That is the whitepaper design,
  and the invariant that licenses it is: **only observable state in guest userspace must be
  correct.** See §5.2 for the condition attached to this.

### 2.2 `Passthrough` means **do not look**

Deliberately the smaller of the two. We do the setup controls, and then: a doorbell token
arrives, we translate it, we ring, we return. We do not inspect the ring, we do not decode
methods, we do not wait, we write no semaphores. The guest writes `GP_PUT`; hardware writes
`GP_GET` and the guest's own semaphore. What happens on the other side is not our business.

★ **This is the single most optimisable trap in the whole of kayfabe**, and it is the only kind
where a host doorbell maps to a guest doorbell at all.

⚠ **Parity ceiling, stated so nobody chases it:** the token translation on this path is
irreducible while the guest driver is stock. No amount of engineering removes it. Paravirtual
forwarding wins here permanently, and that is expected rather than a defect. The rung's
deliverable is *"runs correctly, with the remaining gap **attributed**"*, never a ratio.

---

## 3. ★★★★★ THE RULING, PART TWO — THE CRUX: MEMORY IS NOT OWNED BY PROCESSES

> ### **GPU work is owned by processes (or by the scratchpad). Memory is owned globally.
> ### Processes borrow from it through allocations.**

GPGA — the guest-physical GPU address space we present — is **one contiguous allocation space**
of memory objects, real or emulated. **It is owned by no process.** A process **leases** a
chunk, uses it, releases it; the bytes remain, exactly as on real hardware, until the range is
re-leased.

This is a deliberate departure from nvkvm-pv, and the advantage is explicitness: when the guest
is out of GPU memory we can **return a clean error**, where an implicit model discovers it as a
silent fault after a PTE write into nothing.

---

## 4. ★★★★★ WHY THIS SATISFIES §12.26 RATHER THAN RELAXING IT

This is the load-bearing paragraph of the document.

§12.26's refusal states its own reason at the site:
> *"the system proc's work is forged precisely so it can hold **no host state whose reclaim has
> no defined point**, and a framebuffer object is host state."*

The operative clause is **"whose reclaim has no defined point."** That is a statement about
**lifetime** — procs were merely the only thing in the model that *had* a lifetime to hang it
on. Once GPGA owns memory and defines its own reclaim point (a range is reclaimed when
re-leased), **the premise no longer holds for GPGA-backed memory.**

⇒ §12.26 is **not weakened**. It goes on protecting exactly what it was written to protect —
the *system proc* holding *proc-scoped* host state with no reclaim point. GPGA-backed memory
is simply **outside its scope**, because its reclaim is defined.

⊘ **The distinction matters and must not be lost in a later rewrite:** a change that *satisfies*
a refusal's precondition is sound; a change that *exempts* a case from the refusal is a
weakening wearing the same diff. This one is the first kind. Any future edit that keeps the
outcome but drops the reclaim-point guarantee has silently converted it into the second.

★ **And the data model is already most of the way there.** `kayfabe-mmu::RegionKind`
(`crates/kayfabe-mmu/src/lib.rs:469`) classifies GPGA by **where the bytes live** — kind 2 the
fake framebuffer, kind 3 a range re-pointed at host pages — and carries **no owner field at
all**. The proc-coupling is not in the memory model; it is only in the three `Binding::host`
sites, each keyed on `SYSTEM_PROC`. ⇒ The change is *"attribute host backing to the **region**
it lands in rather than to the **proc** that asked"*, not a memory-model rewrite.

---

## 5. WHAT MOVES, AND WHAT THAT OBLIGES

### 5.1 ★★★★ The no-forgery obligation RELOCATES — it does not disappear

The three red tests in `doorbell_reaches_the_completion_observer.rs` assert, in effect, *"a
guest doorbell reaches the host completion observer."* That is a **passthrough-shaped**
assertion: it presumes a host doorbell exists for the channel. Under §2.1 it does not.

⇒ Under an emulated channel a guest doorbell reaches a **function body**, and the invariant
belongs **inside the body**: *a body must not report work done that it did not do.* Same
prohibition, correct seam.

⚠ **The replacement must be written before the old assertion is retired.** Deleting an
instrument on the strength of an explanation is the failure mode this project has paid for
repeatedly, and a sibling project lost an NVDEC regression detector exactly that way — the
wrong explanation removed the test. The old tests stay red until the new ones exist.

Minimum replacement set for the emulated path:
1. a body that performs no work reports no completion;
2. a body that performs asynchronous work does not report completion before it observes it;
3. the ring is bound and the working set admissible before any body runs;
4. we never report `scheduled` for a channel whose ring is not mapped.

### 5.2 ⚠ A fabricated body must DECLARE what it makes true

§2.1 licenses fabricated function bodies, and the licence is real. The condition is that each
body **declares the observable guest-userspace state it guarantees.**

⊘ Without that, *"fake"* drifts into *"wrong"* with no diff to catch it — and this exact shape
already cost this project a day. The C prototype forged completions **correctly, for the kernel
scrubber channels only**, and that scoping existed solely in a source comment; it was read as
*"the C forged everything"*, and a whole lane was aimed at a question that reading had retired.
A per-body declaration is what makes the scope checkable instead of remembered.

### 5.3 ★★★★★ THE HARD GATE: SCRUB ELISION IS WHERE A CROSS-PROCESS LEAK ENTERS

With leases instead of ownership, **cross-process isolation rests entirely on scrub
discipline.** Scrubs of in-use memory execute as issued — correctness first, that is settled.

The risk is the optimisation: *"do not scrub a range that was freshly allocated and never
touched,"* or *"do not scrub if the next allocation will scrub anyway."* Both are legitimate.
Both are also **exactly** where a leak would enter, and the failure is **silent** — a wrongly
elided scrub produces output indistinguishable from a correct one, until another tenant reads
the bytes.

⇒ **Required, not optional:**
- the elision predicate is **conservative** — it elides only where residency proves nothing was
  written, never where it merely has no record of a write (*"not found" is not "not written"*);
- it carries a **known-positive**: a test that elides on a range that **was** written and
  proves the guard catches it. A predicate that can never refuse is worth nothing, and this
  project has shipped one before.

Applying the standing question — *if this is wrong, what goes unnoticed?* — the answer is
**another tenant's VRAM contents**. That is why this is a gate and not a comment.

---

## 6. ◐ DEFERRED, RECORDED SO IT IS NOT RE-DERIVED

**Full vGPU-style reservation** — requiring that all GPU memory the guest can reach is reserved
up front. It would eliminate ambiguous out-of-memory behaviour permanently, at the cost of
oversubscription. Genuinely attractive; nothing forces the choice now. ⊘ Note the asymmetry
that makes deferral safe: a fixed GPGA range already makes leaked memory **bounded** rather than
unbounded, which is a different risk class from an implicit model, reservation or not.

---

## 7. What this unblocks

The CE completion path, which is on the LLM-compute critical path. With host backing attributed
to the region rather than the proc, an emulated channel's operands may be host-backed, its
function body may call the real engine, and the completion it reports can be one it observed.

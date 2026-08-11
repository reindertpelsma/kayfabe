# The CE executor decision tree, and the trusted scratch VAS

**STATUS: LIVE — owner's design 2026-08-07, EXTENDED by the owner 2026-08-11 (§Scope-2026-08-11
below, which folds in above the §Scope paragraph it extends).** The 2026-08-07 rule about
executor choice is unchanged and unqualified. What 2026-08-11 adds is a *second* axis — the
same populations, now with a rule about **which thread** may do the work — and it is new
normative content, not a restatement of anything already here.

**Owner's design, 2026-08-07**, recorded here because it is load-bearing and was worked out in
conversation. ⊘ This file is the *rule*; `execution_plane_increments.md` §14 (E10) is the
increment that implements it. Where they disagree, this one states intent and that one states
what is built.

## ★★★ The rule

**The executor is chosen by where the bytes actually LIVE, not by what the guest asked for.**

A different executor producing a **true end-state** is *forwarding*, not forgery.
`mode2_forwarding_model.md`: *correctness = observable end-states only*.

⊘ **Forbidden**, and these are the only two things forbidden:
1. Signalling completion for work that did not happen.
2. Landing the data where the guest cannot see it.

★ (2) is `#12` in the C artifact — the completion went to the framebuffer while the guest read
`pbCpuVA`, and it cost weeks. Its lesson is about **aperture**, not about executor identity. An
earlier reading of this repo's principles treated "the host GPU must be the one that did it" as
the rule; the owner corrected that on 2026-08-07 and the correction is what makes the scrubber
servable without a new memory-plane primitive.

## The tree

```
STEP 0 — TRANSLATE
  Bring every operand into one address space (GPGA).
  ⊘ Forward-populated only, never reverse-resolved. A miss is a FAULT, never a guess.
  ⚠ Requires the phys-mode TARGET (LOCAL_FB vs SYSMEM). A raw physical address is AMBIGUOUS
    without it — an FB offset and a guest GPA can collide numerically. (E10a landed this.)

STEP 1 — RESIDENCY  (correctness — always first)
  For each operand: host-RAM-backed | real-host-device-memory | unreachable.
  Unreachable ⇒ fault by name.

STEP 2 — EXECUTOR  (performance — only after residency is known)
  any operand in real device memory  ⇒ HOST GPU CE   (the alternative is PCIe reads)
  all operands host-RAM-backed       ⇒ CPU COPY      (beats submit→doorbell→semaphore-wait)
  small + both CPU-reachable         ⇒ CPU COPY regardless
  ⚠ Size is a tie-break WITHIN the both-reachable branch, never a substitute for reachability.

STEP 3 — SIGNAL TRUTHFULLY
  Release the finishPayload semaphore, in the CHANNEL'S OWN APERTURE, only after the bytes
  are actually in place.
```

⚠ **Ordering between executors.** If some CE ops go to the host GPU and others to a CPU copy,
overlapping regions need a fence between them. Stated now rather than discovered as a race.

## ★★★★★ §Scope-2026-08-11 — THE SAME TWO POPULATIONS, NOW WITH A THREAD RULE (owner)

> *"The emulated arm must not block the vCPU"* → owner: **"yes schedule work asynchronously not
> during the trap"**

This folds in **above** the §Scope paragraph below because it is that paragraph's second half:
§Scope splits doorbells into *guest-userspace passthrough* and *kernel-originated*, and this says
what the **doorbell trap** may do for each.

| population | contract | what the trap does |
|---|---|---|
| **passthrough** (guest unprivileged userspace) | `RingAndReturn` | resolve the guest token to its host token, ring it, return to VM entry. **No inspection, no work.** Bounded by construction, so "non-blocking" needs no separate argument — it is the same fact as §Scope's *"we do not inspect them"*. |
| **emulated** (guest privileged kernel) | `ScheduleAndReturn` | **schedule** the channel's handler and return. ⚠ The handler must **not** run on the vCPU thread. |

★ Both contracts are now types: `kayfabe_core::channel_kind::{GuestChannelKind, TrapContract}`,
with `GuestChannelKind::trap_contract()` total over the kinds and
`TrapContract::may_run_on_the_vcpu_thread()` the one predicate. ⊘ **Declared and reported, not
enforced** — Rust cannot see thread identity, and a witness token would have nothing to guard
until the emulated handler is a separable object. That is stated at the type, not hidden.

### ⊘ THREE THINGS MEASURED ON 2026-08-11 THAT CHANGE WHAT THIS RUNG IS

1. **The trap is INLINE end to end today, and the whole emulated arm violates the new contract.**
   QEMU BQL → `kayfabe_shim_regs_write` (`shim_unsafe.rs:1301`) → `RegPlane::write`
   (`plane.rs:3107`) → `ring_doorbell` (`plane.rs:3328`, `RwLock` read **held across** the call)
   → `SharedDoorbell::ring` (`shim.rs:3517`) → `try_ce_submission` → `ceutils::run_submission`,
   under `ce_session_with_root`'s FSM mutex and a rank-0 device read. **No spawn, no channel
   send, no queue push anywhere on that path.** `completion_wait_architecture.md` §0 already said
   it in one sentence — *"The op finishes before the MMIO write returns"* — and this ruling is
   what makes that a defect rather than a description.
2. ⊘ **The mechanism the ruling asks for EXISTS, is NAMED, and must not be rebuilt** — but the
   "reactor is unreached" citation is **STALE**. Audit `026374c` measured `Reactor::new`,
   `register_source` and `arm_counter` at zero production call sites; `w226` built the composition
   root and each now has **exactly one**: `Regs::start_completion_observer`
   (`shim.rs:6576`) → thread `kayfabe-completion-observer`. `completion_observer.md` §2.4 already
   recorded this. ⚠ Two live scope limits: the whole root is `#[cfg(feature = "host-isolates")]`,
   so the **default archive has no observer thread at all**; and the reactor's output is dropped
   at birth (`let (tx, _rx) = inbox()`), so `SourceRegistry::dispatch` is still unreached and the
   inbox grows one `CoreEvent` per poke, unbounded.
   ★ `Regs::spawn_completion_observer`, cited in `shim.rs`, **never existed** — one hit in the
   whole repo, in the comment citing it. Fixed.
3. ⊘ **Polled vs interrupt-announced is NOT a property of the channel** and must not be added to
   its kind. It is `AWAKEN_ENABLE`, `D[20:20]` of the guest's own `SET_REPORT_SEMAPHORE_D`,
   decoded **per submission** into `completion_watch::CompletionDecl::awaken`; one channel may
   carry submissions with either value. ★ And the finding worth carrying: `awaken` is decoded,
   printed, and **branched on by nothing** — one decode, two prints, one test assertion, zero
   conditions. The split does not need inventing; it needs a **decision point**, one layer below
   a channel's kind. ⇒ Async scheduling ships for the polled case with **no delivery path**, and
   only the announced case waits on F6's masked leaf.

## ★ Scope: this governs KERNEL-originated CE only

Guest **userspace** pushbuffers are mapped straight into the GPU and are **passthrough** — we do
not inspect them, and CE there is always real. This is why "bulk memory movement at LLM scale"
is mostly not our problem: those copies never reach our code. Only syscall-mediated cases (e.g.
cross-process) do, and the C proved CE-forward works for kernel-originated CE on the compute
path (`cup8`, 2048² matmul at `bad=0 maxerr=0`, committed as `cap3_matmul_forwarding`).
⚠ Cross-process CE specifically is **unmeasured in Mode-2** — the C runs exactly one CUDA
process per QEMU lifetime (`mode2_bench_lifecycle.md` §1); Mode-1's per-`mm` isolates are the
multi-process precedent.

## ★★★ The trusted scratch VAS (owner's idea, 2026-08-07)

**Problem.** A guest op names a GPGA and we need a real host-GPU operation over it, but there is
no equivalent *safe* GPU VA — the guest's own VAS may not exist host-side, may not cover the
operand, or (as with the scrubber) the operand may be physical-mode with no VA at all.

**Answer.** The VMM/trusted side owns a **scratch GPU VAS the guest never sees**. When a real
host-GPU op must reach bytes we own, we map them into *our* scratch VAS and issue the op there.

★ Why this is better than reproducing the guest's addressing:
- The guest's VA layout is **irrelevant** — we only need *our own* consistent naming for bytes
  we already own. That is a far weaker requirement than mirroring a guest VAS.
- It is a **security improvement**, not merely a convenience: a guest-supplied address never
  becomes a host GPU address directly. It is translated into a space *we* control, with *our*
  bounds — the same discipline as §4.2.1's bounded objects, applied to the GPU's address space.

⊘ **Scope it PER-ISOLATE, not per-device.** A shared scratch VAS would be a cross-tenant channel
— two guests' bytes namable in one address space is exactly `#14`'s defect class lifted into the
GPU MMU. The scratch VAS belongs to the isolate that owns the process.

⚠ Three costs to design for, not discover:
1. **Lifetime** — mappings must be torn down or host GMMU entries leak. Needs an ownership
   regime, not an ad-hoc map call.
2. **Aliasing** — two ops mapping the same GPGA concurrently should get the *same* scratch VA,
   or coherence is undefined.
3. **Cost** — map/unmap per op is expensive (TLB + invalidate). Wants a cache with a residency
   policy, which is a design, not an optimisation to bolt on later.

## Where the executors can live

★★ `[measured]` E10, 2026-08-07: **the CPU branch cannot execute in the isolate.** The isolate
is a separate sandboxed process that deliberately holds neither the emulated framebuffer nor
guest RAM. So `ce_copy(Ours)` must keep refusing there, and the CPU executor belongs in the
**shell**, which holds `SparseFb` + `Vmm`.

⊘ This corrects §12.4's standing claim that *"the executor is the isolate in both cases."*
It is not a gap — it is the security boundary refusing to leak guest memory into the sandbox,
working as designed.

| executor | lives in | reaches |
|---|---|---|
| CPU copy | **shell** | emulated FB (`SparseFb`) + guest RAM (`Vmm`) |
| host GPU CE | **isolate** | real host device memory; guest RAM only via the DMA primitive below |

## The `OS_DESCRIPTOR` primitive — what it is, and when it is actually needed

`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` inverts the normal RM allocation: instead of *"give me
memory"*, it says *"here is host virtual memory that already exists — pin it and map it into the
GPU's address space."* The chain it enables:

```
guest GPA → offset in the memfd backing guest RAM → host VA (isolate mmaps it)
          → OS_DESCRIPTOR alloc registering that range with host RM
          → RM pins + maps into the host GMMU → a host CE can name it
```

⊘ **Not built**: `ExportSource::HostDeviceMemory` is `NotExportableAsMemory` *by design*, and the
class name exists only in an ABI allowlist with no alloc path. The C had this as Mode 1's
double-mmap / memfd-migration isolate, which is the concrete form of the head start Mode 2
inherited.

★ **It is NOT needed for the scrubber** — the CPU branch reaches both operands. It becomes
unavoidable only when a **real host GPU must touch guest RAM**, i.e. real userspace compute.
Deferred, not dismissed.

## FERMI_VASPACE_A, for the record

An RM object class representing a **GPU virtual address space** — a page-table hierarchy with
its own PDB. ⊘ Not BAR space (a CPU-side window onto device memory) and not an offset: a GPU VA
walks the GMMU through this VAS and resolves to a physical address, which here is a GPGA or a
guest sysmem page. "FERMI" is a class-name generation, inherited forward and current on Ampere —
like the Kepler-era `a06c`/`a06f` control numbers.

★ Which is why `NoVas(ChanId 1)` still matters even on the CPU branch: the scrubber's **ring and
finishPayload semaphore are virtual** (addressed through its VAS) even though the copy operands
are physical. Without publishing that VAS we cannot find the ring.

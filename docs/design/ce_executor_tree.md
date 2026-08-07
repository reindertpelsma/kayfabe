# The CE executor decision tree, and the trusted scratch VAS

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

## ★ Scope: this governs KERNEL-originated CE only

Guest **userspace** pushbuffers are mapped straight into the GPU and are **passthrough** — we do
not inspect them, and CE there is always real. This is why "bulk memory movement at LLM scale"
is mostly not our problem: those copies never reach our code. Only syscall-mediated cases (e.g.
cross-process) do, and the C proved CE-forward works for those.

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

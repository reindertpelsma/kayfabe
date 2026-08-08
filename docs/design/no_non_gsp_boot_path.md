# ⊘ DECIDED: kayfabe does not support a non-GSP boot path

**Owner raised it, coordinator recommended, 2026-08-08.** Recorded so the question is not
re-litigated every time someone meets a pre-Turing GPU. ⊘ This is a **scope boundary**, not a
statement that the work is impossible.

## The question

Nouveau (and NVIDIA's proprietary driver) can bring a GPU up **without GSP**. Some GPUs cannot use
GSP at all. Should kayfabe accept a second, GSP-less boot sequence so those parts work?

## ★★★ The decisive argument: it is not a MODE, it is a SECOND DEVICE MODEL

This port's entire architecture is *"we are the GSP."* The faked GSP and the RPC plane **are** the
product. Everything the campaign has built — serving `0x90f10106`, the static-info reply, the
doorbell executor, the control census, the golden-context boundary — exists because **CPU-RM
delegates to firmware and we answer in firmware's place**.

Remove GSP and CPU-RM drives the hardware **directly**: PMU and falcon bring-up, PGRAPH init,
register-level sequencing, per-chip init tables. That is not an extension of this boot path; it is
a different and much larger emulation surface.

⇒ ★ And it is **more per-chip, not less** — which cuts directly against the universality goal that
motivates the whole rewrite. The C artifact's founding insight, *"fake the boot"*, works precisely
**because** the GSP indirection exists to stand in for. Without it there is nothing to stand in
for.

## The supporting reasons

1. ⚠ **It changes what ground truth is available — and the owner CORRECTED my first version of
   this point, rightly.** I originally wrote that a GSP-less target would leave us with *no* open
   reference. That is wrong: **nouveau is open and documents non-GSP bring-up**, so the
   register-level sequence is not a blind reverse-engineering problem.

   ★ **What survives the correction, and it is subtler:** the open kernel modules require
   firmware, so a GSP-less guest runs the **proprietary** NVIDIA driver. Nouveau then tells us
   what *nouveau* believes about the hardware — excellent, hard-won, and **not** a statement about
   what the proprietary driver expects. ⇒ we would trade a **first-party** oracle (NVIDIA's own
   RM source, which we can *compile and run* against our answers — five oracles do exactly that,
   `isolate_the_drivers_own_checks`) for a **third-party model of the same silicon**, while the
   thing being satisfied is first-party code we could no longer read.

   ⊘ So this is a real cost, but a smaller and more specific one than "no reference exists".

   ★★★ **AND THE OWNER SHARPENED IT FURTHER, which is what actually settles the question.** Three
   points, all theirs:

   - **Nouveau does not implement every op the proprietary driver runs.** A GSP-less guest runs the
     proprietary driver, which will issue sequences nouveau never issues. Those have **no
     reference at all** — not a second-hand one, none.
   - ★★ **Nouveau's bring-up leans on EMPIRICAL constants — and the refined version of this is
     sharper than "it is all replay", which would be unfair and wrong.** Nouveau's knowledge sorts
     roughly into: **(A)** architectural/semantic, **(B)** functional-but-unexplained, **(C)**
     empirical recipe (*"NVIDIA writes `0x00100064` here; anything else breaks init"*), **(D)**
     opaque firmware (FECS/GPCCS blobs loaded into Falcon IMEM/DMEM through a protocol nouveau
     understands well while the blob's internals stay closed).

     ★ For **runtime** operation nouveau is largely A/B — it dynamically builds VM mappings,
     channels, contexts, fences, pushbuffers and interrupts, which cannot be fixed trace replay.
     ⊘ **But C concentrates precisely in BRING-UP**: `{address, count, stride, data}` init tables,
     GR init sequences, *"some context buffer of unknown purpose"* that it nonetheless allocates
     and maps correctly, and memory-training writes whose own comments say *"magic writes that
     improve train reliability?"*.

     ⇒ ★★★ **And bring-up is exactly and only the part we would need.** So the argument is not
     "nouveau doesn't understand its GPU" — it does, mostly. It is that **the one region where its
     knowledge is weakest is the one region we would be copying**, and we would inherit the
     weakness without inheriting the way out: nouveau earned its A-level understanding by
     experimenting against real silicon over years, and we would be re-deriving semantics from a
     recipe with no spec-holder to check against.
   - ⇒ **We would be building against a replay, because there is no spec and nobody to show us
     one.**

   ★★★ **The consequence is the real one: this project's CENTRAL VERIFICATION TECHNIQUE would not
   exist.** Everything that has gone right here came from *compiling the guest's own acceptance
   checks and running them* — five oracles, and every debugging win of the campaign. Against a
   replay there is **no acceptance path to compile**, because nouveau is not the thing we must
   satisfy; the closed proprietary driver is.

   ⚠ And we know what that failure mode looks like, because the C artifact lived it **with**
   readable source available: it resolved one VA to three different wrong pages, reversed its own
   aperture conclusion twice, and shipped `dlen = 0` rows that were positively wrong
   (`c_oracle_empty_rows_are_wrong`). Remove the oracle and that stops being the exception.
2. **The affected silicon is old.** GSP-less means pre-Turing in practice; Pascal is 2016.
3. **Those parts already have an answer.** vGPU unlock exists for them and is reported reliable.
4. **The ecosystem moved.** Current drivers — including nouveau, and including Windows guests —
   attempt GSP by default.

## ⚠ What is NOT claimed

- ⊘ Not that non-GSP emulation is infeasible. It is a coherent project; it is a **different** one.
- ⚠ The **laptop** case is the least certain input. `[inferred]` modern mobile Turing+ parts use
  GSP as desktop parts do, and GSP-less configurations are mainly pre-Turing or
  `NVreg_EnableGpuFirmware=0` on the proprietary driver. **This has not been verified against
  current hardware.** ★ It does not change the decision, because the architectural argument stands
  whatever the population size — but a scope decision should not rest on an unverified population
  estimate, and this one does not.

## ★ The sub-question that IS worth answering, and is being answered anyway

The owner also asked to *"enumerate exactly which RMs the GSP normally handles and which functions
it does, like golden ctx."* That is **our responsibility surface** and it is valuable independent
of this decision — it defines what "complete" means.

Much of it is already measured:
- `traces/real_ga106/rpc_transcript_real_ga106.txt` — **55 distinct controls** across one complete
  `RmInitAdapter` (`docs/reference/remaining_boot_surface.md` §1).
- The control **census** distinguishes served / served-but-REFUSED / never-seen per boot.
- The **golden-context** boundary is characterised (`docs/design/c_cuda_ladder.md` item 3).

⇒ What is missing is the **post-boot** surface — what GSP handles once CUDA is running. ★ The CUDA
ladder (`cup2 → cupctx2_min → cup8`) enumerates exactly that **for free** as it climbs, measured
rather than predicted. ⊘ So it does not need a separate research task; it needs the ladder.

## Revisit if

A **current** driver on **current** silicon is found to run GSP-less in a configuration customers
actually use. Then this is re-opened on evidence, not on the possibility.

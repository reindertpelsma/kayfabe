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

1. ⊘ **It would forfeit `ogkm` as ground truth.** The open kernel modules require firmware, so a
   GSP-less target means committing to the **proprietary** driver permanently. That is the loss
   that actually hurts: five compiled oracles came from readable RM source, and essentially every
   debugging win of this campaign traced back to having it
   ([[isolate_the_drivers_own_checks]], `docs/design/` oracle work).
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

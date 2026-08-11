# w248 — PREDICTIONS, recorded BEFORE the run

Recorded 2026-08-11, before `gpu_fault_containment.sh` was executed on `vh` (RTX 3060 GA106,
host driver 580.159.04). Basis: the script and `gpu_wedge_probe.c` read at `acbb9a3`; no prior
run of this script for this question exists.

## What is being asked

`gr_execution_boundary.md` property **3 — FAULTING/CONTAINED**: *"an unmapped VA raises an MMU
fault rather than aliasing anything, and that fault is contained to this channel's TSG.
⊘ `[NOT MEASURED]` whether a GR MMU fault on this bench is contained to one channel or takes the
host GPU context with it."*

## ⚠ Scope, stated BEFORE the result, so it cannot be adjusted to fit

The attacker is a **host CUDA process faulting in its own context on its own channel** — not
guest-authored methods on our isolate's channel. The script says so itself: *"⊘ It is NOT a
malformed pushbuffer … What it shares with that shape is that it produces a REAL MMU fault and a
REAL Xid, so the escalation path is entered."*

⇒ What this can establish: **the blast radius of an MMU fault + Xid on this GPU and driver** —
whether RM's recovery is context-scoped or takes other live contexts with it. That is exactly the
*"or takes the host GPU context with it"* half of property 3.
⇒ What it cannot establish: that a fault raised **from our isolate's GR channel, in a VAS we
built, by guest methods** has the same radius. That needs GR execution, which property 2 blocks.

## Predictions

1. **Arm A** (baseline victim on an idle GPU): `rc=0`, `[victim] OK bad=0`.
2. **Arm B** (attacker faults alone): the launch or the sync returns a non-zero `CUresult`
   (expected `CUDA_ERROR_ILLEGAL_ADDRESS`, 700), the **Xid count increases**, and the attacker's
   **own context becomes sticky-dead** — `context reusable? rc!=0`.
3. **Arm B2** (fresh victim after the fault): `rc=0`. ⊘ Weak arm by construction.
4. ★★★ **Arm C — THE ARM THAT MATTERS** (victim holds a LIVE context across the fault):
   **victim exit=0** (`errors=0 wrong=0`) ⇒ **CONTAINED**. The fault is scoped to the offending
   context/channel; a live bystander context survives.
5. **Arm D** (aftermath): a brand-new victim `rc=0`, `nvidia-smi` still answers, **no**
   `fell off the bus` / `reboot` / `GPU has fallen` lines.

## ⇒ The falsifiers, named

- If **arm C's victim exits 4** (`context died`) ⇒ **NOT contained**: an MMU fault takes
  bystander contexts. That would make GR execution unsafe for a multi-tenant host by itself, and
  it is the single most important thing this run can say.
- If **arm C's victim exits 3** (`wrong bytes`) ⇒ worse than 4: silent corruption of a bystander.
- If **arm D** shows `fell off the bus` / reboot-required ⇒ node-level escalation
  (`guest_blast_radius.md` §7's third hazard), and the bench is compromised.

## ⚠ Operational risk, accepted deliberately

This run **deliberately Xids the host GPU**. If RM escalates, later bench runs on `vh` are
compromised until the host is re-rented (`vast_host_setup`: hosts are ephemeral — kill, never
nurse). Arm D is the check. The bench is idle and nothing else is queued behind it.

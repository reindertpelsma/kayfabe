# ROADMAP PHASE ORDERING — the owner's sequencing ruling

**STATUS — 2026-09-06 — LIVE. OWNER RULING.** Supersedes any ordering implied elsewhere in
`docs/`. The load-bearing clause is that **driver and architecture breadth comes BEFORE
graphics and before the full application matrix** — not after.

## The order

| # | phase | done when |
|---|---|---|
| 1 | **Compute works** | a real workload produces correct output on a stock guest driver — and is *graded on its output*, not on a count of outputs |
| 2 | **CUDA applications pass** | a set of real CUDA apps run and **must pass**; not a demo, a bar |
| 3 | **Driver + architecture support** | the support matrix broadened, **the way `nvkvm-pv` did it** |
| 4 | **Graphics, and the full application matrix** | — |

⊘ **Phase 3 before phase 4 is the whole point of the ruling.** The tempting order is the
reverse — graphics is more visible and more fun. It is also the order that makes every
graphics feature something you must then re-validate across the matrix. Doing breadth first
means phase 4 is built once, against a base that is already general.

## Why "like `nvkvm-pv`" is a specific instruction, not a vague one

`nvkvm-pv` is the shipped Mode-1 sibling (`/workspace/nvkvm-pv`, read-only, owned by another
agent). It already went through phase 3 and **paid for the failure modes**, and they are all
failure modes *of breadth*, not of depth:

- ★★★★★ **A capture-derived table expires as a vendor regression.** The defect tracked
  **driver version**, with two controlled comparisons. Any table we derived from a capture of
  580 is a hypothesis about 580, and it will present as a regression on the next driver rather
  than as a missing feature.
- ★★★★★ **The missing table row never denies.** Five instances across four architectures:
  allowlisted but **unsized** ⇒ zero bytes of params, a silent log line, no refusal. A new
  arch's new row does not announce itself.
- ★★★★★ **An advertised limit is a hypothesis about what binds.** The "16 MiB cap" was three
  limits and the binding one was undocumented.

⇒ Phase 3 is not "add more rows". It is **making the rows falsifiable across versions**, which
is a different engineering activity and needs its own instruments.

## What this ruling constrains, starting now (phase 1/2 work)

The compatibility axes are a **constraint on today's parity work**, not a later chore —
because phase 3 is where any shortcut taken now gets paid for at matrix scale.

- **No GA10x-specific fast path outside `kayfabe-chips`.** The arch seam is what made a second
  architecture additive; a parity hack in the shim silently re-couples it.
- **No QEMU-only mechanism.** `Vmm::defer` exists so deferral is expressible VMM-neutrally, and
  both adapters are byte-identical today. Anything added for parity goes through that seam.
- **No pinning to driver 580 or to the open module.** Both closed and open drivers must work;
  `ogkm` is versioned, not the spec.
- **aarch64 CI check stays green.**
- Only the **VMM axis** may drop versions — that asymmetry is the single degree of freedom.

⚠ **One already-known phase-3 debt, live today:** `Ad10xArch::mmu()` delegates to `MockArch`'s
**invented** GMMU format while the Ampere row is oracle-checked. That is precisely the shape
phase 3 exists to catch, and it is already flagged — it should be closed *as* phase 3 opens,
not discovered by it.

## What phase 1 is still missing, and it is not compute

★ The compute milestone is graded on `LLM_TOKENS`, a **count**. A run has already passed that
grade while emitting degenerate text under greedy decoding. **Phase 1 is not done until the
grade asserts the output**, because phase 2 ("CUDA apps must pass") inherits whatever
"pass" means here. A bar built on a count cannot carry a phase that says *must pass*.

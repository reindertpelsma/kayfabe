# w276 — PRE-REGISTRATION

**Written before the boots.** Branch `w276-port-the-whole-vas-sweep`, base `fffde60`.

## The one variable

`KAYFABE_PT_SWEEP`. Everything else is `w271_pin`'s arming byte for byte.

## The arms, and the least-weighted one is named

★ Six of the last eight rungs had their least-weighted arm fire. Weights are stated so that
cannot be re-assigned after the fact.

| arm | weight | what would make it fire |
|---|---|---|
| `CUP2_RC=0` | very low | the sweep is the whole wall |
| `CUP2_RC=1` (bounded) | low | progress past the wall into a different failure |
| `124`, GR fault **gone**, page still frozen | medium | corruption story, not mapping |
| `124`, fault **moved** | medium | the sweep bound something and exposed the next miss |
| `124`, fault **unmoved** | **high** | the sweep changed nothing observable |
| leaf **ABSENT** at the fault VA | **high — and it KILLS the rung** | the guest never described it ⇒ mirroring cannot bind it |
| the sweep **too expensive** at doorbell time | medium | truncation, or a boot that does not reach `cuCtxCreate` |
| ⊘ the sweep **publishes nothing** | *not weighted before the run* | — |

⊘⊘ The last row is stated as a gap in this table, not back-filled as a prediction. It is what
happened, and pretending it was anticipated would be the thing this file exists to prevent.

## What a null result here does NOT mean

`C: nvkvm_gpu_emul.c:8676-8688`, read verbatim from the C tree: *"at this release a root walk
of the PDB can't yet reach the page (**bench-proven: root re-walk read `runs=0` while the host
CE faulted one page past the last-backed leaf**), but the page itself already holds committed
PTEs."*

⇒ **The C measured a root walk as INSUFFICIENT at exactly this point.** A null result from the
sweep is *expected by the C* and refutes nothing about the address plane.

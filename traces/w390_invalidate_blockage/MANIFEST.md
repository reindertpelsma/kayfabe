# w390 — TLB-INVALIDATE BLOCKAGE POINT: evidence, 2026-09-07/08

Real GA106 (RTX 3060), open **580.159.04**, vast KVM template, box `kayfabe-w390-mode2-bench`
(instance 50171360, **destroyed 2026-09-08** — vast is compute, never storage).
Guest: stock unpatched NVIDIA 580.159.04 on 6.8.0-138-generic. Eleven boots.

⚠ **A bench claim without its SOURCE REVISION is not a claim.** Every rung's revision is below,
and each was gate-checked on the box (`STAMP == HEAD`, `w290p_run.sh` refuses otherwise).

| rung | revision | doorbell arm | invalidate | `CUP3_VAL` | host Xid | rows published |
|---|---|---|---|---|---|---|
| `a1` | 5aaebee8 | ⊘ **no arms at all** | off | hang | 0 | — (invalid run, see below) |
| `b2`,`b3`,`b4` | 5aaebee8 | `drain` | off | **43** | 0 | 74 (doorbell) |
| `c1` | 5a8d8fd7 | `drain` | `off` | **43** | 0 | 74 |
| `c2` | 5a8d8fd7 | `drain` | **`on`** | **43** | 0 | 1 (invalidate) |
| `c3` | 93e07bfd | `drain` | `on` | **43** | 0 | 1 |
| `c4` | a8459f0e | `drain` | `on` | **43** | 0 | 1 |
| `d1`,`d2` | 6c6bc0bf | ⊘ **VOID** | — | 43 | 0 | — |
| `e1` | 6c6bc0bf | `assert` | off | `NO_KERNEL_LINE` | **16 × Xid 31** | **0** |
| `e2` | 6c6bc0bf | `assert` | `on` | `NO_KERNEL_LINE` | **16 × Xid 31** | 1 ⊘ **VOID** |
| `f1` | d0655640 | `assert` | off | `NO_KERNEL_LINE` | **16 × Xid 31** | **0** |
| `f2` | d0655640 | `assert` | `on` | `NO_KERNEL_LINE` | **16 × Xid 31** | **25** |

## ⊘ THREE RUNS ARE VOID AND ARE KEPT ANYWAY — each records a trap, not a result

- **`a1`** — booted `boot_capture.sh` bare, so every arm sat at default (`GR_ROUTE=refuse`) and
  cup3 hung in `cuCtxCreate`. Not a defect; the known-positive recipe is `w290p_run.sh <arm>`.
- **`d1`/`d2`** — `KAYFABE_VAS_PUBLISH=assert` was passed **in the environment**, which
  `w290p_run.sh:105` overrides unconditionally from the positional `$ARM`. Both actually ran
  `arm=drain` with the doorbell publishing normally, and `d1`'s `43` was one message from being
  reported as *"publication is not needed for correctness"* — **the exact opposite of the
  truth.** Caught by grepping the arm ACTUALLY IN FORCE, not the env passed.
- **`e2`** — `PublishStamp` was inserted `if !budget_hit`, unconditional on whether anything was
  **published**, so the census-only `assert` doorbell stamped every epoch and suppressed the
  invalidate 293 times. **The arm I had disarmed was still suppressing the arm I was testing.**
  Fixed at `d0655640`; `f2` is the fair re-test.

## The finding

`f1` vs `b3` — publication **is** required. `f2` vs `b3` — the invalidate publishes **25 of the
74** rows the doorbell does, so it is a valid **barrier** and an invalid **notification**: the
guest fires no invalidate at all for two rows in three, exactly as
`NVOS46_FLAGS_DEFER_TLB_INVALIDATION` permits.

Full ruling: `docs/design/w390_the_invalidate_blockage_point.md`.
Harnesses that produced these: `scripts/bench/w390/`.
⊘ The multi-MB `run_*_qemu.log` device logs are NOT committed (4.3 MB each); the probe,
hostdmesg and guest-dmesg evidence is.

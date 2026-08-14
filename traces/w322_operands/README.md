# w322 — LOCATE THE OPERANDS: the artifacts

Write-up: `docs/design/w322_locate_the_operands.md`. Bench `vh2`, RTX 3060 GA106, driver
580.159.04, **PCIe gen3 ×16** (`pcie.link.gen.current=3 width=16` — recorded, not assumed).
Source at `scripts/bench/cup8bench.c`; each native log prints its own `source md5`.

## The ruler — native, no guest, no QEMU

| file | what |
|---|---|
| `native_summary.log` | the R=1 sweep across all five placements, with the cross-mode table |
| `native_bw_<mode>.log` | one placement's `bw` sweep, verbatim, including its negative control |
| `rep_<mode>_{1,2,3}.log` | ★ n=3 at 256 MiB. **This is where the instability lives**: `hostalloc` 11.886/10.887/9.613 against `hostreg` 12.350/12.321/12.315 |
| `native_mm_<mode>.log` | the matmul re-run of w320 §5.5 against an honest control |

## The guest

| file | what |
|---|---|
| `run_w322bw_probe.log.gz`, `run_w322bw2_probe.log.gz` | the headline aperture sweep, two boots |
| `run_w322bwneg_probe.log.gz` | ⊘ **the arm whose arming was DROPPED.** It reads `BENCH_MODE=MEASURE` and `BENCH_VERDICT: PASS`. Kept as the fixture for §6.7(1): a negative control that silently became a positive one. Its bandwidth rows are valid; its *control* is void |
| `run_w322bwhost_probe.log.gz` | the guest's own `cuMemHostAlloc` — 4 points, the fit that survives its guard, and 32/64 MiB succeeding where the default chain cannot |
| `run_w322sizes_probe.log.gz` | the same-hour matmul curve, `bad=0 maxerr=0` ×4 |
| `run_*_fb.log` | the 1 Hz host framebuffer / RSS sampler. ⊘ **Inconclusive** — see §6.4. Kept because a pre-registered instrument that did not resolve is a result |
| `run_w322c{1,2,3}cup3_probe.log.gz` | `CUP3_VAL=43`, n=3 |
| `w317_repeat_w322r33.log` | `R33 arm 1` fired, n=3, byte-identical each boot |
| `w322batch.log` | the serial batch's own log, including the arm that exited 127 while printing its terminator |

⚠ **Read `native_bw_*.log` for R=1 rows only.** The first sweep (superseded, not kept) ran the
pass R times per launch and the reuse set is `resident_threads × (NF/NT) × 4`, not the buffer —
it measured L2 at every placement. Any row whose `reps=` is not 1 is a cache number.

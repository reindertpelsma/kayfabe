# w320 — the sync-floor artifacts

Bench `vh2`, RTX 3060 GA106, driver 580.159.04, 2026-08-14.
⊘ `crates/` is byte-identical to master `c2b0f3e6`: this rung changed the measurement program
and nothing in the device.

| file | arm | revision | what it is |
|---|---|---|---|
| `run_w320ksweep_probe.log` | `ksweep` | `e8be2da5` | K launches per sync, N=128, K∈{1..128}×5 reps |
| `run_w320sizes_probe.log` | `sizes` | `e8be2da5` | ★ the duration discriminator, N∈{128,512,1024,2048} |
| `w320_native.log` | native VRAM | src md5 `562ad895` | the reference arm — SAME source as the guest arms |
| `w320_native2.log` | native VRAM | src md5 `e6f92ad2` | re-run at the committed source; reproduces within 3 % |
| `w320_native_hostmem.log` | ★ native HOST-MEM | src md5 `e6f92ad2` | the MECHANISM control — no guest, no QEMU |
| `run_w320q{1,2,3}_probe.log` | cup8 N=2048 ×3 | `468e29de` | the quietly-wrong workload, `bad=0 maxerr=0` ×3 |
| `run_w320negctrl_probe.log` | negctrl | `468e29de` | `BENCH_NOLAUNCH=1`, first and only context of its boot |
| `w320_corr.log` | the ladder | `468e29de` | cup3 + R33 ×3, cup8 ×3, negctrl |
| `w320_tests.log` | workspace suite | `468e29de` | 252 targets ok; failing set == master's |

## Reproducing

```
scripts/bench/w320_sync.sh ksweep     # the discriminator
scripts/bench/w320_sync.sh sizes      # sync vs kernel duration
scripts/bench/w320_sync.sh negctrl    # the guarded negative control
BENCH_SIZES=128,512,1024,2048 BENCH_HOSTMEM=1 scripts/bench/w311_native.sh   # the mechanism control

scripts/bench/w320_fit.py --selftest              # ★ run FIRST — it must refuse a 2-point fit
scripts/bench/w320_fit.py <probe.log> [...]       # THE DELIVERABLE
```

Knobs: `BENCH_BATCH_SWEEP`, `BENCH_BATCH_REPS`, `BENCH_CTX_FLAGS`, `BENCH_HOSTMEM` — all
default to the w318 behaviour, so existing arms remain one-variable against these.

⚠ Read `docs/design/w320_the_sync_floor.md` §5.3 before quoting any intercept from these logs.

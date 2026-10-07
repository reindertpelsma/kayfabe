# w385 — the concurrent-fuzz run logs

Source `b16dca06`..`2db47b0e`, RTX 3060 (GA106), host driver **580.159.04 open** kernel
module, kernel `6.8.0-59-generic`, vast instance **50080465** (19 cores, destroyed after the
run). ⊘ **The rented box is not the artefact; these files are.**

| file | what it is |
|---|---|
| `w385d_concurrent_fuzz.log` | ★ the ladder, all twelve arms + the watchdog control + the replay pair, as one table |
| `run_w385d_watchdog.log` | ★★ the WATCHDOG'S OWN NEGATIVE CONTROL — a 5 s deadline against 100 000 iterations. It fires, prints `RUNG_concurrent_fuzz=FAIL` with `FUZZ_REASON=DEADLOCK/WATCHDOG`, dumps all eight workers' last verb and iteration, and exits 3 |
| `run_w385c_t32i200c4.log` | ⊘ **THE RED**, before the fix: 66 `ALIAS_REVOKED`, 22 per 32-thread arm, all from tids 28/29, all at VAs in the one window that started at `0x100_0000_0000` |
| `w385_replay_of_the_red_after_the_fix.log` | ★ the SAME seed after the fix — byte-identical program (`ops=6400 engine_ops=1784 alias_ops=774`), `refused` 1200 → 0, `viol` 66 → 0 |
| `run_w385d_t48i200c4.log` | the widest arm: 48 workers on 4 cores, 615 195 measured cross-thread overlaps |
| `run_w385d_t3i5000c3.log` | the deepest arm: 120 000 ops at the guest's own `-smp 3` |

Read `docs/design/w385_the_concurrent_fuzz.md` §4 for what these do and do not say. ⚠ The
headline is a **budget, not a proof**: 1 599 703 sampled overlaps is 1 599 703 interleavings
out of an unbounded space.

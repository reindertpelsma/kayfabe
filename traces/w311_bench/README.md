# `traces/w311_bench/` — the w311 throughput measurement

Three runs, all on the SAME physical GA106 (`vh`, RTX 3060, host driver 580.159.04).
Full reading: `docs/design/w311_throughput_ratio_and_the_llm_question.md`.

| file | what it is |
|---|---|
| `w311_native.log` | ★ THE NATIVE REFERENCE — no QEMU, no guest, same GPU. 3 sizes + the known-positive. |
| `w311bench.log` | the guest measurement boot: runner + the full w311 grading block incl. the ratio table |
| `run_w311bench_probe.log` | ★ the guest arm's own metrics (`GUEST_BSUM`, `GUEST_B<N>_*`) and the stage/timing ladder |
| `run_w311bench_qemu.log.zst` | the device's own report for that boot (drain, doorbells, semaphores) |
| `run_w311bench_dmesg.log` / `_serial.log` | guest driver output / guest console |
| `run_w311bench_hostdmesg.log` | ⊘ **0 BYTES.** (E1)'s Xid check passes by reading an empty file and says so. The measured Xid=0 is the GUEST-side one. |
| `w311neg.log`, `run_w311neg_*` | the second boot: the negative control as the FIRST AND ONLY context, `GUEST_NEGCTRL_TOTAL_BAD=262144` |
| `w311_analysis.py` | the arithmetic behind §1–§3 of the doc, with both arms' numbers inline |

## The three numbers to read first

- **RATIO** (guest ÷ native GFLOP/s, steady state): `0.00306` / `0.01520` / `0.03602` at
  N = 512 / 1024 / 2048 ⇒ pre-registered outcome **(C)**.
- **`guest = C + k·native`** fits at **C ≈ 115–132 ms fixed per launch** AND **k ≈ 22–27×**
  proportional. ⊘ (D) and (E) are BOTH true; they were pre-registered as alternatives.
- **The copy plane is a flat ~9 MiB/s** = **~420–500 µs per 4 KiB page**, both directions, all
  three sizes, ~800× slower than native.

## ⚠ Two things in here that must not be misread

- ⊘ **The `SEMA-WRITE` 251 ms cadence is the OBSERVER'S clock, not the plane's.** It looks
  exactly like a 250 ms completion tick and 251/2 matches the fitted fixed cost — and it is
  refuted by the guest's own N=512 latencies, which form a continuous 102.9–138.1 ms band.
  See §6 of the doc before quoting it.
- ⊘ **The ~100 ms per-launch floor is MEASURED; its mechanism is UNATTRIBUTED.** It is not
  publication (§5 measures that separately) and not the sync (batching does not remove it).

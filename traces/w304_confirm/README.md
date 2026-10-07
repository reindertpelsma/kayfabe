# `traces/w304_confirm/` — the eight w304 boots, real GA106 (`vh`), 2026-08-14

Every boot ran `scripts/bench/w304_confirm.sh`, which invokes `w297_cup3.sh` →
`w290p_run.sh` → `boot_capture.sh`. Sequence drivers: `w304_seq.sh` (the six confirmations)
and `w304_seq2.sh` (the joint boot + the (E) known-positive).

## Part 1 — the five confirmations, ONE VARIABLE PER BOOT, at n=2

`w298` measured each of these cells ONCE. These are the second measurements. **Every arm is
read from the DEVICE's own emissions**, never from the environment — see `w304_summary.txt`,
rows `ARM (device)` and `ARM (counts)`.

| boot | override | device-side witness | `^CUP3_VAL` |
|---|---|---|---|
| `w304base`     | (none)                        | PB-PIN 1142 · SEMA-PIN 458 · OPERAND-PIN 325 · PT-SWEEP 230 | **43** |
| `w304ptsweep`  | `KAYFABE_PT_SWEEP=off`        | PT-SWEEP 230 → **1**, no `ran=` line                        | **43** |
| `w304opjoin`   | `KAYFABE_OPERAND_JOIN=assert` | `OPERAND-JOIN arm=assert`                                   | **43** |
| `w304gpushbuf` | `KAYFABE_GUEST_PUSHBUF=off`   | PB-PIN 1142 → **0**                                         | **43** |
| `w304gsema`    | `KAYFABE_GUEST_SEMA=off`      | SEMA-PIN 458 → **0**                                        | **43** |
| `w304goperand` | `KAYFABE_GUEST_OPERAND=off`   | OPERAND-PIN 325 → **0**                                     | **43** |

⊘ `PB-PIN` is NOT a witness for `GUEST_OPERAND`. It reads 1142 / 1142 / 458 / 0 / 1142 / 1142
across these six GREEN boots — it tracks the workload. `w304_confirm.sh` first claimed a
`PB-PIN 1142 → 637` witness for that arm, inferred from w298's aggregates; that was wrong and
the file records the correction.

## Part 2 — the joint boot, AFTER the deletion

`w304joint` — nothing set, the five gone **by construction**. `^CUP3_VAL=43`, ladder 8/8,
`Xid=0`, `host_rows=18295 of 18309` (identical to the baseline).
Device witness: `PB-PIN=0 SEMA-PIN=0 OPERAND-PIN=0 PT-SWEEP=0`, and
`OPERAND-JOIN arm=assert` with `OPERAND-JOIN-TABLE=96` — the surviving instrument still runs.
★ `HOST-PUBLISHED` lines went **229 → 230**: the census is unconditional now.

## Part 3 — criterion (E)

`w304_e_selftest.txt` is the full known-positive run: six crafted fixtures (one per clause)
plus the OLD and NEW criteria replayed side by side over all **33 recorded boots** on the
bench. The OLD criterion calls a `^CUP3_VAL=43` a REGRESSION on **2** of them
(`w298ptsweep`, `w304ptsweep`); the NEW one passes every one of the 13 greens.

`w304ekp` is the LIVE known-positive: `KAYFABE_VAS_PUBLISH=assert`, a genuine address-plane
regression. (E1) fires on **Xid=16** and (E2) fires on **`★DRAINED` rows = 0**.
⊘ Note it is a POST-DELETION boot — the publication is still load-bearing with all five pins
gone, so the deletion did not make `VAS_PUBLISH` redundant.

## What is here

- `w304_summary.txt` — one row per boot, appended live; the primary artefact.
- `w304_e_selftest.txt` — criterion (E)'s known-positive run.
- `run_w304*_probe.log` — the guest-side hook output (`^CUP3_VAL`, the stage ladder).
- `run_w304*_hostdmesg.log` — the per-boot host dmesg DELTA. ⚠ **0 bytes is the normal
  green**, not a failed capture; the probe log states the watermark independently.
- `run_w304{base,ptsweep,joint,ekp}_qemu.log.gz` — the device's own emissions for the four
  boots the argument turns on.
- `w304*.log` — the outer harness logs, including each boot's grading block.

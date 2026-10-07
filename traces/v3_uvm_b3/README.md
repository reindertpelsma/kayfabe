# v3-uvm-b3 — the host-only EXTERNAL FAULT SERVICE (EFS) proof

Evidence for `docs/design/V3_UVM_B3_IMPLEMENTATION.md`. Measured 2026-09-30 on a rented vast
KVM-template box (RTX 3060 / GA106, PCI `0000:00:07`), host **open** NVIDIA kernel module
**580.159.04**, Linux 6.8.0-59, CUDA 12.6. No guest, no kayfabe VMM — this is the privileged half
alone, as the owner required before any guest-managed-memory claim.

The build/run scripts are `tools/uvm_efs/box/{provision,build_efs,run_experiment}.sh`; the tests are
`tools/uvm_efs/tests/`; the patch is `tools/uvm_efs/patch/nvidia_uvm_efs_580.159.04.patch`.

## Files

- `run_full.log` — the full matrix (A stock baseline, B patched/default-off, C the EFS cases, D
  coexistence, E latency), one `RESULT`/`### exit` per step.
- `clean_reruns.txt` — the data-check modes re-run at `c94601d9` after fixing a test-only
  verification aliasing (see below), plus the coexistence-during-pressure combination, all
  `RESULT PASS` `DATA bad=0`.
- `dmesg_efs_shutdown.txt` — the kernel's own per-VA-space shutdown accounting
  (`diverted/delivered/replayed/cancelled/timed_out/parked_at_shutdown/hw_cancelled/...`).
- `dmesg_errors.txt` — every `Xid` in the run: exactly 6, all from the six deliberately
  **unserviced** cases (2× stock, cancel, timeout, crash, ctxdestroy); zero on any serviced run.
- `ko_sha.txt` — sha256 of the built `nvidia-uvm.ko` and of the patch. `driver.txt` — the loaded
  module string. Repo head is `c94601d9`.

## Headline results

| case | evidence | verdict |
|---|---|---|
| stock module: unmapped access | `KERNEL_RESULT 700 CUDA_ERROR_ILLEGAL_ADDRESS`, Xid 31 | fails as any host does |
| patched, default off | `efs_auth RESULT PASS`; unmapped access still 700 | stock behaviour, EFS refused |
| opt-in gating | `OPTIN_DISABLED_REFUSED` (module off), `OPTIN_REQUIRES_HMM_OFF` | two-key opt-in enforced |
| negative control (pre-mapped) | `RECORDS serviced=0`, `DATA bad=0`, `RESULT PASS` | **zero faults** |
| real shader fault → deliver → map (cuMemMap) → replay | `service 256`: serviced=256, `DATA bad=0`, `RESULT PASS`, zero Xid | **correct data** |
| same, kayfabe's own RM vidmem via UVM ioctls | `service 8 raw`: serviced=8, `DATA bad=0`, `RESULT PASS` | **correct data** |
| scoped cancel | `cancelled=1`, only that VA space's kernel → 700; neighbours fine | scoped |
| kernel timeout (never answered) | `timed_out=1`, kernel RCs at the ~4 s deadline | bounded |
| VMM crash (SIGKILL, fault parked) | shutdown `parked_at_shutdown=1 hw_cancelled=1`, `nvidia-smi` healthy after | bounded, no wedge |
| context destroy, fault pending | `CTXDESTROY_COMPLETED CUDA_SUCCESS`, bounded by the timeout | bounded |
| foreign-tgid / fabricated-id refusal | `WAIT_FOREIGN_TGID_REFUSED`, `RESOLVE_FABRICATED_STALE` | authorized |
| **native host CUDA coexistence** | `coexist` 203.6 vs 211.9 iters/s idle (96%), `managed_bad=0`, during 5× EFS `service 128` all PASS | **unaffected** |

## Fault-delivery latency (GPU PTIMER-calibrated, `service 256`, quiet box)

`hw_access→packet` p50 1.8 µs · `packet→parked` p50 94.5 µs · `parked→user` p50 36.8 µs ·
**`DELIVERY packet→user` p50 131.2 µs, p99 206.1 µs** · `map (cuMemMap)` p50 143 µs ·
`user→replay` p50 153 µs · `total access→done` p50 288.8 µs, p99 370.7 µs. A lighter 32-page run
measured delivery p50 ≈ 83 µs (`efs_fault service 32`, 2026-09-30, `run_full.log`); under
concurrent coexistence pressure delivery held p50 ≈ 90–117 µs (`clean_reruns.txt`, at `c94601d9`).

⚠ Of the `service 256` figures above, only `hw_access→packet` is in a committed log: `run_full.log`
section E (the `service 256` "quiet box, larger sample" run, 2026-09-30) reads DELIVERY p50 73.7 µs /
p99 98.8 µs and `total access→done` p50 208.9 µs / p99 244.7 µs. The run behind the other figures
is not committed (found 2026-09-30, attributing this paragraph for the claim ledger).

## The test-only verification fix

The first full run (`run_full.log`) shows `RESULT FAIL` `DATA bad=24/248` on `service`: the test
backed N virtual pages with only 8 physical chunks, so pages sharing a chunk overwrote each other's
marker and `bad` counted the collisions. `KERNEL_RESULT 0` and `RECORDS serviced=N unexpected=0`
already proved every fault was delivered, mapped and replayed; the fix (one physical chunk per
store page) makes the data check meaningful and every mode reports `DATA bad=0` in
`clean_reruns.txt`. The latency numbers were never affected (latency does not depend on the data
check).

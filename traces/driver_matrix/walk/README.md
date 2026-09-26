# Driver-matrix hardware walk — the evidence `V3_DRIVER_MATRIX.md` §6 cites

Pulled off the bench boxes after each run (owner rule 2026-09-26: a vast box is untrusted and not
guaranteed to persist, so nothing a claim rests on may live only there). Data only — logs,
summaries, traces; no binary. One directory per box:

| box | vast id | GPU | role |
|---|---|---|---|
| `kfd/` | 52746206 | RTX 3090 (GA102 `0x2204`), host **580.159.04** open | guest walk (queues `q3` … `q8`) |
| `kfh/` | 52788835 | RTX 3080 Ti (GA102), host driver swapped per version | host walk (`q4h`, `hostwalk`, `hostwalk2`) + mixed pairs |

Per box:

- `summary/` — the queue logs (`q*.log`, `hostwalk*.log`: one line per step, every row names its
  revision), every thin-suite summary (`*_suite.out`, one `FAST_CELL_ARM` line per arm) and every
  fat-guest ladder summary (`cl_*_guest.out`, one `CL_ROW` per rung, with the doorbell ledger).
  `kfd/summary/host_kernel_1930_1940.log`: the host GPU's GFW-boot failure (the wedge an FLR cleared).
- `gates/` — `scripts/bench/v3_gates.sh` logs per revision (and per host driver on `kfh`).
- `tests/` — the `cargo test` result lines per revision.
- `fp.tar.xz` — `failure_point.sh` outputs and each point's QEMU + guest-console logs.
- `red_arms.tar.xz` — the QEMU + guest-console logs of every thin-suite arm that did not pass.
- `ladder_boots.tar.xz` — every ladder boot's `run_cl_*` evidence (`qemu`, `dmesg`, `probe`,
  `hostdmesg`), e.g. `run_cl_r6de22590_fat5755708_cup2_1_qemu.log`: the 575 guest's `cuInit`
  wall (`GSP rpc Other(54)` → `UNSERVICED { code: 54 }`).
- `nvdiff.tar.xz` — bare-metal `cup2` under the nvdiff shim on the host driver of the time.

Refreshed after each run; a file here is the box's copy at the time of the commit that added it.

# v3-uvm-research evidence (2026-09-26)

Inputs and results for `docs/design/V3_UVM_DEMAND_PAGING.md`.

- `um_probe.cu` — copied verbatim from `traces/v3_appfix/um_probe.cu` (branch `v3-appfix`).
- `readmostly_probe.cu` — does kayfabe honour the guest UVM's read duplication (§3 of the doc).
- `bm_run.sh` — bare-metal baseline: every probe + clpeak, bracketed by the host UVM's
  `fault_stats` (needs `nvidia-uvm uvm_enable_debug_procfs=1`), with HMM on and off.
- `holder.cu` — keeps the GPU UVM-registered so `fault_stats` exists during the run.
- `bm_rtx3060ti_580.159.04_hmm{1,0}.out` — bare-metal results (vast 52726826, destroyed), see doc §1.2.

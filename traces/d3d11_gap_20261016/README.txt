d3d11-gap campaign (2026-10-16/17). SCRATCH. NEVER MERGEABLE: the build carries the scan-off experiment modes 1/2/3, counters, env switches and a changed private-ring layout.
Read REPORT.txt first (per-frame budget table at the top), then PREREG.md (predictions and falsifiers, written before the runs), mutation_table.txt, tools_out/ (run tables, per-frame tables, ladder, gate, hang correlation, SET_OBJECT summary).
Raw evidence per run: heaven/<label>/ (assert.txt, env_pre/post, vmstat, dmon, thread CPU, guest logs, prof/ summaries), arm/<tag>.* (memory records, display-line log with wall-clock stamps), chain_*.log (start/exit markers).
On the host (not in git): qemu logs gz ev/qemulog/, perf.data under ev/heaven/*/prof/, binaries fp/kf3-bins/<rev>, source snapshot src/, all under /data/claude-gap/.
Liftable (production quality, no environment, no clock): crates/kf-chan/src/user_lazy.rs and crates/kf-chan/tests/user_lazy_model.rs (commit 214a1ecd2).
Scripts: scripts/bench/d3d11_gap/ (harness copy of the perf campaign with chain3/5/6/7, perfall.sh, profsum.sh, analysis scripts; lists/ has every arm list incl. the ones NOT run).

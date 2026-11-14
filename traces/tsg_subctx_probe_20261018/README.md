# tsg_subctx_probe_20261018

STATUS: RESEARCH, 2026-10-18. Evidence for `docs/design/V3_TSG_SUBCONTEXT_FEASIBILITY_20261018.md`.

`summaries_all_batches.txt` = `scripts/bench/tsg_subctx_probe/sum.sh` run over raw batches b1..b9 (72 runs, one fresh unprivileged RM client each,
uid 1000, host RTX 4070, driver 595.91.07, owner VM stopped during the runs). Per run: the commands issued, every RM status that matters, engine-work
results, `[meta]` line with rc, probe pid, Xid delta and Xid lines for the pid (all 0), and any NVRM journal line (GSP refusals, asserts).
Raw logs (full RM call lists, pushbuffer words, journal windows): host `/data/claude-tsgprobe-20261018/raw/b1..b9` (1.1 MB, not in git).
Batch notes: b1 = harness defect (work-submit token asked before bind, baseline runs 01/02 MISS; b2 repeats them); b3/b4 ran `scripts_v1/`
(ASYNC channels at runqueue 0: the 0x31 refusals); b5 = runqueue-1 runs; b6 = latency + D3D12/Vulkan shapes; b7 = re-run of the shape scripts with runqueue 1
on ASYNC channels; b8 = free order, CUDA-8, timeslice; b9 = control scope (channel schedule, DISABLE_CHANNELS, TSG schedule(0), PREEMPT).

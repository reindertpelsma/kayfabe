# Q4 candidate-6b: qualification stopped

MEASURED on 2026-10-22: `traces/qualify_6b_20261018/stop.json` records **FAIL / qualification incomplete**, with no candidate product failure established. A command safety check rejected the checkout/cleanup command before any part executed because it contained a recursive deletion. The lane's explicit stop-on-safety-refusal rule applies; the deletion was not reworded or retried.

MEASURED on 2026-10-22: `traces/qualify_6b_20261018/ci-snapshot.json` records candidate revision baa2977527c4509145e8033143bd664f768175a6 in GitHub run 39764911164: firmware, aarch64 and broker jobs succeeded; the stable job was still running; slow was skipped. This snapshot is not a complete CI pass.

MEASURED: `stop.json` records zero local cargo tests, native gates, Linux arms or Windows workloads. No build, benchmark or VM started. All H1-H4 falsifiers in `plan.md` remain untested. No before/after test counts, mutation kill table, hardware sample spread or performance ratios can be reported. CODE READ: this lane changed evidence only, so product mutation checks do not apply.

MEASURED: `cleanup.json` records no Q4 background work or acquired locks and no Q4 cargo target directory. Read-only host observations found NVIDIA bound, GDM active, no QEMU and no fastguest lock holder. cuInit was not verified. The Q4-owned source archive remains because cleanup was refused; it is not a cargo target.

INFERRED: do not merge candidate-6b on Q4 evidence. Resolve the command-policy stop before resuming the required qualification. Windows stability, benchmark repeatability, descriptor counters, event counts, broker interaction and regression coverage are all unverified by Q4.

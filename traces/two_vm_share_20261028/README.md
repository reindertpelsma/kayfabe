# Z1 two Linux VMs on one GPU

STATUS: UNRUN, 2026-10-28. H1 remains untested.

CODE READ, branch `codex/sw3-z1-20261028`: [two_vm_share.sh](../../scripts/bench/two_vm_share.sh) prepares three solo CUDA measurements, three concurrent pairs and three concurrent short raw subsets on one explicitly selected GPU, with separate QEMU processes/logs/private inner locks under the shared host lock. The fixed workload is cup3 plus1024x1024 matmul,4000 verified iterations; the requested180-second oracle is unchanged. Actual loop-window overlap, per-VM correctness and throughput distributions are required. [Preregistration](PREREG.md).

MEASURED 2026-10-27, run `Z1-d9cfef0f`: the native oracle completed cup3=43 and the final4000-iteration matmul with zero mismatches. However, the earlier rootless process-inspection/host-observer refusal stopped the campaign before this two-VM phase; [rootless closeout](../rootless_20261028/README.md), [status](result.json). No solo guest or paired CUDA measurement was made. No throughput ratios, n>=3 result, fairness, hostile-kernel latency or isolation result is available. Windows guests were out of scope.

CODE READ, historical records: [V3_MULTI_GPU_AUDIT](../../docs/design/V3_MULTI_GPU_AUDIT.md) records two separate QEMUs and CE clients on one GPU at an older revision. [Run54](../windows_code43_walls_20261007/README.md) records Linux/Windows boot-level co-residence for200seconds at Windows revisiona88764b3 beside Linux4bc62999. Neither is the newly requested concurrent CUDA correctness/throughput experiment at current next. The owner's earlier belief is preserved as the hypothesis, not upgraded to a Z1 result.

Decision: keep the prepared experiment for a controller-cleared follow-up after the rootless observer STOP is addressed. Z1 launched no further guest or rental after the live host-observer STOP.

# M1 correctness gates preregistration

STATUS: LIVE, 2026-10-19 00:10 UTC. Lane M1. No hardware gate has run in this lane.

## Hypotheses and falsifiers (written before runs)

1. INFERRED hypothesis: an existing Linux disk contains enough stock userspace for the fat-guest gates. FALSIFIER: offline inspection finds missing gcc, libcuda, stock NVIDIA kernel modules, graphics workloads/dependencies, or video tooling. A disk filename or an old successful boot is insufficient evidence.
2. INFERRED hypothesis: mirror ON preserves CUDA results, graphics hashes, video frame/bitstream checks, and broker/interactive functionality relative to candidate-5 in the same session. FALSIFIER: any new host/guest Xid (including 69/31/13/43), hang, channel refusal, wrong result, missing completion, or regression against the paired base arm. An unexecuted prerequisite makes a gate UNRUN, never GREEN.
3. INFERRED hypothesis: mirror OFF equals candidate-5 for one complete gate. FALSIFIER: a result-set mismatch, different refusal, hang, or Xid in the OFF arm absent from base.
4. INFERRED hypothesis: CUDA multi-stream/context coverage reaches the explicit TSG path and the existing many-channel/cap tests enforce the declared boundary. FALSIFIER: logs show only legacy TSGs, fewer than the required channels, a missing named cap refusal, or wrong output. Do not claim coverage from workload names.

## Inputs and execution

CODE READ: start at author branch c2685412e, code/binary 0d7ba74389; expected mirror MD5 48ab1b70c2cf2c26d2c1287bf7311a9a. Base is candidate-5 f8f9d1a5f (not production); production 0452613b is only for restoring the owner's VM. Resolve and hash both binaries before use. No product behavior change is planned.

CODE READ: documented CPU/disk provisioning is stock QEMU + slirp in provision_bench_tree.sh, then provision_guest_gfx.sh and provision_guest_display.sh; llm_parity.sh requires its separately provisioned venv/model. cuda_ladder.sh uses four rungs, one boot each; gfx_suite.sh compares five steps and at least ten hashes against bare metal; video_lane.sh requires identical ffmpeg bytes in both arms and checks NVDEC against software decoding. dbfast_lane.sh is a doorbell A/B harness, not a substitute correctness gate.

MEASURED: at lane start, host main filesystem has 97 GiB free; dev root has 53 GiB free. Owner run 3009 reports READY. Existing Linux image chains were located and will be inspected read-only; no suitable gate receipt has yet been established.

CODE READ constraint: common rules require the shared lock for every VM boot, and the provisioning scripts refuse a concurrent QEMU. Part A therefore permits offline disk inspection/preparation while the owner runs; do not evade either guard to boot a provisioning VM. If a new provisioning boot is necessary before Part A can be ready, STOP and record the contradictory requirements and options.

Hardware order, after Part A is ready: gracefully stop owner with the authorized flag and verify its EXIT plus QEMU exit; bounded acquisition of the shared GPU lock; release between stages. Bare-metal controls first. CUDA base then mirror; graphics mirror then base; video base then mirror; extras mirror then base, plus mirror-OFF CUDA stage. Reuse one M1-owned overlay or identical fresh children of one frozen M1 seed. Print exact binary revision, hash, flags (including default/off measurements), command, n, start/exit, and result per arm.

Capture full dmesg and nvidia-smi before/after each stage into lane-owned host files, plus journal timestamps to avoid the known dmesg watermark/ring-wrap error. Preserve a small public summary with Xid delta, refusing to infer zero from unreadable logs. Extract guest dmesg after workloads and explicit-mode birth/counter lines. Compare content results 1:1. Reproduce any failure once, then mirror ON/OFF; stop at an unexplained failure. Any product fix requires an isolated EXPECTED RED commit, fix commit, and each mutant killed by at least two tests.

Hard stop: 04:30 CEST (02:30 UTC). Write lane DONE with GREEN/NOT GREEN and every unpassed gate, release the lock, and restore production with the untouched launcher unless P1 owns the lock. No other lane messages or writes. Never write the owner's disk. Optional Windows work only after Linux gates and within the deadline.

Every stage: secret-scan the complete staged diff, commit using configured identity and the M1 trailers, push only codex/tsg-mirror-gates-20261019, and verify remote head equals local head. Final public report and private owner report list every gate including UNRUN, test counts, mutation applicability, evidence, decisions, and cleanup.

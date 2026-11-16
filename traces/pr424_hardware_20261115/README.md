# PR #424 hardware validation on one held Vast box, 2026-11-15

**STATUS: LIVE, 2026-11-15.** MEASURED on one box, local evidence only (no Worker, no run token, no attested setup cache; OWNER_RULINGS section AR item 4). Not a release record.
The report is `REPORT.md` here (copy of the coordinator note `PR424_HARDWARE_20261115.md`); the rest of this directory is the evidence.

- Tree under test: PR #424 head `9ddb897e7281ecff4baa9d35acb863bbd265644a` (branch `claude/va-apply-bounds-20261115`, base `origin/next` 84dff90b6; the head contains the
  unmap-call cap, the address cut, per-space progress, the 64-window budget, the exact `kf_cut_point` and the rebuilt PTX). Cloned on the box from public GitHub with the
  sparse list of `ci/vast-broker/src/onstart.js` (`clone.sh`); `git rev-parse HEAD` on the box printed the same sha.
- Instruction: `instruction.json`, sha256 `63ab6fbeb11f83d43c1e8af991ae774dcdd3c5680727a7cf4f8f4e1bc2cc6c34` (`--emit-instruction` with
  `KF_MERGE_WINDOWS=1 KF_TIER=large KF_WINDOWS_RUNS=1 KF_RELEASE_REHEARSAL=1`; no apps, no SPEC). Same hash in `kf-result.json`.
- Command (`launch.sh`, as root): `python3 -I scripts/ci/vast_payload.py --instruction /workspace/instruction.json --only provision --only cuda_host --only merge_bar --only cuda_guest --only windows_gate <sha> <nonce>`.
  `cuda_host` (13 s, the bare-metal reference that makes `host_reference` GREEN) was added to the four requested stages. The wrapper `vast_payload.sh` was not used (it cannot take `--only`
  and needs the Worker's blob helper), so apt and cargo went online.
- Box: Vast 60811732, label `kf-pr424-20261115`, RTX 4090 (AD102), AMD EPYC 7V73X, 30 vCPU, 98 GB RAM, disk 194 GB (186 free), `/dev/kvm` present, KVM image, driver 575.51.03 as
  rented and 580.159.04 after provisioning, 0.959 USD/h. Two earlier rentals (60809802, 60811069, a Denmark host whose forwarded ssh port never answered) were destroyed unused.
- Kernel log: `dmesg -w --time-format iso >> kernel.log` from before the payload; `stage_kernel_offsets.tsv` maps stage start/end to kernel.log byte and line offsets and the Xid count.
- On-GPU self test: `kf_tests/` (`run_kf_tests.sh`, `ptx_jit.py`). The box had no CUDA toolkit; `cuda-nvcc-12-2` and `cuda-cudart-dev-12-2` (12.2.140-1) were installed from NVIDIA's apt repo.

Files: `kf-result.json` (verbatim), `payload_stdout.log` (console incl. `KF_FINAL_SUMMARY`), `kf-ci-output.log.gz`, `payload_start.marker`, `payload_exit.line`, `kernel.log`,
`stage_kernel_offsets.tsv`, `evidence/` (stage `*.run`, `native-receipt.json`, `arm-diagnostics.json`), `prov/` (stage logs), `bench/` (xid-watch logs, suite outputs),
`winvm/` (Windows gate: both QEMU logs gzipped, run log, checks, host dmesg), `collect_final/` (`collect.py` output: `pattern_counts.tsv`, `summary.json`, `lines_*.txt`),
`collect_mid/` (same, taken during the Windows install), `kf_tests/`, `teardown_verify.txt`, `box_hw.txt`.

Limits of `collect.py`: it scanned the 188 `*.log` files under `/workspace/bench`, `/root/prov`, `/workspace` and the Windows gate logs on the box. The per-arm Linux QEMU logs were scanned but
not copied (only files with a hit would be listed in `pattern_counts.tsv`; `lines_*.txt` carry every hit). `walk report CUT` is logged only for the first 32 windows per process and
`SLOW host unmap call` for the first 16, so a count is a lower bound; the windows-per-request maximum is not logged at all (only the budget failure is named).

Scrubbed before commit: GPU UUIDs became `GPU-fa4efa4e-...` placeholders (a different last group per board; the guest's virtual UUID likewise), the VM identity serial and its MAC were replaced.
No public IP, token or key was in the copied files. Only text files were copied back. `scripts/ci/no_gpu_uuids.py` and `scripts/ci/no_private_refs.py` pass.

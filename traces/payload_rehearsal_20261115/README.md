# Release-gate payload rehearsal on one held Vast box, 2026-11-15

**STATUS: LIVE, 2026-11-15.** MEASURED. The payload itself (`scripts/ci/vast_payload.py --instruction FILE`) ran end to end on one rented box
in the REHEARSAL scope (large tier, one die, no driver sub-run matrix, no SPECviewperf, no llm_parity). It is **local evidence only**: no Worker,
no run token, no attested setup cache (OWNER_RULINGS section AR, item 4). It is not a release record.

- Tree: `origin/next` = `84dff90b6a4b088f2817e5d32c0b5136303148d6` (merge of #416, the first run of `--instruction/--only/--dry-run` on a box).
  Checkout `/workspace/kf-ci-src`, sparse list copied from `ci/vast-broker/src/onstart.js`, cloned from GitHub (public repo; no credentials on the box).
- Instruction: `instruction.json`, sha256 `bbe559444a6c7bf1cbf41489b169c70d34ee8b413bafeab214a1273d070537fb` (made with `--emit-instruction` from the
  env `KF_MERGE_WINDOWS=1 KF_TIER=large KF_WINDOWS_RUNS=2 KF_LINUX_APPS=1 KF_WINAPPS=mid KF_RELEASE_REHEARSAL=1`). The same hash is in `kf-result.json`.
- Box: Vast instance 60780372, label `kf-payload-rehearsal-20261115`, RTX 4090 (AD102), AMD Ryzen 9 5900X, 23 vCPU, 49 GB RAM, 194 GB disk, `/dev/kvm` present,
  image `docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16`, 0.496 USD/h. Created 11:35:03Z, payload 11:40:24Z to 15:41:05Z (4 h 01 min), destroyed 15:45:34Z
  (4.17 h, about 2.1 USD from the offer rate, not an invoice). Host driver 575.51.03 as rented, 580.159.04 after provisioning.
- Command (as root): `KF_SOFT_DEADLINE_MS=<now+300min> python3 -I scripts/ci/vast_payload.py --instruction /workspace/instruction.json <sha40> <nonce32>`
  (`launch.sh`). The shell wrapper `vast_payload.sh` was NOT used: it takes exactly two arguments and its first step needs the Worker-only
  `/usr/local/bin/kf-box-blob-url` (see the report). So `KF_APT_CACHE_ROOT` and `CARGO_NET_OFFLINE` were not set: apt and cargo went online.
- Kernel log: `dmesg -w --time-format iso >> kernel.log` from before the payload (harness fix #420 not merged); `kernel_offsets.log` samples size and Xid count
  every 30 s; `stage_kernel_offsets.tsv` maps stage starts to offsets.

Files: `kf-result.json` (verbatim), `payload_stdout.log` (full console, including `KF_FINAL_SUMMARY`), `kf-ci-output.log.gz` (the payload's own capped 983040-byte
arm-diagnostics log, not the console), `payload_start.marker`, `payload_exit.line`, `kernel.log`, `evidence/` (`*.run`, `arm-diagnostics.json`, `native-receipt.json`,
`native-controls/`), `prov/` (stage logs and per-gate directories), `bench/` (ladder rows, suite output, xid-watch logs with an Xid), `box_hw.txt`,
`prep_*.txt` / `box_dryrun_checks.txt` (the `--dry-run` checks), `teardown_verify.txt`.

Scrubbed before commit: GPU UUIDs became `GPU-fa4efa4e-...` placeholders (a different last group per board), MAC addresses other than the QEMU default prefix became
`02:00:00:00:00:NN`. No public IP, token or key was in the copied files; only text files were copied back (no images, no tarballs).

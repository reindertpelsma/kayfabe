# Linux apps media on a rented box: build, provisioning and matrix timings, 2026-11-10

STATUS: LIVE, 2026-11-10 (evidence of one box agent's run; the files are scrubbed copies of its logs). One Vast rental, RTX 4090 (AD102), 30 vCPU,
49 GB RAM, 340 GB disk, Romania; rented 16:28Z, destroyed after the evidence was pulled. Revision under test: `0e90b617e`, the head of PR #335
(`claude/apps-proof-20261110`, no new commits). The run was driven by hand with the payload's environment (`BASH_ENV` hooks and `KF_PAYLOAD_*`), under
`host_xid_watch.sh`; it did not go through the Worker. MEASURED means a line in a file of this directory; the 13.5 min split between rebuild and
guest image is INFERRED (the log does not separate them).

| File | What it is |
|---|---|
| `bundle_build.log` | `build_bundle.sh` by hand, 16:48:04Z to 17:02:02Z (14 min, CUDA toolkit install included): every `BUILD_*=ok`; `LLAMA_REV=4b1a27fa0`; the shallow checkouts of cuda-samples and clpeak print their commits (`9c688d7ff784...`, `a4d617d7d054...`) |
| `apps_provision.log` | the stage's own provisioning pass (`KF_APPS_PROVISION=1`) over those sources, then the guest image: `SETUP_*=ok`, torch 2.6.0+cu124, `GUEST_APPS_DONE` at 18:38:04Z |
| `apps_stage.out` | the stage output under `host_xid_watch.sh`: `APPS_PROVISION_START` 18:24:37Z, `APPS_PROVISION_END rc=0` 18:38:04Z (13.5 min), `APPS_STAGE_MATRIX_END rc=0 secs=1029`, 66 `APP_ROW ... result=PASS` |
| `apps_matrix.log` | the matrix driver: 66 of 66 batched, no isolated re-run needed |

Measured from these files: bundle 14 min, provisioning pass 13.5 min, matrix 1029 s (17 min) for 66/66, host Xid 0 (`xid_before=0` in `apps_stage.out`).
Slowest apps (`APP_ROW secs=`): `vkpeak` 332 s, `clpeak` 112 s, `gpu_burn` 95 s, `torch_correct` 53 s. Provisioning here ran with the toolkit and sources already
present, so it is not a fresh-box number; the fresh-box number is the 2026-11-07 record, [29 min](../apps_linux_20261107/README.md).

Not shown by these files: that the build works under the payload's cache-only `apt-get` shim (it was run by hand, so the shim did not apply), and what the
Windows stages of the same box did (they are not part of this directory). The revision's `build_bundle.sh` cloned gpu-burn at HEAD and printed no revision.

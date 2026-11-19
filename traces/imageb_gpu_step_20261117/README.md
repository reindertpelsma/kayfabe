STATUS: LIVE, 2026-11-17 (measured on ONE Vast RTX 4090 box; text evidence only; UUIDs, IPs and presign parameters stripped by `redact` and a second scrub pass).
Labels: MEASURED = in a file of this directory; INFERRED = my reading. `expected` = the seed-derived GPU UUID, `observed` = what the guest's `nvidia-smi` printed, `host` = the box GPU's own UUID; no value of any of them is in this directory.

# Image B `gpu` step: three boots with the new diagnostics (plus one extra boot)

Question (`/root/night/GPU_UUID_MISMATCH_20261116.md`, section 7): why did the stage fail with "Post-fix seed GPU UUID mismatch" on 3 of 3 fresh stage boots of image B, while the guest had returned NO UUID for 80+ s?

## Run

- Box: Vast instance 61031540 (label `kf-gpustep-20261117`, offer 52153651, RTX 4090 24564 MiB, AMD EPYC 7502, host driver 580.159.04, host kernel 6.8.0-59; `host_facts.txt`). Rented 2026-11-17 06:46Z, destroyed 08:25Z and verified absent from `vastai show instances-v1`. Cost ESTIMATED 1.2 USD (0.551 USD/h x 1.65 h + about 70 GB at 0.004 USD/GB; not read from an invoice).
- Code: PR branch `claude/imageb-gpu-diagnostics-20261117`. kf3 built on the box by the repo's own provisioning (`vast_payload.py --only provision`, `payload_stdout_P.log`) at code commit `28ccb618a252899d0b8951c363c1ca526e6a7084` (`kf3-bins/28ccb618`). That commit is PR #446 head `50d8c2fac` plus the diagnostics (poll loop, gate on the device being started, log tail kept, honest detail).
- Image B: downloaded by the BOX from the private R2 object through a 12 h presigned URL minted on the coordinator with the read-only remote (URL deleted from the box after the download; never logged), 62,506,351,001 B in about 6 min; `sha_image.txt` shows sha256 `fe75afa1...cc154a`, equal to the pin.
- Boots (all `--fb-mb 8192`, app list `gpudiag` only, offline overlay):

| boot | path | what came before | boot started | gpu step | wait before restart | restart at | first answer, after the restart |
|---|---|---|---|---|---|---|---|
| d1 | stage (`propimage_apps_b_stage.py run`, its own clone path), code 28ccb618 | the stage's own unpack of the 59 GB image (07:05:35 to 07:09:17) right before | 07:09:17Z | PASS, 241 s | 135 s | 165 s | `expected`, 76 s after |
| d2 | dev (`propimage_check.py`), code 28ccb618 | `unpack_b.py` of the 59 GB image (07:26:35 to 07:30:11) right before | 07:30:11Z | PASS, 239 s | 133 s | 164 s | `expected`, 74 s after |
| d3 | dev, code 28ccb618 | `sync` (0 s: the page cache held 4 kB dirty, the unpack was 17 min old) and a 300 s rest | 07:52:26Z | PASS, 238 s | 134 s | 164 s | `expected`, 74 s after |
| d4 (extra, not asked) | dev, code `612978cb` (the gate fixed, see below) | d3 had just ended | 08:09:30Z | PASS, 121 s | 15 s | 40 s | `expected`, 80 s after |

Per-boot files: d1 `kf-propimage-b/out/boot1/` and `kf-propimage-b/logs/boot1_qemu_kf3.txt`, d2/d3/d4 `hold_out_d{2,3,4}/` and `collect/d{2,3,4}_qemu_kf3_tail.txt`; each `gpu_diag.json` has the case, the PnP state before and after the restart, the timings and the poll records. `timings.txt`, `stage_stdout_d1.log`, `hold_run_d*.log` and `host_kernel_kf3_lines.txt` (host `dmesg`: kf3, NVRM, Xid, vamgr lines) hold the clock times.

## What each boot shows

All four boots PASSED: the case was `expected` (one GPU line, equal to the seed's UUID, return code 0) with PnP problem code 0 at the end, and the `gpu` row reads "observed UUID" facts from the result instead of a fixed string. No `no_uuid`, `error_text`, `multi`, `wrong_value`, `timeout` case occurred after the restart, so `gpu_events.txt` (the failure-only guest events: nvlddmkm, Kernel-PnP, `setupapi.dev.log`) and the failure-only log tail were NOT produced on this box; they are tested GPU-free only (`scripts/ci/test_gpu_diag.py`).

MEASURED 2026-11-17 in all four boots of this run on the Vast box (`gpu_diag.json` `waited`, `pnp_before_restart`), the same each time:

1. **Before the repair the new die's device is in Code 43, not installing.** Two VEN_10DE display entries: one phantom entry (not present, Status Unknown; INFERRED: the image's own die) and the present device with Status Error, problem code 43, service `nvlddmkm`. `nvidia-smi` printed its "couldn't communicate with the NVIDIA driver" text (case `error_text`) on every query of the wait. So the wait for "Status OK" that I implemented from the analysis' patch A can never succeed before the repair: d1 to d3 burned the whole bound (133 to 135 s) for nothing. Commit `612978cb` changes the gate to "settled" (bound to the NVIDIA service, problem code 0 or 43); d4 shows it working (restart after 40 s instead of 165 s).
2. **The first `nvidia-smi` after the restart blocks about 60 s.** A normal query call takes 13 to 18 s; the first one after the restart returned the expected value only after 74 to 80 s. INFERRED: this is the guest driver's re-initialisation under kf3. It also explains why the old single 120 s call (restart, 15 s sleep, then the first `nvidia-smi`) had little room. The new query limit was 90 s (too close to 80 s); the final commit raises it to 150 s and records per-query `dur_s` and `smi_s` (not measured on hardware).
3. **The kf3 log did not show a restart-window problem.** The stage's census for d1 counts refused and unserviced RM calls over the whole boot (`propimage_b_result_d1.json`: refused 176, unserviced 346, Xid 0) in a boot that passed, so those counts alone are not a failure signature. The `kf3-vamgr` "freeing invalid memtype" host line appears at the restart in d1, d2, d3 (07:18:20, 07:39:25, 08:01:30) and does NOT appear at the restart in d4, which also passed: its absence is not a failure signal (it weakens the reading in section 4.3 of the analysis).

## What differed from the failing runs, and what this does and does not settle

- Failing runs (analysis, section 2): kf3 `d54cda1a`, the old step (restart, 15 s sleep, poll inside one 120 s call), stage boots 3 of 3. Here: kf3 `28ccb618`, the polling moved to the host, stage boot 1 of 1 and dev boots 3 of 3, all PASS.
- Disk state: ruled out as the cause at this revision. d1 and d2 booted straight after a 59 GB write, d3 after a rest; the result and the timings are the same (the I/O-timing arm of the analysis is not supported).
- Restart timing: d1 to d3 restarted 164 to 165 s after the step began (late), d4 after 40 s (`power.json` is written right before the step); the old runs restarted within about 15 s of `power.json` (the repair was the first thing the script did). An even earlier restart is therefore NOT excluded as a factor (the first-boot race stays open), but a restart 40 s after `power.json` passed.
- kf3 revision: INFERRED candidate only. PR #438 (the Windows support profile, kf3 ABI 33) is between `d54cda1a` and `28ccb618` (`git log d54cda1ab..28ccb618a -- crates qemu`). The original failure was not reproduced, so this run cannot say that it fixed it.
- Conclusion: the diagnostics did not identify the cause of the original failure because the failure did not occur in 4 of 4 boots. They did measure the pre-restart state (Code 43), the blocked first query (about 60 s) and one defect of my own gate. If the failure returns, the new step names its case, the PnP code, the nvlddmkm, Kernel-PnP and setupapi text and the kf3 log tail in `gpu_diag.json`, `gpu_events.txt` and `qemu_kf3_tail.txt`.

Not measured here: the VFIO control, other dies, the default 4096 MiB framebuffer (all four boots used 8192 as the stage does today), any boot with more than the `gpudiag` row.

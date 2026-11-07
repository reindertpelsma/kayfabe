# Windows apps on a Vast box: first hardware run, 2026-11-07 (box 59505074, RTX 4090)

**STATUS: LIVE, 2026-11-07.** Measured on one rented box (Vast instance 59505074, label `kf-winapps-20261107`, RTX 4090 /
AD102, Israel, host driver 595.91.07, destroyed 2026-11-07 about 05:55Z, lease 03:12Z to 05:55Z, about 1.75 USD at 0.644 USD/h
(estimate from the offer's `dph_total`, not an invoice)). kf3 revision `b11655b7` (`next` at the time, the checkout the box cloned and
built). The harness files of branch `claude/winapps-20261107-1` (`sshguest.py`, `WinVmVm`, the `win_vm.sh` opt-ins, `windows_apps.sh`,
`grade.py`) were copied onto the box after the gate had passed, so the box checkout was `b11655b7` plus those files (the log's
`rev=b11655b7` is the kf3/product revision). Nothing from this run is a claim about another die, driver or revision.

## What ran (in order)

| step | what | result |
|---|---|---|
| media | `win_vm.sh fetch` on the box; the virtio-win ISO from fedorapeople ran at 267 KB/s (53 min estimate), so it was fetched on a workstation and copied in (sha256 equals the pin in `win_vm.sh`) | media complete 03:30Z |
| install | `win_vm.sh install` (SMP 16, cache=unsafe, as `windows_lane.sh` does) | 16 min (03:30:48Z to 03:47:00Z) |
| gate | `windows_gate.sh wa1` | **`WINDOWS_GATE_PASS rev=b11655b7 gpu="NVIDIA GeForce RTX 4090" ... hostref=green/green secs=1012`** (`gate/`) |
| apps, small | `windows_apps.sh wa1`, profile `small`, 8 apps | **8/8 PASS**, 12 minutes including one guest boot (`small/`) |
| apps, full | `windows_apps.sh wa1f`, profile `full` (79 open apps, shortest timeout first), per-guest 16, budget 5000 s | **58 PASS, 9 FAIL/TIMEOUT, 12 NOT RUN**; stage verdict `WINAPPS_FAIL diag=KAYFABE`, hostref green/green, host Xid delta 0, 0 guest TDR, 0 WER (`full/`) |

The app disk was NOT the future R2 bundle: it was built on a workstation by `appdisk.py build` from the repository's manifest
with the open packages PLUS GravityMark, FurMark and GpuTest (22 packages, 7.0 GB, image sha256 `99cee299...29219`), copied to the box
by rsync (the box downloaded no app, no model and no wheel). The ffmpeg package could not be fetched (pinned URL 404), so the 10
video apps were not part of this disk. The 79 apps of the `full` profile need only packages that ARE in the proposed open bundle.

## Per-app table (full run `wa1f`; small run `wa1`)

`RESULTS_TABLE.md` (79 rows). The batched verdict is the only one: the driver was stopped (SIGTERM at 05:52Z, driver rc 143) to
respect the lease limit while its fifth guest was still working, so phase 2 (re-run of every non-PASS app alone in a fresh guest) did
NOT run, and the 12 apps that had not run are `NOT_RUN`, not failures.

| app | verdict | what the log says | attribution |
|---|---|---|---|
| `overlay_yuv_probe` (tier 1) and `overlay_yuv_{hide,occlude,resize,move,recreate,fullscreen}` (tier 2) | FAIL, 4-6 s | `CheckOverlaySupport reports no DXGI_OVERLAY_SUPPORT_FLAG_DIRECT for the chosen format` (`result=SETUP`, rc 5) | UNATTRIBUTED: no same-box native reference ran; the 2026-10-11 kayfabe campaign did not list these as failing (different flags, display and box) |
| `nbody_demo_gl` | FAIL, 30 s | survived 25 s, `no-gpu-proof`: the GPU-engine counters of its process tree showed nothing | UNATTRIBUTED: the 2026-10-11 campaign recorded a TIMEOUT for it; a counter-sampling fault of the harness is not excluded |
| `managed_t` | TIMEOUT, 181 s | prints `MANAGED_STAGE allocating` and never ends | the same app was TIMEOUT in the 2026-10-11 campaign (`traces/windows_matrix_20261011/`): a known open defect |
| not run: `overlay_yuv_soak, torch_correct (PASS in the small run), cdp_quicksort, hf_generate, llama_cuda_gen, llama_cuda_bench, llama_vulkan_bench, vkpeak, blender_cycles, torch_ai_bench, clpeak_cuda, clpeak_ocl` | NOT_RUN | the slowest apps are last by design | harness stop, not a verdict on the guest |

## Falsifiers (written before the runs) and results

1. *The gate's overlay is a usable parent: a boot of a child overlay of it reaches the NVIDIA adapter at code 0 with nvidia-smi
   working.* Falsifier: the setup step reports no NVIDIA adapter or a problem code. **Not falsified**: every one of the 5 guests of the
   full run and the guest of the small run ended `setup ok: ... nvidia_adapters=1 NVIDIA GeForce RTX 4090, 580.88` and `READY ... smi_ok=True`
   (`full/wa1f_winapps/run/session.log`).
2. *The ssh transport carries the QGA-shaped driver unchanged.* Falsifier: `NOTRUN`/`BOOT_FAIL` rows from staging or launch. **Not falsified**:
   0 NOTRUN, 0 BOOT_FAIL in both runs.
3. *The stage never turns a harness fault into a pass.* **Held**: the stopped driver (rc 143) and the 12 missing rows could not
   produce a pass; the verdict was FAIL because of guest failures, and the grade line lists the NOT_RUN apps separately.
4. *The same-box host reference stays green around the run.* **Held**: before and after both `green cup8=PASS`, Xid count 0/0.

## Measured costs of the harness (the budget problem)

- A guest takes about 3.5 minutes from `win_vm.sh run` to `READY` (ssh up 83-95 s, 26 scripts pushed by scp about 60 s, setup 12 s).
- Sum of the apps' own runtimes in the full run: 900 s. Wall clock for the 67 apps that ran: about 94 minutes. The rest is harness overhead:
  about 47 s per app even for a trivial one (two scp writes, a launch task, polling, cleanup and a health check, each an ssh round trip)
  and 5 guest boots. At this rate 79 apps need about 2 h before the slowest apps (clpeak 1500 s timeout each, LLM, Blender). This is the main
  thing to fix before the release tier (section 9 of the design document).

## Files

`gate/` (the gate's log and expanded evidence), `small/` and `full/` (run directories of `winapps.py`: `session.log`, `win.res`,
`apps/<id>.{json,log,verdict.json}`, `grade.txt`, `summary.md`, host reference files, per-guest QEMU run logs; PNG screenshots and the
gzipped QEMU logs are left out), `wa1f_winapps.log` and `wa1f_apps_stdout.txt` (the stage's own log). GPU and VM UUIDs are replaced by
distinct synthetic `fa4efa4e-...` values (one per distinct value). No password, key or token is in these files: the harness passes none
(autologon).

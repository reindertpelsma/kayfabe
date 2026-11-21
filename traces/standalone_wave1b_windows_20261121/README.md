STATUS: LIVE (DRAFT, partial), 2026-11-21. MEASURED on two rented Vast boxes (nested KVM guests; evidence is text only: logs, JSON, TSV, TXT). This is standalone WAVE 1b, the Windows half, at origin/next `9b3d165192e03b5cd27a7eca1a0f7f494cbc058a` (merge of #550). The sha is in every leg's `payload_start_<leg>.marker`. The stages are the ones that were RED in wave 1 (`traces/standalone_wave1_20261119/README.md`): `windows_apps` (F1, pin drift) and `specviewperf` (F3, copy time), plus `windows_gate_2`. No kayfabe code was changed and no test was relaxed in this run.

## Verdicts
| stage | verdict | note |
|---|---|---|
| provision (W, S) | PASS | W 705 s, S 744 s, host driver 580.159.04 on both |
| windows_gate (W, S) | PASS | W 3608 s (install 1861 s), S 3108 s (install 1632 s); `resets=0`, hostref green/green |
| **windows_apps** (W) | **PASS** | 10 of 10 apps PASS, `WINAPPS_GRADE kind=PASS expected=10 pass=10 guest_fail=0 boot_fail=0 harness=0 not_run=0 extra=0`, 1899 s |
| **windows_gate_2** (W, own gate inside the same leg) | **PASS** | 1652 s, `resets=0` |
| **specviewperf** (S) | **IN PROGRESS: lane A done (3 PASS, enscape-01 FAIL TIMEOUT), lane B running** | copy step PASS at 46.9 MB/s; rows below |
The W leg ended `EXIT rc=0` with `KF_FINAL_SUMMARY verdict=PASS stages=4` (provision, windows_gate, windows_apps, windows_gate_2).

### windows_apps (F1 of wave 1: closed on `next`)
The public bundle came down through the repo's own fetcher (`w/root/prov/ci08d954cc3f1a9bb7_winapps/fetch_bundle.txt`: `FETCH public-bundle/winapps-open.iso bytes=2708133888`, `PUBLIC_BUNDLE_OK`); the app disk sha256 is `5e01ceff9a475a1b490f00c4823e8c2373144a5786f207224980180fe13f076d` (the bundle of #529/#545), and the verify step that raised `embedded recipe input identity mismatch` in wave 1 did not fire. Apps (mid profile): nvidia_smi, deviceQuery_demo, matmul_t, cup8_ladder, dxprobe_d3d11, glgears_wgl, vulkaninfo, torch_correct, edge_webgl, dxprobe_d3d12: all `class=pass`, `tdr=0 wer=0 gsp=0`. NOT_RUN policy rows of the profile (overlay_yuv_* need a real display, managed_t is UVM) are by design and are listed in `w/root/prov/ci08d954cc3f1a9bb7_winapps/grade.txt`. `rootless=violated` in the verdict lines is expected: rootless is not in scope of this run (`--rootless off`).

### specviewperf (F3 of wave 1)
SPECviewperf 15.0.0, lane `both` (9 viewsets, lane A then lane B), online guest (Windows 11 LTSC evaluation, real clock). Input: the 42 GB private archive `specviewperf-15.0.0.tar.zst` from R2 through a presigned download-only URL minted for this box (12 h TTL, `kayfabe.download-urls/1` file at `/run/kf-ci/download-urls.json`, mode 0600; the URL is not in this directory). It is the one input of the run that is not a public download; the stage computes and records its sha256 (`s/root/prov/ci50cc0f2d21a57148_spec/trust.jsonl`).

**Copy rate (the point of this rerun): 94,190,929,920 bytes in 2009 s = 46.9 MB/s** (the stage's own byte counter after zstd and the lane filter; `copy progress` lines every 60 s, steady 44 to 49 MB/s; `stage.log`). Wave 1 measured about 5 MB/s at the rx counter on an EPYC 7B13 (and 13.7 MB/s on a Ryzen 9 5950X in the earlier run), so the copy was about 9 times faster than the wave-1 figure; as compressed archive bytes (42.0 GB over 2009 s) it is 20.9 MB/s (computed). The derived budget the stage printed was 24724 s (the 5 MB/s floor for 100.7 GB, +20 percent and 600 s, clamped to the time left) and was not close to being hit (the copy took 8 percent of it). The copy was verified (`copy verified: 9 viewset directories match the manifest`). INFERRED, not separated by a control: what changed against wave 1 is #534 (a budget derived from the size, with progress notes, so the rate is now visible) and the host CPU: a Xeon W-2170B (desktop-class single-thread clock, 13 vCPU) against the EPYC 7B13 of wave 1; the guest-side path (ssh tar through the user-mode network) is the single-thread-bound part named in F3/F5.

SPEC rows (`SPEC_ROW` lines; walls are from `SPECRUN_END` in the per-row `*.end.txt`; the "native" wall is `native_s` of `scripts/bench/specviewperf/lanes.json`, MEASURED natively on an RTX 4090, so the ratio mixes the kayfabe overhead with the slower RTX 3090 and is not a kayfabe overhead figure):
| lane | viewset | verdict | subtests | wall in guest | native | ratio |
|---|---|---|---|---|---|---|
| A | energy-04 | PASS | 6/6 | 448 s | 323 s | 1.39 |
| A | creo-04 | PASS | 13/13 | 2219 s | 1243 s | 1.79 |
| A | blender-01 | PASS | 7/7 | 4541 s | 1876 s | 2.42 |
| A | enscape-01 | **FAIL** `reason=TIMEOUT` (budget 6300 s hit) | 5/10 | 6300 s budget hit, from 15:02:23Z | 2042 s | more than 3.1 |
| B | medical-04, maya-07, catia-07, solidworks-08, unreal_engine-01 | NOT RUN YET (follow lane A in the same stage) | | | 318, 400, 803, 989, 1236 s | |
No viewset failed, no guest shutdown, no licence event so far (`SPEC_LICENCE_STATE ... status=Licensed` at every probe, `s/root/prov/ci50cc0f2d21a57148_spec/licence_states.txt`).

### enscape-01: FAIL TIMEOUT, classified as a kayfabe gap (documented; NOT confirmed by a control on this die)
MEASURED (`s/root/prov/ci50cc0f2d21a57148_spec/enscape-01.cli.txt`, status lines only): subtests 01 to 05 completed in 6.7, 14.1, 2.5, 5.3 and 8.1 minutes (37 minutes in all; the UltraHQ subtests took about 2.1 times their High twins, 02 against 01 and 04 against 03). Subtest `06_SanMiguelFullUltraHQ` started at 15:40:43Z and never completed; the stage's budget for the viewset (6300 s, 3 x native) ended it at about 16:47Z, after 66 minutes in that one subtest, where the High twin (05) took 8.1 minutes. The guest stayed up and `Licensed`; the host logged no Xid (`kernel_offsets.log`: `xid=0` throughout) and the viewset's TDR file is empty. One host RM line falls in the stall: `NVRM: virtmemAllocResources: VA Space alloc failed! Status Code: 0x51 Size: 0x90000 RangeLo: 0xb0000, RangeHi: 0x13ffff` at 15:48:51Z (`s/workspace/kernel.log`); its relation to the stall is NOT established (the same RM line family is documented for the VA reservation granule in `docs/design/V3_HELD_HOLE_HANDOFF_20261010.md`, but that fix concerns a different geometry). The boot-B kf3 FB was `fb-mb=8192` on a 24 GB card (`gate_ci50cc0f2d21a57148_run.log`).
Classification (exactly one): **kayfabe gap, already documented**: `docs/design/V3_SPEC_GATE.md` (g) records that enscape-01 under kayfabe fails ("refused by SPEC validation (`raytracing`, `sufficientVideoMemory`, with `fb-mb` 8192 on a 24 GB card) ... until kayfabe passes it natively-like"). What is new is the failure mode: on this sha the viewset gets through five subtests and then stalls, instead of being refused by validation. Hypothesis (stated before any control): the heaviest scene (San Miguel at Ultra HQ) needs more VRAM than the 8 GB framebuffer kf3 serves, and the guest waits instead of failing. Falsifier: the same subtest on native Windows on the same RTX 3090 also takes more than 66 minutes, or the same stall appears with `fb-mb` raised to the card's size under kayfabe (then the cause is not the framebuffer size, and if native also stalls it is a die limitation, not a kayfabe gap). No control was run (no native Windows on this box and no budget for a second boot in this run), so this stays a classification by the documented record, not a confirmed one.

## Boxes (label prefix `kf-accept-w1b-`, KVM image, `vms_enabled`, offers by final total price)
| dir | instance | offer | die | host driver | CPU / RAM | legs |
|---|---|---|---|---|---|---|
| `w` | 61791942 (destroyed by id, absence verified 13:32Z) | 61212266, 0.46 USD/h base | RTX 4090 AD102, 24564 MiB | 580.159.04 | EPYC 7B12 VM, 30 vCPU, 98 GB | W1: provision, windows_gate, windows_apps, windows_gate_2 |
| `s` | 61791940 | 61777894, 0.441 USD/h incl. storage | RTX 3090 GA102, 24576 MiB | 580.159.04 | Xeon W-2170B VM, 13 vCPU, 98 GB | S1: provision, windows_gate, specviewperf |
Both legs ran from the payload (`python3 -I scripts/ci/vast_payload.py --instruction ... --only ...`) at the sha above, started 2026-11-21T11:08:38Z (W1) and 11:09:44Z (S1) from a sparse public `git clone` of the sha; the only files pushed to a box were the instruction JSON (sha256 `e1d68a1a...`), the presigned-URL file (S only) and the process-audit helper. No token, key, e-mail or licence identifier was on a box.

## Named deviations
- D1 (SPEC die): the SPEC box is an RTX 3090, not a 4090. Reason: the rerun asked for the fastest single-thread CPU offer within 1 USD/h; the only RTX 4090 offer with a desktop-class CPU (i7-14700K) lists 31 GB of RAM, below the stage's own 24 GB-in-VM floor, so it could not run it. The 3090 (GA102) has its own Windows gate PASS (`docs/design/V3_WINDOWS_GATE.md`) and passed it here. The die change is a variable of the native-vs-guest ratio above.
- D2 (a launch of mine): the first W1 launch (11:07:50Z) exited at once with `KF_PAYLOAD_FAIL suite=preflight`: the instruction file I had pushed was the wrong one (a shared scratch file had been overwritten by another process; it said `windows: false`). No stage ran. Relaunched at 11:08:38Z with the right file (sha256 `e1d68a1a...`, identical to wave 1's); the leg logs are of the relaunch.
- D3 (leg deadline): the S1 leg's soft deadline was set to 480 minutes (the stage refuses to start a lane when less than setup + 1.25 x native walls + tail remains: about 4.1 h for `both`); the run's own limit was the coordinator's budget, not this value.
- D4: the instruction carries `spec: both`, so the payload set `WINVM_DISK_GB=220` on both boxes (as in wave 1).

## Findings
None new so far. F1 and F3 of wave 1 are not reproduced on this sha (F1 closed for windows_apps; F3's copy step no longer limits; the rows' wall times, not the copy, now decide whether a `both` run fits a time box).

## Scrubbing
GPU UUIDs are replaced by the all-zero UUID, presigned-URL parameters, e-mail-shaped strings and public IPv4 addresses are replaced by script and re-checked by search; no image, screenshot, archive, URL file or QEMU command line is in this directory.

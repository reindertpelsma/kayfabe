# Open Linux rows and the GPU-use assertions: first kf3 guest data (2026-11-17)

STATUS: LIVE, 2026-11-17. MEASURED, one run on one box, text evidence only. Nothing here is a claim about another die, another driver or another day. UUIDs, IP addresses and presign parameters are scrubbed (the only UUID is the all-zero placeholder); there is no identity value, no credential and no URL in these files.

## The run

- **Box**: one Vast KVM VM (`vms_enabled`), rented 20:55 UTC, destroyed 23:10 UTC (2 h 15 min), label `kf-openguest-172054`. RTX 3090 (GA102, 24 GB), AMD EPYC 7B13 as a VM (30 vCPU, 98 GB RAM), Ubuntu 22.04.5, kernel 6.8.0-59, host driver **580.159.04** (the box image shipped 575.51.03; `provision` swapped it), kf3 guest driver **580.159.04**, host ffmpeg 4.4.2 (Ubuntu 22.04); the fat guest's ffmpeg is a different build (6.x: `Lavc60.31.102` in `ENCODER_TAG`).
- **Revisions**: tree branch `claude/open-rows-tree-20261117` = `origin/next` `5e24c253c` plus normal merge commits of PR 471 (FFmpeg and DCGM rows), PR 474 (Vulkan CTS row), PR 475 (GPU-use assertion), PR 464 (MATLAB rows, not exercised) and PR 469 (the llama.cpp build fix). **Runs R1, R1c, R1p, V1, V1b, V1c, G1, G1n and the bring-up ran at `04613288e`** (kf3 binary `kf3-bins/04613288`); **runs F1, V2 and V3 ran at `6f9124b48`** (the three proof and threshold fixes below; V2 and V3 on a scratch copy whose `vkcts` row names a trusted `--caselist` of one case, see `box_scripts/chain6.sh`). `04613288e` and `6f9124b48` differ in the GPU-use proofs (`gpu_use.py`, `gpu_use_rows.py`), the two `nvenc` rows' and the `nvdec` row's commands in `run_apps.sh`, the known-limits table of PR 471, tests and docs. `6f9124b48` does not yet carry the `vkcts.py census_stop` change (a later tree commit), which is why V2's `VKCTS_GPU` line says `video_memory_did_not_rise`.
- **Bring-up** (`bringup/`, `box_scripts/`): `vast_payload.py --instruction payload.instruction.json --only provision --only cuda_host` (552 s, rc 0: host driver, raw client, fast guest, kf3), then the apps stage's own provisioning (`KF_APPS_PROVISION=1 scripts/bench/box/apps_gate.sh` with a two-row list: bundle 14 min, guest image 9 min, `APPS_STAGE_PASS`, `guest_xid=0`), `vkcts.py build` (5 min, 30 cores), and the host baseline's runtime with `setup_side.sh` (3 min). `merge_bar` and `cuda_guest` were not run (time); the first guest rows are the smoke test.
- **Commands** (the PRs' own, with `KF_GUEST_IMG=/workspace/bench/guest_apps.qcow2 KF_DEVICE=kf3`, one GPU job at a time, host first):
  - PR 471: `apps_matrix.sh host R1 nvcodec_matrix nvml_dcgm`, then `APPS_APP_TMO=3600 apps_matrix.sh guest R1 ...`; control `host R1c nvcodec_matrix`; persistence run R1p (`nvidia-smi -pm 1` on the host, `APPS_GUEST_PM=1`).
  - PR 474: `apps_matrix.sh host V1 vkcts`, `guest V1 vkcts`, `vkcts.py diff`; control `host V1c`; repeat `guest V1b`; the three candidates alone: `host|guest V2` (DGC case), `V3` (one sparse case).
  - PR 475: the section 7 rows (`blender_cycles llama_cpp_gen llama_bench torch_correct hf_generate cupy nvenc_h264 nvenc_hevc nvdec_h264 hashcat clpeak vkpeak egl_offscreen`; `torch_ai_bench` left out) host `G1`, guest `G1` in ONE boot (`APPS_PER_BOOT=100`; the matrix re-ran the three non-PASS rows alone in fresh boots, same result); negative controls `KF_NEGCTL=1` host and guest `G1n` (without `vkpeak`, 7.4 minutes per side); the fixed rows `F1`.
  - Not run: the MATLAB rows (image C was not needed once the time went to the candidates), `torch_ai_bench`, the vkpeak control, `merge_bar`, `cuda_guest`.

## Results, host against guest (same die)

| row | host | guest | diff |
|---|---|---|---|
| `nvcodec_matrix` (47 cases) | pass 35, not_supported 7, fail 0, mismatch 0, not_run 5, 143 s; host against host (R1 against R1c): 47/47 agree, 0 capdiffs | pass 31, not_supported 16, fail 0, mismatch 0, not_run 0, 245 s | 38 agree, 9 disagree, 4 candidates, 1 capdiff: all explained (below) |
| `nvml_dcgm` | nvml_fields_ok 57, queries 63/74, smi 647/734, dcgm_fields 159/768; DCGM diag pass 2 / skip 7 (persistence on) | NVML compare `FINDING_NA_IN_GUEST` 132, DCGM 74, `OK_SERVED_THOUGH_LISTED` 3, `GUEST_EXTRA` 2; host against host 0 | all telemetry (below) |
| `vkcts` (291 cases) | pass 189, not_supported 102, fail 0, 104 s, digest `70d5ed8f0ca07431`; control V1c identical (91 s) | pass 186, not_supported 102, fail 1 (`DeviceLost`), timeout 2, 877 s, digest `688c33b1ebe96264`; repeat V1b identical (868 s) | 186 `SAME_OK`, 102 `SAME_NOT_SUPPORTED`, **3 `GUEST_ONLY_FAIL`**, 0 `GUEST_LOSES_SUPPORT`, 0 `GUEST_GAINS_SUPPORT`; extension sets equal (250) |

### The candidates and their buckets (owner's three-bucket rule; a host control for each)

| finding | bucket | why |
|---|---|---|
| `nvcodec_matrix` `sess_h264_n04`, `n05`, `n06`, `n08` (host PASS, guest NOT_SUPPORTED `session_limit`) and the capdiff `sessions.opened` host 8, guest 3 | **1, documented** | `crates/kf-qemu/src/encquota.rs` `ENC_SESSIONS_PER_VM = 3` (security review S1-42, a design choice in the source comment; `V3_SECURITY_MODEL.md`). Every other probe line is equal. Not listed in `nvcodec_known_limits.tsv` (it was pinned empty): fixed on PR 471's branch, test first |
| `nvcodec_matrix` 5 x `enc_av1_*` (host NOT_RUN `unknown_encoder`, guest NOT_SUPPORTED, rc 234) | **3, our configuration** | host ffmpeg 4.4.2 has no `av1_nvenc`, the guest's ffmpeg has one and the attempt fails; both probes say the die has no AV1 encode (the summary headers carry the two ffmpeg digests). The AV1-encode hole of PR 471 is not closed: no Ada die here |
| `nvml_dcgm` 132 NVML and 74 DCGM findings | **1 once read against `V3_UTILIZATION_FLAG.md` section 10**, now MEASURED | clocks, power, temperature, fan, perf state, throttle reasons and clock-event counters, PCIe throughput, replay counter and link width, `utilization.*` columns: the guest answers `NOT_SUPPORTED` / `[N/A]`. Whether the guest's NVML gets them another way (the open question of PR 471) is answered: no. The table `nvml_expected_not_supported.tsv` still does not list them (the owner decides) |
| `nvml_dcgm` DCGM diag `Deployment/software` FAIL | **3, persistence mode** | FAIL on the host as well (rc 226 after 1 s) with persistence off; with `nvidia-smi -pm 1` / `APPS_GUEST_PM=1` (R1p) PASS on both. The other seven level-3 tests SKIP on both sides on a GeForce |
| `vkcts` `sparse_resources.buffer.texel_buffers.sparse_residency.storage_texel_buffer_read_size_2_24_r32ui` (Pass / Timeout) | **candidate, unclassified** | reproducible (V1, V1b, and alone in a fresh boot, V3 uses the other sparse case); host Xid 109 `CTX SWITCH TIMEOUT`, channel 0x31, process = the guest's QEMU; host passes twice |
| `vkcts` `sparse_resources.multisampled_image_sparse_residency.rgba16ui.samples_4` (Pass / Timeout) | **candidate, unclassified** | the same: V1, V1b, V3 alone; Xid 109 |
| `vkcts` `dgc.ext.graphics.multiview.view_mask_2.no_ies_gpl.indexed_draw_buffer_tokens` (Pass / DeviceLost) | **candidate, unclassified** | reproducible in V1, V1b and alone in a fresh boot (V2); host Xid 13 `3D-Z SYSMEM KIND Violation` on GPC 1 and 2, class `0xc797`, channel 0x25; kf3 logs `RC host twin 0x25 ... except_type=0xd (Xid 13) - forwarding RC_TRIGGERED`. Lead: `crates/kf-mem/src/apply.rs` `host_pte_kind` keeps PITCH for guest RAM ("sysmem holds no depth/stencil surface kinds") |

Not bucket 2 (the host passes the same cases in two full runs with equal digests and in two runs of the case alone, with no host Xid); not bucket 1 (nothing in `docs/` lists sparse residency or this depth case as unsupported); bucket 3 is not excluded for the guest's memory placement. **No kayfabe bug is claimed**: the host control and the isolation say the failures are the guest's, the root cause is not found, a GPU-free regression test needs it. `host_xid_lines.txt` has every host Xid of the run (each full vkcts guest run: two Xid 109 and one Xid 13 event; V2: the Xid 13; V3: one Xid 109; none during any host run and none in any other row). H5 of the vkcts plan ("the first candidates are in `video_*`") was wrong: the 43 `video_*` cases all agree.

## kf3 census per row (guest)

`census_rows.tsv`: per APPRES line the verdict, `gpu_use`, `gpu_auth`, `gpu_subs` (submissions in the window), births, `guest_xid`, the host census of the open rows, `unserviced_in_row` and the ids, and from the row's kf3 log slice the cumulative `gsp_refusals` total, distinct count and top commands (and the delta against the previous row of the same boot). Summary: guest Xid 0 in every row (the Xids are on the host, above); `REFUSED` lines: see the file; **unserviced GSP RPCs in the row window**: `nvcodec_matrix` 4193 (`fn76` = GSP_RM_CONTROL answered `0x56`; top commands `0x20802087` 1830, `0x20802a12` 1616, `0x20808165` 303 (the NVENC session accounting control, `V3_FEATURE_GAPS.md` L13), `0x20810108` 86), `nvml_dcgm` 1083 and 793 (ids `code:76`, `code:50`), simple rows 33 to 36 in their whole log slice. The applications survived all of them. `kf3_refusals=` in the row line counts every kf3 line that contains `refus` (including `refused=0` fields), so 15 to 50 is the floor of an ordinary row, not a signal.

## The GPU-use assertions (PR 475, section 7 of its design)

| id | result |
|---|---|
| V1 positives | host: 10 of 13 rows `gpu_use=PASS`, 3 FAIL (`nvenc_h264`, `nvenc_hevc`, `nvdec_h264`: harness errors, below); guest: 10 of 13 `gpu_use=PASS gpu_auth=host` (`llama_cpp_gen` 3176 submissions, `llama_bench` 4118, `torch_correct` 4004, `hf_generate` 3717, `cupy` 1070, `nvenc_h264` 1501, `nvenc_hevc` 1493, `clpeak` 10528, `vkpeak` 1620, `egl_offscreen` 56), 3 FAIL (`blender_cycles`, `nvdec_h264`, `hashcat`) |
| V1 after the fixes (F1, `6f9124b48`) | `nvenc_h264`, `nvenc_hevc`, `nvdec_h264`, `hashcat`, `blender_cycles` all `PASS gpu_use=PASS`, host and guest (guest `gpu_subs` 1192 to 23253) |
| V2 negative controls | 9 of 9 `CONTROL_OK judged=FAIL` on the host and 9 of 9 in the guest (seven of the guest judgements are the host census, one the host engines, one the app proof). `egl_offscreen`: the Mesa vendor file makes `eglInitialize` fail here (no software EGL device), so that control only proves the missing-proof path |
| V3, V5, V6 | held (the `pix_fmt: cuda` marker is the second `Reinit context` line; the host GPU engine was busy in every passing row that requires it; every window had a status line before it) |
| V4 | held with a correction: hashcat in the guest runs on the NVIDIA GPU through OpenCL (`* Device #1: NVIDIA GeForce RTX 3090, 7552/7871 MB ...`) because its CUDA backend is unusable there (`CUDA SDK Toolkit not installed`, bucket 3); the host uses CUDA. `ENCODER_TAG` exists with ffmpeg 6 only |
| V7 | not run (MATLAB) |

The three defects (all wrong thresholds or proofs, found by the host control or by the measured numbers, no threshold relaxed; fixed test first, commits on PR 475's branch):
1. a guest's device memory is the 8 GiB store the host reserves at boot, so host video memory does not rise with a guest run (`blender_cycles`: +9512 submissions, GPU 53 percent busy, 8761 to 8792 MiB): `mem` is required of a bare-metal run only (`host_engines(app, side)`); the same wrong requirement was in `vkcts.py census_stop` (PR 474's tree commit);
2. ffmpeg 4.4.2: no `encoder` stream tag in the mp4 (the encoder is only in the indented stream-mapping line, which the row's `grep -v "^  "` dropped, and `MAPPING_RX` stopped at the first `)`), and `grep -m1 "Reinit context"` kept the software-format line and dropped `pix_fmt: cuda`;
3. hashcat prints `* Device #1: CUDA SDK Toolkit not installed ...` in the device-line form, which was read as a non-NVIDIA device.

## Other fixes found (harness errors)

- `scripts/ci/vkcts_mutants.py` did not copy the two real-output fixtures into its mutation tree, so its unmutated baseline died (`FileNotFoundError`) and the CI step could not pass: fixed on PR 474's branch (42/42 mutants killed).
- `test_gpu_use.py` pins 71 rows of `run_apps.sh`; PRs 471 and 474 add three discovery rows (74): class `own` in `gpu_use_rows.py` for `nvcodec_matrix`, `nvml_dcgm`, `vkcts` (they assert GPU use inside their own driver); this belongs to whichever of the three PRs merges last, it is on the tree branch only.
- Cosmetic: `apps_matrix.sh` prints `guest_isolated.res: No such file or directory` at the end of every one-app-per-boot run.

## Follow-up, 2026-11-18: two hardware confirmations (`confirm/`)

On a second box (same die, same driver, kf3 built from the tree plus PR 480): **the DGC multiview case passes alone on the fixed kf3 (`ad1a0c4c`, no host Xid) and fails alone, with the same host Xid 13 `3D-Z SYSMEM KIND Violation`, on a control kf3 that differs only in `kf-mem/src/apply.rs` (`ad1a0c4c-dirty`)**: updated verdict for that candidate: a kayfabe-core defect (`host_pte_kind` rewriting a Z kind on a guest-RAM row to PITCH), fixed by PR 480, hardware-confirmed once per binary. For the two sparse-residency Timeouts: each half-resident case hangs with host Xid 109 and each fully bound case passes before and after (texel buffer `size_2_16` against `size_2_24`, and the size-matched image pair `multisampled_image_sparse_binding` against `..._residency`); the named twin of the brief does not exist in vulkan-cts-1.4.7.1; the mechanism (sparse entries never written) is NOT shown by the kf3 logs. Updated bucket for those two: an undocumented kayfabe limitation, probably a core gap, reproducible, root cause open. Details and the exact commands: `confirm/README.md`.

## Caveats

- One die (GA102), one driver (580.159.04), one host kernel; ffmpeg differs between host (4.4.2) and guest (6.x), which is a bucket-3 difference of its own.
- The guest positives ran batched in one boot (the matrix default of the gate), not one app per boot as section 7 asked; every non-PASS row was re-run alone and reproduced.
- The per-step logs of `/workspace/steps` were not collected before the box was destroyed; the results directories (`runs/`), the payload and provisioning logs (`bringup/`) and the exact scripts (`box_scripts/`) are.
- `*.kf3.log` slices, serial logs and sampler CSVs are not kept (size); `census_rows.tsv` carries what they were read for.

## Files

`runs/<RUN>/`: `host.res`, `guest.res`, `negctl.res`, `host_negctl.res`, `*.host.log`, `*.guest.log`, `*.compare.txt`, `vkcts_diff.tsv`, `vkcts.*.log`, `*.gpu_use.json` (the judge's record per guest row), `*.guest_dmesg.log`, `boot_*.driver.log`. `bringup/`: payload start, exit and stdout, provisioning logs. `box_scripts/`: the scripts that ran on the box and on the coordinator. `census_rows.tsv`, `host_xid_lines.txt`.

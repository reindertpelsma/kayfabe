# V3 open rows: the FFmpeg NVENC/NVDEC capability matrix and the DCGM + NVML sweep

**STATUS: LIVE, 2026-11-17 (code and GPU-free tests; native results from two Vast container boxes are in section 10; the FIRST kayfabe guest run (RTX 3090, kf3 `04613288`, same day) is in section 13. Every sentence tagged [inferred] or [not measured] is a hypothesis with a falsifier in section 11).** Owner ruling 2026-11-17 (binding): go OPEN FIRST; an open row works for a standalone third party who trusts git only, from the repository alone, fully automated: no private R2 object, no account, no secret, no licence. A Worker may keep a cache, but nothing in this change reads one.

Labels: **[read]** a file or page read, **[run: <directory>]** a logged run (its directory is named), **[inferred]** my reading, not tested.

## 1. What the two rows are

Both are **discovery rows** of the open Linux matrix: they are in `scripts/apps/run_apps.sh` (so `apps_matrix.sh` runs them on the host and in a guest like any app) and in **no gate list** (`gate_list.txt`, `gate_list_middle.txt`, `expected_rows.txt`; a test pins this). They become gate rows only after the discovery lane has run clean three times on two dies (the proposal of `V3_PROPIMAGE_LINUX.md` section 7, applied here).

| row | what it asks of the driver | what a failure is |
|---|---|---|
| `nvcodec_matrix` | 47 cases of `committed table x the machine's own caps`: NVENC H.264/HEVC/AV1 encode (8 and 10 bit, 4:4:4, B-frames and lookahead, above 4096, 5K, 8K), parallel sessions around the session cap, NVDEC decode of H.264/HEVC/VP9/AV1 (8 and 10 bit, 4:4:4, 8K) through `-hwaccel cuda` and `cuvid` | a case the machine says it supports that did not work, or one it says it does not support that worked (MISMATCH) |
| `nvml_dcgm` | every NVML field value id the driver knows, 74 typed NVML queries, `nvidia-smi -q -x`, every `nvidia-smi --query-gpu` property, `dcgmi discovery/dmon/diag` | a number the host answers that the guest answers with `[N/A]`, `NOT_SUPPORTED` or nothing, unless the exact value is in the committed table of documented limitations |

## 2. Files

| file (`scripts/apps/open_rows/` unless noted) | role |
|---|---|
| `nvcodec_probe.c` | asks the driver: `NvEncGetEncodeCaps` for every cap id of H.264/HEVC/AV1, the profiles and input formats, `cuvidGetDecoderCaps` for 10 codecs x 4 chroma formats x 3 depths, and how many encode sessions the driver lets this machine open at once. Prints TSV; never the UUID, serial or bus id |
| `nvcodec_cases.tsv` | the case table (id, group, op, codec, profile, pix_fmt, chroma, depth, size, frames, sessions, mode, flags, `needs`, PSNR threshold) |
| `nvcodec_lib.py`, `nvcodec_matrix.py` | the decisions (pure functions) and the runner; `compare` of a host and a guest run |
| `nvcodec_known_limits.tsv` | documented kayfabe limitations of the FFmpeg row: since the first guest run (2026-11-17, `traces/open_rows_guest_20261117/`) the per-VM NVENC session cap of 3 (`encquota.rs`, S1-42), cases `sess_*_n04` and above |
| `nvml_sweep.py`, `nvml_lib.py`, `dcgm_nvml.py` | the NVML sweep over ctypes, the row diff and the DCGM parsers, the DCGM runner and its `compare` |
| `nvml_field_classes.tsv`, `nvml_expected_not_supported.tsv` | how each value is compared (static, live with a tolerance, only-a-number); the documented NOT_SUPPORTED table, each line with its reason |
| `census.py`, `rowio.py`, `compare_logs.py` | the host census (GPU use, unserviced-RPC count), the `@@FILE` hand-off of result files through a log, the comparison front end |
| `provision.sh`, `pins.env`, `run_standalone.sh` | fetch and build on one machine (glue, under 100 lines); the pins; the standalone entry |
| `scripts/ci/test_open_rows.py`, `scripts/ci/open_rows_mutants.py` | GPU-free tests (fixtures with the fake UUID prefix `fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e....`) and 83 mutants with two named killers each |

Wiring: `run_apps.sh` (two rows), `build_bundle.sh` (a non-fatal `provision.sh` call and the copy of `open_rows/` into the bundle), `apps_hook.sh` (pushes `open_rows/` into the guest, samples the HOST for each of the two rows, prints the unserviced-RPC census), `apps_matrix.sh` (the compare at the end), `image/recipe.json` (the new sha256 of `run_apps.sh`, which the open image recipe pins).

## 3. Supply chain: the trust anchor of every input

Owner rule 2026-11-17: a sha256 of a downloaded binary pinned in git is not an anchor (it cannot be reviewed). Accepted: an official upstream signature or checksum fetched at run time from the upstream origin; a GitHub attestation; for sources, a pinned upstream commit or tag. A test (`StaticContracts`) fails when any file of the row holds a 64-hex digest.

| input | where it comes from | anchor |
|---|---|---|
| nv-codec-headers (MIT, headers only, for the probe) | `https://github.com/FFmpeg/nv-codec-headers.git`, tag `n12.2.72.0`, commit `c69278340ab1d5559c7d7bf0edf615dc33ddbba7` (the commit, not the tag object) | pinned upstream commit, checked by `pin_lib.sh: pin_git_checkout` (HEAD must equal the pin; the first attempt pinned the tag object by mistake and the helper refused it, [run: traces/open_rows_ffdcgm_20261117]) |
| ffmpeg and ffprobe | the distribution's package (`apt-get install ffmpeg`) | the distribution's signed archive; nothing is downloaded by us. A static build from a third party was considered and dropped: its checksum is published beside it and is no anchor |
| DCGM 4.8.0 (`datacenter-gpu-manager-4-cuda12`, `...-proprietary-cuda12`) | NVIDIA's apt repository `developer.download.nvidia.com/compute/cuda/repos/<ID><VERSION_ID>/x86_64`; the repository key `3bf863cc.pub` is fetched at run time from the same official URL over https and installed as `signed-by` | the package manager's signed-repository mechanism; the version string `1:4.8.0-1` is pinned (a version, not a hash). A machine that already has NVIDIA's repository configured keeps its own key |
| `libnvidia-ml.so.1`, `libnvidia-encode.so.1`, `libnvcuvid.so.1`, `libcuda.so.1` | the driver on the machine under test | not downloaded |
| NVML field ids | the driver (ids 0 to 1023 are asked; the driver's answer decides which exist). `nvml.h` is read only for labels, when a toolkit's copy is next to the bundle | not downloaded |
| test media | generated at run time by ffmpeg's `lavfi` (`testsrc2` plus a seeded `noise` filter; seed 1234) | none needed |

DCGM exists in NVIDIA's public repository without a login [read: the repository's `Packages` index, 2026-11-17, versions 2.4.5 to 4.8.0 for Ubuntu 22.04 and 24.04]. Redistribution is not needed: each machine installs it itself.

**Decided 2026-11-17 (coordinator): Worker runs stay at `dcgm=NOT_RUN`.** Under the Vast payload `apt-get` is a cache-only shell function that holds no NVIDIA package. No bypass is added: the sanctioned one is in `pin_lib.sh` and `test_apps_pins.py` pins it to that one place. Making DCGM a member of the cache-only package set was examined and not done, because it would weaken that set: the set is `provision_box.sh --packages <profile>` and a test fails on any cuda or nvidia package in it (owner ruling 2026-11-07, nothing from NVIDIA is stored in our images, R2 or release assets); the cache builder `setup_cache_build.py` fetches from Ubuntu archive hosts only, and the cache is a stored blob, so DCGM's `.deb` files would be stored NVIDIA binaries from a new host. So `provision.sh` reports `OPENROWS_dcgm=FAIL reason=apt_shim_cache_only` under the payload and the row reports `dcgm=NOT_RUN reason=dcgmi_not_installed`, while the NVML sweep, the nvidia-smi sweep and the FFmpeg row run; a third-party standalone run installs DCGM from NVIDIA's signed repository as above.

Bandwidth: a standalone machine downloads the distribution's `ffmpeg` and its dependencies and DCGM (section 10 gives the sizes the boxes logged); `ci/vast-broker/bandwidth.json` carries both under the `apps` stage.

## 4. Row 1: the FFmpeg matrix

**Derive, never capture.** No table in the repository says what a die can do. `nvcodec_probe` asks the machine it runs on; each case has a `needs` expression over that answer, for example `enc:hevc & enc:hevc:SUPPORT_YUV444_ENCODE>=1 & enc:hevc:dim`, `enc:av1`, `enc:sessions>=5`, `dec:vp9:420:10:dim` (`dim` compares the case's size with the reported min and max). The grammar is parsed, never evaluated as code. `nvcodec_cases.tsv` contains no die, family or driver name (a test pins it).

**Every case is run, whatever the caps say.**

| the caps say | the attempt | result |
|---|---|---|
| supported | worked and the output validated | PASS |
| supported | failed | FAIL (the reason from ffmpeg's own error text: `encoder_params_rejected`, `hw_decode_unsupported`, ...) |
| not supported | failed | NOT_SUPPORTED (the driver and ffmpeg agree it cannot) |
| not supported | worked | MISMATCH (the caps lie, or the probe is wrong: a finding) |
| the probe could not ask (NVENC or NVDEC unreachable), no ffmpeg, no encoder in this ffmpeg, no software encoder for a decode stream | not attempted | NOT_RUN with the named reason; our own configuration, never a statement about the GPU |

**Validation.** Encode: `ffprobe` must read a stream of the right codec, size and frame count; the output is decoded in software and compared with the same lavfi source (`psnr` filter, per-case threshold in the table, 28 dB for ordinary cases, 25 dB for 5K and 8K). Decode: a software encoder (`libx264`, `libx265`, `libvpx-vp9`, `libsvtav1` or `libaom-av1`, whichever this ffmpeg has) makes the test stream at run time; one ffmpeg process then decodes it twice, in software and on NVDEC, and prints their PSNR (identical frames print `inf`; the threshold is 45 dB). `-hwaccel_output_format cuda` plus `hwdownload` makes a failed NVDEC setup an error instead of a silent software fallback (the same trap `video_lane.sh` records).

**Parallel sessions.** The probe opens encode sessions until the driver refuses (up to 16) and reports the count. The table runs N parallel ffmpeg processes for N = 1, 2, 3, 4, 5, 6, 8, 9, 12. Up to the discovered cap all N must work; above it exactly the cap must work, and a different count is a MISMATCH (`partial_sessions`). Whether the cap is 3, 5 or 8 on a given driver is the machine's answer, not ours.

**Host and guest.** The same table and tool run on both sides. `compare` lines up the cases and also diffs the probes (every cap id, every decoder row, the session count). A case that works on the host and fails in the guest is a candidate kayfabe bug unless `nvcodec_known_limits.tsv` explains it.

## 5. Row 2: the DCGM + NVML sweep

`nvml_sweep.py` talks to `libnvidia-ml.so.1` over ctypes (no pynvml, no download). Field value ids 0 to 1023 are asked 64 at a time; a per-field `INVALID_ARGUMENT` means the library does not know the id and the id is not listed. The typed queries cover utilization (gpu, memory, encoder, decoder, JPEG, OFA), clocks (current, max, applications), power (usage, limits, constraints, energy), temperature and thresholds, fan, throttle reasons, ECC, PCIe (links, throughput, replay counter), memory and BAR1, encoder stats and sessions, compute, graphics and MPS process counts, identity-like statics, NvLink state. Their struct layouts are the public `nvml.h` ones.

`nvidia-smi -q -x` is flattened to one row per leaf (identity leaves become `<scrubbed>`); `nvidia-smi --help-query-gpu` supplies the property names and one call reads them all.

**The rule** (`nvml_lib.py: compare_row`): for each row the HOST answers with a number, the guest must answer with a number. Equal for `static` values, within a tolerance for `live` counters (`nvml_field_classes.tsv`, tolerances [inferred] and expected to move with the first guest run), only-a-number for the rest (memory totals differ by kayfabe's frame-buffer policy). A guest `N/A`, `NOT_SUPPORTED`, a missing row or a missing part is `FINDING_*`, unless `nvml_expected_not_supported.tsv` lists the value: ECC and NVLink (`OWNER_RULINGS.md` section H), the profiler fields, per-process figures (`V3_UTILIZATION_FLAG.md` section 10). Power, clocks, P-state and thermals are **not** in the table on purpose: that document says the utilisation flag leaves them N/A as RUSD records, whether the guest's NVML gets them another way is not measured, so the row reports them. A listed value the guest does serve is reported `OK_SERVED_THOUGH_LISTED`, which says the line should go.

DCGM: `dcgm_nvml.py` starts `nv-hostengine` itself, records `dcgmi discovery -l`, every field of `dcgmi dmon -l` (eight per call, three samples), and `dcgmi diag -r 1` and `-r 3 -j` (the level is an option; the diag timeout defaults to 1500 s in the matrix row). The diag JSON is parsed into one row per test and GPU; a test the host passes and the guest fails is a finding. Without DCGM the part is `NOT_RUN` with the reason in `dcgm_status.txt`. In an offline guest nothing is installed: the box installs DCGM and the bundle carries the installed files (`dcgm_root.tar`), which the row unpacks as root.

## 6. The host census

A guest's `nvidia-smi` cannot prove that work reached the GPU: `utilization.*` is `[N/A]` there until the utilisation flag (`V3_UTILIZATION_FLAG.md`, PR #459) serves it. So `apps_hook.sh` runs `census.py sample` on the HOST around each of the two rows (`nvidia-smi --query-gpu=encoder.stats.sessionCount,utilization.encoder,utilization.decoder,utilization.gpu,memory.used -lms 250`) and appends `host_gpu_use=PROVEN|UNPROVEN|FAILED` to the row's result line. PROVEN needs a readable counter that moved (session counter at least 1 for the encode row; two samples at 1 or more for an engine); readable and idle is FAILED; only `N/A` or no samples is UNPROVEN, never a pass. In a native run the rows sample themselves (`gpu_use=` in the summary).

The same line carries `unserviced_before`, `unserviced_after` and `unserviced_in_row`: the number of `GSP rpc UNSERVICED` lines in the kf3 QEMU log up to the moment the row started, up to its end, and the difference, counted by `scripts/bench/windows/kf3_census.py` (the census the Windows and gfxset lanes read), plus the distinct ids seen in the row. That is the before/after record of the owner's request. A zero is trusted only because `test_open_rows.py` feeds the census a log with known positives.

## 7. Comparing, and the three buckets

`compare_logs.py <row> HOST_LOG GUEST_LOG` (also run by `apps_matrix.sh` at the end of a guest run) prints one line per case or finding and one summary; its exit status is 1 when there is a candidate.

1. **Documented kayfabe limitation**: the guest result is explained by a line of `nvcodec_known_limits.tsv` or `nvml_expected_not_supported.tsv`, which name the document that decides it.
2. **Application or harness bug, shown by the host control**: the host fails or mismatches the same case (so it is not kayfabe's).
3. **Our own configuration error**: `NOT_RUN` with a named reason (`nvenc_unavailable`, `no_encode_device`, `driver_api_too_old`, `unknown_encoder`, `no_sw_encoder`, `apt_shim_cache_only`, ...).

Anything else that the host does and the guest does not is `kayfabe_candidate_bug_undocumented`.

## 8. Running it (the commands the next gate or standalone run uses)

Standalone, any machine (root, network once): `sudo bash scripts/apps/open_rows/run_standalone.sh OUT`.

Gate lane, a bench box that already has the bundle: `bash scripts/apps/build_bundle.sh` (adds the probe and, where apt is not the shim, DCGM), then the host control and the guest run, strictly serial:

```text
KF_DEVICE=kf3 bash scripts/apps/apps_matrix.sh host  RUN nvcodec_matrix nvml_dcgm
KF_DEVICE=kf3 APPS_APP_TMO=3600 bash scripts/apps/apps_matrix.sh guest RUN nvcodec_matrix nvml_dcgm
```

The matrix row's own time limit is 3000 s and the DCGM row's 3300 s (the diag level 3 has a 1500 s cap inside). Durations of the native runs are in section 10; the guest run is expected to be slower [inferred]. Read `RUN/*.compare.txt`, the `APPRES` lines (`host_gpu_use=`, `unserviced_*=`) and `RUN/*.guest.log`.

## 9. Tests and mutants (GPU-free)

`scripts/ci/test_open_rows.py` (225 tests): table validity and its tamper cases; the probe parser; the caps comparison on made-up Ampere-like and Ada-like probes; the NOT_SUPPORTED, MISMATCH and NOT_RUN handling; PSNR threshold boundaries; command construction (argument vectors, never shells); ffmpeg error classification; the host/guest buckets; the NVML diff (every status, tolerances at their boundaries); the DCGM parsers on both JSON layouts; the census with known positives; the `@@FILE` hand-off with corruption; the sweep against a fake `libnvidia-ml` built from C in the test; the FFmpeg runner end to end against a fake `ffmpeg` and a fake probe; the DCGM runner end to end against a fake `dcgmi`; and text pins for the trust rules and the wiring (marked as pins: they prove the line is there, not that it works). `scripts/ci/open_rows_mutants.py` runs 83 mutants, each killed by at least two named tests; it is part of the `scripts/ci` step of `ci.yml`.

## 10. Native results

Evidence: `traces/open_rows_ffdcgm_20261117/` (text only; every file passed `scripts/ci/no_gpu_uuids.py` and the scrubbing of `census.py`). Revision under test: `da4fa3b3e` (branch `claude/open-ffmpeg-dcgm-20261117`); earlier runs at older revisions found the defects listed below and are not kept. Two machines:

| | machine | driver | notes |
|---|---|---|---|
| B1 | Vast **container** box, RTX 3060 (GA106), Ubuntu 24.04 image `nvidia/cuda:12.6.3-base-ubuntu24.04`, no KVM | 580.126.20 | ffmpeg 6.1.1 (distribution), DCGM 4.8.0 |
| B2 | Vast **KVM VM**, RTX 3060 (GA104) passed through, Ubuntu 22.04 | 575.51.03 | ffmpeg 4.4.2 (distribution) |

**Vast containers do not give a process NVENC or NVDEC** [run: traces/open_rows_ffdcgm_20261117/container_rtx3060_ga106_driver_580.126.20/matrix_in_container.txt]. On B1 the probe's `NvEncOpenEncodeSessionEx` answers `NV_ENC_ERR_UNSUPPORTED_DEVICE` and `cuvidGetDecoderCaps` answers `CUDA_ERROR_NO_DEVICE`, although `libnvidia-encode` and `libnvcuvid` of the matching driver are in the image and the instance's environment lists `NVIDIA_DRIVER_CAPABILITIES=all`; ffmpeg says the same (`OpenEncodeSessionEx failed: unsupported device (2)`). A second container host with an RTX 4070 (Ada, driver 580.126.09) gave the same probe error at the first probe; its output was not kept. The row reports every case `NOT_RUN` with `nvenc_unavailable` or `nvdec_unavailable` (the harness behaves as designed; it never guesses an expectation) and the FFmpeg row was therefore run on a KVM VM (B2). The reason is not established [inferred: the container runtime or the host's device handling]. The ask "validate natively on a container box" holds for the NVML and DCGM row only.

**FFmpeg matrix on B2** [run: traces/open_rows_ffdcgm_20261117/vm_rtx3060_ga104_driver_575.51.03/results.tsv, summary.txt, probe.tsv]: `pass=35 not_supported=7 fail=0 mismatch=0 not_run=5 cases=47`; the summary's `gpu_use` field is set because the encoder session counter reached 8; 159 s for the whole row.

- NOT_SUPPORTED, each shown by the failing attempt: `enc_h264_10bit_1080p` (`10 bit encode not supported`), `enc_h264_above4096` and `enc_h264_8k` (`Width ... exceeds 4096`), `sess_h264_n09` and `sess_h264_n12` (exactly 8 of N sessions worked, then `incompatible client key`), `dec_h264_420_10_hw` and `dec_h264_above4096_hw` (NVDEC reports no H.264 10-bit and a 4096 maximum).
- NOT_RUN: the five AV1 encode cases, reason `unknown_encoder`: the die has no AV1 encode (probe: `enc_codec av1 0`) and Ubuntu 22.04's ffmpeg 4.4.2 has no `av1_nvenc`, so a die that does have AV1 encode is **not exercised by any run**: no Ada die with a working NVENC was available (the one KVM Ada offer failed to pull its image; the container Ada box has no NVENC). This is the largest hole of the native evidence.
- The session cap on this driver is 8 and the driver words it `NV_ENC_ERR_INCOMPATIBLE_CLIENT_KEY`, not out-of-memory; N parallel sessions around it behave as the probe predicted.
- Encode PSNR against the lavfi source was 27.9 to 30.0 dB at constant QP 24 (H.264 27.90, HEVC 28.06 to 29.97), so the committed threshold is 24 dB; the first draft's 28 dB failed the H.264 case by 0.1 dB.
- Decode: NVDEC and software agree bit for bit (PSNR `inf`) for H.264, HEVC (8, 10 bit, 4:4:4 8 and 10 bit), VP9 (8, 10 bit) and AV1 (8, 10 bit), through `-hwaccel cuda` and through `cuvid`. AV1 through `-hwaccel cuda` needs `-c:v av1`: ffmpeg prefers `libdav1d`, which has no NVDEC hwaccel (the first run failed with `Impossible to convert between the formats`).

**NVML and nvidia-smi on B1** [run: traces/open_rows_ffdcgm_20261117/container_rtx3060_ga106_driver_580.126.20/nvml_sweep.tsv, summary.txt]: the driver knows 287 field ids of the 1024 asked (56 answer a number, 231 `NOT_SUPPORTED`); of 74 typed queries 62 answer; `nvidia-smi -q -x` has 1133 leaves (85 `N/A` on this GeForce, for example ECC, remapped rows, module power); `nvidia-smi --query-gpu` has 154 properties (84 `[N/A]` on the host: the same ECC, retired and remapped rows, module power and power smoothing, tlimit). The twelve typed queries the HOST refuses are: `ecc.mode`, the four `ecc.total_errors.*`, `static.mig_mode`, the four `clock.app*`, `temperature.threshold.2` and `nvlink.state.0`. These are not owed by a guest (the host answers no number). B2 shows the same twelve.

**DCGM 4.8.0 on B1** [run: traces/open_rows_ffdcgm_20261117/container_rtx3060_ga106_driver_580.126.20/dcgm.tsv, dcgm_status.txt, diag_r3.json, nvvs_test_lines.txt]: it installs from NVIDIA's repository with the key from NVIDIA's official URL, no login. The first install was the `cuda12` flavour and `dcgmi diag` failed with `Cannot load plugins ... plugins/cuda13/` because the driver reports CUDA 13.0: the flavour must follow the driver, which `provision.sh` now reads from `nvidia-smi` (the cuda13 flavour was then installed beside it). `dcgmi dmon -l` lists 768 fields; 159 answer, 97 are refused (`Profiling is not supported for this group of GPUs or GPU`, -36, which fails a whole group, so the row splits a failing group until the refused field stands alone) and 513 print `N/A`. `dcgmi diag -r 3 -j` ran in about one second and returned one `Pass` (`Deployment/software`) and seven `Skip` (`memory`, `diagnostic`, `nvbandwidth`, `pcie`, `memory_bandwidth`, `targeted_stress`, `targeted_power`); DCGM's log says only "The <name> test is skipped. Check DCGM and system configuration." The whole DCGM + NVML row took 24 s. **On a GeForce the DCGM diagnostics do no GPU work**, so they are not the self-checking stress suite hoped for [the reason, that a GeForce is outside the SKUs DCGM's diagnostics support, is inferred]; the row records `gpu_use=NA` instead of failing. Whether they run on a data-center die is not tested.

**The whole standalone flow on B2** [run: traces/open_rows_ffdcgm_20261117/vm_rtx3060_ga104_driver_575.51.03/standalone_run_log.txt]: `run_standalone.sh` from a plain `git clone` of the branch took 472 s end to end. `provision.sh` built the probe and installed ffmpeg; its DCGM step failed with `Unable to acquire the dpkg frontend lock`, because a fresh Ubuntu VM was running `unattended-upgrades` (the row therefore reported `dcgm=NOT_RUN reason=dcgmi_not_installed` and everything else ran). `provision.sh` now waits for a busy package manager (at most 20 minutes) before each apt call; after `unattended-upgrades` was stopped on that throwaway VM the DCGM 4.8.0 `cuda12` flavour (the flavour of its 575 driver, CUDA 12.9) installed in 567 s including the wait. The installed DCGM files make `dcgm_root.tar` 2,001,244,160 bytes uncompressed, 195 entries [run: .../bundle_tar_facts.txt].

**DCGM on B2** [run: .../vm_rtx3060_ga104_driver_575.51.03/dcgm.tsv, diag_r3.json]: `dmon` 155 of 768 fields answer; `dcgmi diag -r 3` reports `Deployment/software` as **Fail** (`Persistence Mode: Persistence mode for GPU 0 is disabled`; persistence mode was off on that VM and on in B1's container, where the same test passed) and the same seven `Skip`. A guest run therefore needs persistence mode on (`APPS_GUEST_PM=1` of `apps_hook.sh`) for the software diagnostic to compare like with like; with it off the failure is bucket 3 (our own configuration), not a kayfabe finding.

**The comparison on real data.** B1 against B2 (different die, driver and clock state) was used only to exercise `compare`: it flagged clocks that follow the P-state (210 against 1777 MHz), the persistence and virtualization modes, the supported-clock lists (index-keyed), the device identifier `pdi` and the part number; each was classified (`numeric`, `ignore`, scrubbed) from that run, and a test pins every one. Nothing in this comparison is a statement about kayfabe.

**Not measured:** a kayfabe guest; the unserviced-RPC census (`unserviced_before/after`) and the host GPU-use record need kf3 and were exercised only against fixtures; AV1 encode on a die that has it; DCGM on a data-center die; the Worker path (the apt shim).

## 11. Hypotheses and falsifiers (stated before the run)

Native, now answered:

| hypothesis | falsifier | result |
|---|---|---|
| a Vast container with `NVIDIA_DRIVER_CAPABILITIES=all` gives NVENC/NVDEC | the probe's `NvEncOpenEncodeSessionEx` or `cuvidGetDecoderCaps` fails | FALSIFIED on two hosts (section 10) |
| the expectation derived from the machine's own caps agrees with the attempt for every table case | any MISMATCH, or any FAIL that is not a harness defect | 0 and 0 on B2 after three harness fixes (PSNR threshold, ffmpeg capability wording, AV1 decoder choice); every fix started from a native log |
| DCGM installs from NVIDIA's apt repository with only the key from the official URL | apt cannot resolve the pinned version, or the key URL fails | held (4.8.0, `1:4.8.0-1`) |
| `dcgmi diag -r 3` exercises the GPU on a GeForce | all stress tests skip | FALSIFIED: 7 of 8 skip |
| the GeForce session cap is 8 on a current driver | the probe's session discovery opens another number | held on driver 575.51.03 [one die, one driver] |

For the guest run (not done); a result that contradicts one of these says kayfabe differs from the host:

- **G1** the guest's probe equals the host's (`CAPDIFF` lines of `compare_logs.py`): falsified by any cap id, decoder row or session count that differs.
- **G2** every case that is PASS or NOT_SUPPORTED on the host is the same in the guest: falsified by any `kayfabe_candidate_bug_undocumented` line.
- **G3** with `report-utilization=on` (PR #459) the guest answers `utilization.*` and the encoder and decoder utilisation with numbers: falsified by `FINDING_NA_IN_GUEST` on `utilization.*` or by `host_gpu_use=UNPROVEN` with a readable host counter.
- **G4** the next unserviced fields are named by the rows that stay `FINDING_*` (power, clocks, P-state, thermals are expected there, see section 5); `unserviced_in_row` for `nvml_dcgm` is the census count the row adds to the roughly 1955 lines per boot recorded before PR #459 [read: GPUPV_FEATURE_GAPS note; not run again here].
- **G5** every probe session the guest opens is counted by the HOST's `encoder.stats.sessionCount` (`sess_peak` of the sessions cases equals the host's): falsified by a count that is lower.

## 12. Not done

- (Folded 2026-11-17, section 13: both rows have now run once in a kayfabe guest, on one die, one driver.) The tolerances of `nvml_field_classes.tsv` were never exercised on a live value that moved; every guest expectation not named in section 13 is still [inferred].
- DCGM under the Worker is NOT_RUN by decision (section 3); its bandwidth is therefore not counted for the Worker.
- `dcgm_root.tar` is 2.0 GB uncompressed for the cuda12 flavour [run: traces/open_rows_ffdcgm_20261117/vm_rtx3060_ga104_driver_575.51.03/bundle_tar_facts.txt]; what it adds to `bundle.tgz` is not measured.
- No R2 cache of any fetched artifact exists in this change; if one is added it must be optional, and a value the user passes is trusted as given (owner addendum 2026-11-17).
- Hopper and Blackwell codec tables are not asserted: the matrix derives them; the video-engine document says they were not measured on hardware.

## 13. First guest data (2026-11-17, RTX 3090 GA102, Vast KVM box, host and guest driver 580.159.04, kf3 `04613288`, Ubuntu 22.04 host with ffmpeg 4.4.2, guest ffmpeg a different build)

Evidence: `traces/open_rows_guest_20261117/` (runs R1, R1c, R1p). Commands: the ones of section 8 (`apps_matrix.sh host|guest RUN nvcodec_matrix nvml_dcgm`, `APPS_APP_TMO=3600`, one app per boot), plus `KF_GUEST_IMG` = the apps image, plus a second host run (R1c, the control) and a persistence-mode run (R1p: host `nvidia-smi -pm 1`, guest `APPS_GUEST_PM=1`).

| | host (R1, control R1c identical) | guest (R1) |
|---|---|---|
| `nvcodec_matrix` | pass 35, not_supported 7, fail 0, mismatch 0, not_run 5 (AV1 encode: `unknown_encoder`), 143 s, `gpu_use=PROVEN` | pass 31, not_supported 16, fail 0, mismatch 0, not_run 0, 245 s, `host_gpu_use=PROVEN` (encoder 100 percent in 28 samples) |
| compare | host against host: 47 of 47 agree, 0 capdiffs (noise floor zero) | 38 agree, 9 disagree, 4 candidates, 1 capdiff |
| `nvml_dcgm` | nvml_fields_ok 57, queries 63/74, smi 647/734, dcgm_fields 159/768 | NVML compare: 132 `FINDING_NA_IN_GUEST`, DCGM 74; host against host: 0 |

- **The 4 candidates (`sess_h264_n04`, `n05`, `n06`, `n08`) and the capdiff `sessions.opened host=8 guest=3`: bucket 1**, documented: `kf-qemu encquota::ENC_SESSIONS_PER_VM = 3` (security review S1-42, a design choice in the source comment). Every other probe line (all cap ids of three codecs, every decoder row) is equal. `nvcodec_known_limits.tsv` now says so (it was pinned empty); up to the cap (n01 to n03) guest and host agree.
- **The 5 `enc_av1_*` DISAGREE (host NOT_RUN, guest NOT_SUPPORTED): bucket 3**, our configuration: the host's ffmpeg (Ubuntu 22.04, 4.4.2) has no `av1_nvenc`, the guest's ffmpeg (a different build, sha in the summary header) has one and the attempt fails; both probes say the die has no AV1 encode. The AV1-encode die hole of section 10 is NOT closed (no Ada die in this run).
- **NVML and DCGM findings, 132 and 74: bucket 1 once read against `V3_UTILIZATION_FLAG.md` section 10**, and now MEASURED: clocks (graphics, memory, SM, video, max), power (usage, limits, constraints, energy), temperature and thresholds, fan, perf state, throttle reasons and clock-event counters, PCIe throughput, replay counter and link width, and the `utilization.*` columns are `NOT_SUPPORTED` or `[N/A]` in the guest although the host serves them. The question of section 5, "whether the guest's NVML gets them another way", is answered: it does not. The table `nvml_expected_not_supported.tsv` deliberately does not list them yet; that is the owner's decision (listing them turns the row quiet, and they are the cheapest next telemetry to serve).
- **DCGM diag (H4 held): bucket 3.** With persistence mode off, `Deployment/software` FAILS on the host as well as in the guest (`dcgmi diag` rc 226 after 1 to 2 s); with it on (R1p), it PASSES on both. The other seven level-3 tests are `SKIP` on both sides on this GeForce (no profiling), so the diagnostic does not work the GPU here and its guest-versus-host comparison is a statement about the harness only.
- **kf3 census (H7 not held).** In the row window the guest saw 4193 (nvcodec) and 1083 / 793 (nvml_dcgm) unserviced GSP RPCs, all `fn76` (GSP_RM_CONTROL, answered `0x56` not-supported by kf3); the top commands in the cumulative `gsp_refusals[...]` of the last status line of the nvcodec row (4282 in all, 49 of them before the row) are `0x20802087` (1830), `0x20802a12` (1616), `0x20808165` (303, the NVENC session accounting control of `V3_FEATURE_GAPS.md` L13) and `0x20810108` (86). The applications survived all of them (no row failed). 0 refused, guest Xid 0 and host Xid 0 in these rows. The kf3 log slices of two simple rows run the same day (`nvidia_smi`, `vectorAdd`, apps-stage smoke test) hold 33 and 36 `UNSERVICED` lines in all, so the open rows are one to two orders of magnitude noisier: the census belongs in the row line, as `apps_hook.sh` prints it.


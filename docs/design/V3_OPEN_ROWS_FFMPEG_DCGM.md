# V3 open rows: the FFmpeg NVENC/NVDEC capability matrix and the DCGM + NVML sweep

**STATUS: LIVE, 2026-11-17 (code and GPU-free tests; native results from two Vast container boxes are in section 10; NOT run in a kayfabe guest. Every sentence tagged [inferred] or [not measured] is a hypothesis with a falsifier in section 11).** Owner ruling 2026-11-17 (binding): go OPEN FIRST; an open row works for a standalone third party who trusts git only, from the repository alone, fully automated: no private R2 object, no account, no secret, no licence. A Worker may keep a cache, but nothing in this change reads one.

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
| `nvcodec_known_limits.tsv` | documented kayfabe limitations of the FFmpeg row (empty on purpose) |
| `nvml_sweep.py`, `nvml_lib.py`, `dcgm_nvml.py` | the NVML sweep over ctypes, the row diff and the DCGM parsers, the DCGM runner and its `compare` |
| `nvml_field_classes.tsv`, `nvml_expected_not_supported.tsv` | how each value is compared (static, live with a tolerance, only-a-number); the documented NOT_SUPPORTED table, each line with its reason |
| `census.py`, `rowio.py`, `compare_logs.py` | the host census (GPU use, unserviced-RPC count), the `@@FILE` hand-off of result files through a log, the comparison front end |
| `provision.sh`, `pins.env`, `run_standalone.sh` | fetch and build on one machine (glue, under 100 lines); the pins; the standalone entry |
| `scripts/ci/test_open_rows.py`, `scripts/ci/open_rows_mutants.py` | GPU-free tests (fixtures with the fake UUID prefix `fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e....`) and 70 mutants with two named killers each |

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

**Open decision for the owner (not taken here).** Under the Vast payload `apt-get` is a cache-only shell function that holds no NVIDIA package, and the one sanctioned bypass is in `pin_lib.sh` (`test_apps_pins.py` pins it to exactly one place). `provision.sh` therefore does NOT bypass it: under the payload the DCGM step reports `OPENROWS_dcgm=FAIL reason=apt_shim_cache_only` and the row reports `dcgm=NOT_RUN reason=dcgmi_not_installed`, while the NVML sweep, the nvidia-smi sweep and the FFmpeg row run. Letting Worker runs install DCGM needs a third bypass call in `pin_lib.sh` and the test's expected count changed; that is the owner's call.

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

`scripts/ci/test_open_rows.py` (about 190 tests): table validity and its tamper cases; the probe parser; the caps comparison on made-up Ampere-like and Ada-like probes; the NOT_SUPPORTED, MISMATCH and NOT_RUN handling; PSNR threshold boundaries; command construction (argument vectors, never shells); ffmpeg error classification; the host/guest buckets; the NVML diff (every status, tolerances at their boundaries); the DCGM parsers on both JSON layouts; the census with known positives; the `@@FILE` hand-off with corruption; the sweep against a fake `libnvidia-ml` built from C in the test; the FFmpeg runner end to end against a fake `ffmpeg` and a fake probe; the DCGM runner end to end against a fake `dcgmi`; and text pins for the trust rules and the wiring (marked as pins: they prove the line is there, not that it works). `scripts/ci/open_rows_mutants.py` runs 70 mutants, each killed by at least two named tests; it is part of the `scripts/ci` step of `ci.yml`.

## 10. Native results

[RESULTS-PENDING]

## 11. Hypotheses and falsifiers (stated before the run)

[FALSIFIERS-PENDING]

## 12. Not done

- No kayfabe guest has run either row; the tolerances of `nvml_field_classes.tsv` and every guest expectation are [inferred].
- The apt bypass for Worker runs (section 3) and the DCGM bandwidth under the Worker are open.
- `dcgm_root.tar` makes the bundle larger by the size of DCGM [not measured in the bundle].
- No R2 cache of any fetched artifact exists in this change; if one is added it must be optional, and a value the user passes is trusted as given (owner addendum 2026-11-17).
- Hopper and Blackwell codec tables are not asserted: the matrix derives them; the video-engine document says they were not measured on hardware.

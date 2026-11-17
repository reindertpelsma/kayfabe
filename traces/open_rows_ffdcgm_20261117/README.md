# Open FFmpeg NVENC/NVDEC matrix and DCGM + NVML sweep: native runs on two Vast boxes, 2026-11-17

STATUS: LIVE, 2026-11-17 (evidence of native runs, no kayfabe guest). Revision under test: `da4fa3b3e` of branch `claude/open-ffmpeg-dcgm-20261117`
(PR against `next`); the design is `docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md`, section 10 reads these files. The files are text, scrubbed by
`scripts/apps/open_rows/census.py` (GPU UUIDs, serials, bus ids, addresses; the device identifier `pdi` and the part number are `<scrubbed>`).
The two earlier native runs, at older revisions, found the defects fixed since (PSNR threshold, ffmpeg capability wording, the AV1 decoder choice,
the DCGM CUDA flavour, the `dcgmi dmon` column format) and are not kept.

| directory | machine | what ran |
|---|---|---|
| `vm_rtx3060_ga104_driver_575.51.03/` | Vast KVM VM, RTX 3060 (GA104) passed through, Ubuntu 22.04, driver 575.51.03, ffmpeg 4.4.2 of the distribution | `nvcodec_matrix.py run` (159 s) and `nvml_sweep.py` |
| `container_rtx3060_ga106_driver_580.126.20/` | Vast container box (no KVM), RTX 3060 (GA106), Ubuntu 24.04 image, driver 580.126.20, ffmpeg 6.1.1, DCGM 4.8.0 | `dcgm_nvml.py run` (24 s) and `nvcodec_matrix.py` (every case NOT_RUN: this container gives no NVENC/NVDEC) |

Files: `probe.tsv` (what `nvcodec_probe` read from the driver), `results.tsv` and `summary.txt` of the FFmpeg row, `nvml_sweep.tsv` (every NVML field id the
driver knows, 74 typed queries, `nvidia-smi -q -x` leaves, every `--query-gpu` property), `dcgm.tsv`, `dcgm_status.txt`, `diag_r3.json` (the raw
`dcgmi diag -r 3 -j`), `nvvs_test_lines.txt` (why DCGM skipped each test), `dmon_list_head.txt`, `matrix_in_container.txt`, `selected_logs.txt` (ffmpeg's own
error text of five cases), `environment.txt`.

What the files say (each line is in the file named):

- `vm_*/summary.txt`: `pass=35 not_supported=7 fail=0 mismatch=0 not_run=5 cases=47`; the five NOT_RUN are the AV1 encode cases (`results.tsv`, reason `unknown_encoder`: the die has no AV1 encode and ffmpeg 4.4.2 has no `av1_nvenc`).
- `vm_*/probe.tsv`: `enc_codec av1 0`, `sessions opened 8`, `stopped_by NV_ENC_ERR_INCOMPATIBLE_CLIENT_KEY`, H.264 `WIDTH_MAX 4096`, HEVC `WIDTH_MAX 8192`.
- `container_*/matrix_in_container.txt` and `container_*/probe.tsv`: `open session NV_ENC_ERR_UNSUPPORTED_DEVICE`, and `cuvidGetDecoderCaps` answers `ERR:cuda100` for every codec.
- `container_*/dcgm.tsv`: 1 `PASS` and 7 `SKIP` diagnostics; `nvvs_test_lines.txt`: "The memory test is skipped. Check DCGM and system configuration."
- `container_*/nvml_sweep.tsv`: 56 NVML fields answer a number, 231 `NOT_SUPPORTED`; the twelve typed queries refused by the host itself are the ECC, MIG, applications-clock, `temperature.threshold.2` and NvLink ones.

Cost of the rentals, from `python3 /root/night/vast_bill.py` on the coordinator, is in the PR description (it was not final when this file was written). Boxes were untrusted: no credential, token or key was copied to them, the repository came by `git clone` of the pushed branch, and nothing but these text files came back. The kf3 guest was not run.

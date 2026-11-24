# Run the open FFmpeg and DCGM rows

Two Linux checks look at the GPU's video engines and at its monitoring interface, on any NVIDIA machine, from a plain `git clone`. They need no account, no licence and no private download. Run them on the real GPU first, then in a kayfabe guest, and compare the two.

- **FFmpeg NVENC/NVDEC matrix** (`nvcodec_matrix`): about 47 encode, decode and parallel-session cases (AV1, HEVC Main10, 4:4:4, above 4096 and 8K, B-frames and lookahead, NVDEC through `-hwaccel cuda` and through `cuvid`). What each case should do comes from the machine's own answer (`NvEncGetEncodeCaps`, `cuvidGetDecoderCaps`, how many encode sessions the driver opens), so a GPU without AV1 encode reports `NOT_SUPPORTED`, not a failure.
- **DCGM + NVML sweep** (`nvml_dcgm`): every NVML field value id the driver knows, the common NVML queries, `nvidia-smi -q -x`, every `nvidia-smi --query-gpu` property, and, when DCGM is installed, `dcgmi dmon` and `dcgmi diag`.

## Requirements

Linux with an NVIDIA driver, `python3` (3.9 or newer), root, and the network once. The first step installs `gcc` and `git` for the small capability probe, `ffmpeg` from your distribution, and DCGM from NVIDIA's apt repository (the repository key is fetched from NVIDIA's own URL and apt verifies the signed index). If any step fails, its row says `NOT_RUN` with the reason and the other row still runs.

## On one machine

```text
sudo bash scripts/apps/open_rows/run_standalone.sh /tmp/open_rows_out
```

It prints one `NVCODEC_MATRIX_SUMMARY` and one `NVML_DCGM_SUMMARY` line and keeps the details in `/tmp/open_rows_out/`: `results.tsv` (one line per case), `probe.tsv` (what the driver reported), `nvml_sweep.tsv`, `dcgm.tsv`, `dcgm_status.txt` and a `logs/` directory. Nothing in these files carries a GPU UUID, a serial number or an IP address.

To run one row alone: `python3 scripts/apps/open_rows/nvcodec_matrix.py run --out DIR` or `python3 scripts/apps/open_rows/dcgm_nvml.py run --out DIR` (add `--only 'enc_hevc_*'` to select cases; `--ffmpeg PATH` uses your own ffmpeg, whose digest is recorded).

## Host against guest

Run the rows on the host, then in the guest, and compare the two logs (the rows print their result files between `@@FILE` markers, so a log is enough):

```text
bash scripts/apps/apps_matrix.sh host  RUN nvcodec_matrix nvml_dcgm
bash scripts/apps/apps_matrix.sh guest RUN nvcodec_matrix nvml_dcgm
```

`apps_matrix.sh` runs the comparison at the end of the guest run; by hand: `python3 scripts/apps/open_rows/compare_logs.py nvcodec_matrix HOST_LOG GUEST_LOG` (or `nvml_dcgm`). A case that works on the host and fails in the guest, and a number the host serves that the guest answers with `N/A`, are listed as candidates. Every failure belongs to one of three buckets: a documented limitation (`nvcodec_known_limits.tsv`, `nvml_expected_not_supported.tsv`), a fault of the application or of the harness (the host fails the same way), or a configuration error of your own (the row is `NOT_RUN`, with the reason).

The DCGM diagnostic at level 3 can run for twenty minutes or more; give the guest run a long per-app time limit (`APPS_APP_TMO=3600`).

# Open ML rows (vllm, tensorrt, jax): native runs on three Vast container boxes

STATUS: LIVE, 2026-11-17 (evidence of `docs/design/V3_OPEN_ML_ROWS.md` section 8). Text only: the logs of `scripts/apps/mlrows/mlrows.py run`, the setup state, and the box facts. No GPU UUID, serial number or address is in any file (`scripts/ci/no_gpu_uuids.py` and `no_private_refs.py` pass over this directory). No kayfabe was in the path: bare metal first, Vast CONTAINER boxes (image `nvidia/cuda:13.0.1-devel-ubuntu24.04`, CPython 3.12.3, no KVM), one GPU each, nothing but a public key on a box, code by `git clone` of the branch `claude/open-rows-batch2-20261117`.

Revision under test: `f4df2f99f` (the head when the final runs started; nothing under `scripts/apps/` has changed since, later commits touch tests, docs and this directory). The first runs, from which the thresholds and four defects came (design doc section 8), were at older revisions of the same branch and are not kept.

| directory | die | driver (CUDA) | what is in it |
|---|---|---|---|
| `rtx3060_ga106_sm86_driver_580.159.03/` | RTX 3060, 12 GB, GA106, sm_86 | 580.159.03 (13.0, the lowest the cu13 flavour takes) | the three rows (`jax.log`, `tensorrt.log`, `vllm.log`), their CPU forms (`negctl_*.log`), the CUDA 12 flavours of JAX and TensorRT forced on this CUDA 13 driver (`cu12_*.log`), `run_apps_host.txt` (the same rows through `run_apps.sh host` and `gpu_use_rows.py annotate-res`, then their negative controls), `setup_log.txt` and the two `setup_state*.json.txt` (setup seconds and bytes at an older revision, two setups at a time on an 870 Mbit offer) |
| `rtx4060ti_ad106_sm89_driver_580.178.04/` | RTX 4060 Ti, 16 GB, AD106, sm_89 | 580.178.04 (13.0) | the rows, the CPU forms, `standalone_pipeline.log` (the whole `run_standalone.sh` flow from a plain clone: setup 4 min 14 s for vLLM, 71 s TensorRT, 70 s JAX on a gigabit link, then the three rows), `setup_state.json.txt` |
| `rtx2080ti_tu102_sm75_driver_615.71.09/` | RTX 2080 Ti, 11 GB, TU102, sm_75 (Turing, the floor) | 615.71.09 (13.4) | the same as the Ada directory |

Each `<row>.log` is the stdout of `mlrows.py run <row>` (the row's own first 60 and last 100 log lines prefixed `| `, `MLROW_JSON` the measurements, `MLROWS_CHECK` one line per check, `GPU_USE_PROOF`, `<ROW>_SUMMARY`, `MLROWS_RESULT` the judged line). The tqdm progress of CUDA graph capture is thinned to its 100 percent line. `pipeline.log` has the exit code and time of each run: `RUN_EXIT <row> rc=0` is a PASS, `NEGCTL_EXIT ... rc=1` a FAIL by design (the CPU form). `scripts/ci/fixtures/mlrows/*_rtx4060ti_580.log` are copies of the Ada logs of the first full pipeline (revision `5d9c6dbaf`) that the GPU-free tests read.

Reproduce one die: `sudo bash scripts/apps/mlrows/run_standalone.sh /tmp/out` on a CUDA 13 machine, then `KF_NEGCTL=1 python3 scripts/apps/mlrows/mlrows.py run <row> --root /tmp/out/root`.

Nothing here is a kayfabe result: the guest runs are the validation request of the design doc, section 7.

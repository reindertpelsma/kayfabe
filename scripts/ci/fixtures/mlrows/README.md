# Fixtures of the open ML rows (scripts/ci/test_mlrows.py)

STATUS: LIVE, 2026-11-17. Which file is real and which is synthetic.

REAL (captured 2026-11-17, text only, no GPU UUID, no address):

- `hf_tree_smollm2_135m.json`: the answer of `GET https://huggingface.co/api/models/HuggingFaceTB/SmolLM2-135M/tree/93efa2f097d58c2a74874c7e644dbc9b0cee75a2` (keys sorted). The test of the model plan reads it.
- `nvidia_smi_head_580_rtx3060.txt`, `query_gpu_580_rtx3060.csv`: the header of `nvidia-smi` and `nvidia-smi --query-gpu=name,driver_version,compute_cap,memory.total --format=csv,noheader,nounits` of the first native box (RTX 3060, driver 580.159.03, CUDA 13.0); the bus id of the table row is zeroed.
- `*_run_*.log`: the stdout of `mlrows.py run <row>` of the native runs (the `MLROW_JSON`, `MLROWS_CHECK`, `GPU_USE_PROOF`, `<ROW>_SUMMARY` and `MLROWS_RESULT` lines), which the tests feed to the judges, the comparison and the GPU-use judge. They are the same files as in `traces/open_ml_rows_20261117/`, the README there names the run, revision and box.

SYNTHETIC: nothing in this directory; the tests build their other inputs in code (and the fake UUID prefix `fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e....` is the only one a fixture may hold).

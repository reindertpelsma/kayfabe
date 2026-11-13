"""All tests. Specs are declared in the workload modules (torch-free at import) and listed here."""
from __future__ import annotations

import importlib
import json
from pathlib import Path

NAMES = ("nanogpt_compile", "triton_kernels", "cuda_ext_inline", "amp_convnet_cifar", "stress_alloc",
         "managed_oversub")
TIMINGS_FILE = Path(__file__).resolve().parent.parent / "timings.json"


def all_specs():
    return [importlib.import_module("kfml.workloads." + n).SPEC for n in NAMES]


def load_timings(path=None) -> dict:
    try:
        with open(path or TIMINGS_FILE) as f:
            return {k: v for k, v in json.load(f).items() if v is not None}
    except FileNotFoundError:
        return {}

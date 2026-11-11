"""Context handed to a workload. Stdlib only."""
from __future__ import annotations

import time


class Ctx:
    def __init__(self, d: dict):
        self.name = d["name"]
        self.cc = tuple(d["cc"]) if d.get("cc") else None
        self.vram_total_gb = d.get("vram_total_gb")
        self.vram_free_gb = d.get("vram_free_gb")
        self.data_dir = d["data_dir"]
        self.reference = d.get("reference")  # dict from --reference or None
        self.budget_s = float(d["budget_s"])
        self.seed = int(d.get("seed", 1234))
        self.upstream_paths = d.get("upstream_paths", {})
        self.dev_cpu = bool(d.get("dev_cpu"))  # KF_ML_DEV_CPU=1: smoke-run logic on CPU (author debugging only)
        self.guest = bool(d.get("guest"))
        self.t0 = time.time()

    @property
    def device(self):
        return "cpu" if self.dev_cpu else "cuda"

    def time_left(self) -> float:
        return self.budget_s * 0.8 - (time.time() - self.t0)  # soft deadline: 80% of the hard budget

    def expired(self) -> bool:
        return self.time_left() <= 0

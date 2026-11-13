"""Test and upstream declarations. Pure data, no torch import."""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Callable, Optional, Tuple

SHA40 = re.compile(r"^[0-9a-f]{40}$")
TIERS = ("mid", "large")
LAUNCH_BOUND_FACTOR = 4.0
LARGE_KERNEL_FACTOR = 1.5
MID_TEST_LIMIT_S = 480.0
MID_TOTAL_LIMIT_S = 900.0


@dataclass(frozen=True)
class Upstream:
    """One upstream code source. commit is always a full 40-hex SHA, never a branch or tag.

    role: 'vendored' (copied into vendor/<vendor_dir>, MIT/BSD/Apache only),
          'fetched' (cloned at the pin into a fresh temp dir, patch applied),
          'derived' (we wrote our own code following it; credit only).
    """

    name: str
    repo_url: str
    commit: str
    license: str
    role: str
    patch: Optional[str] = None
    vendor_dir: Optional[str] = None

    def __post_init__(self):
        if not SHA40.match(self.commit):
            raise ValueError("upstream %s: commit must be a full 40-hex SHA, got %r" % (self.name, self.commit))
        if self.role not in ("vendored", "fetched", "derived"):
            raise ValueError("upstream %s: bad role %r" % (self.name, self.role))
        if self.role == "vendored":
            if not self.vendor_dir:
                raise ValueError("upstream %s: vendored needs vendor_dir" % self.name)
            if self.license not in ("MIT", "BSD-2-Clause", "BSD-3-Clause", "Apache-2.0"):
                raise ValueError("upstream %s: only MIT/BSD/Apache-2.0 may be vendored" % self.name)

    @property
    def vendored(self) -> bool:
        return self.role == "vendored"


@dataclass(frozen=True)
class Spec:
    name: str
    module: str
    description: str
    tier: str  # mid | large
    budget_s: float  # hard wall-clock budget of the test process (includes first-run downloads)
    launch_bound: bool = False  # many small kernel launches (eager small-batch training)
    min_vram_gb: float = 8.0  # TOTAL vram; documented floor is 8
    min_cc: Tuple[int, int] = (7, 5)
    max_cc: Optional[Tuple[int, int]] = None
    min_torch: str = "2.7"
    needs_nvcc: bool = False
    needs_triton: bool = False
    needs_compiler: bool = False  # C compiler + Python.h (triton launcher, inductor)
    datasets: Tuple[str, ...] = ()
    upstreams: Tuple[Upstream, ...] = ()
    gate: bool = True  # False: only runs when named with --only or --all
    download_note: str = ""  # the first run downloads several GB of CUDA wheels / datasets
    precheck: Optional[Callable] = field(default=None, compare=False)  # (env, environ) -> reason|None

    def __post_init__(self):
        if self.tier not in TIERS:
            raise ValueError("%s: bad tier %r" % (self.name, self.tier))
        if self.min_vram_gb < 8.0:
            raise ValueError("%s: documented floor is 8 GB total" % self.name)
        if self.budget_s <= 0:
            raise ValueError("%s: budget must be positive" % self.name)


def guest_estimate_s(spec: Spec, timings: dict):
    """host_runtime_s * (4.0 if launch_bound else 1.5); None until a host runtime is measured."""
    host = (timings or {}).get(spec.name)
    if host is None:
        return None
    return float(host) * (LAUNCH_BOUND_FACTOR if spec.launch_bound else LARGE_KERNEL_FACTOR)


def tier_problems(specs, timings: dict):
    """Rule: mid only if guest_estimate_s <= 480; the whole mid set stays <= 900 s."""
    problems = []
    total = 0.0
    for s in specs:
        est = guest_estimate_s(s, timings)
        if s.tier == "mid" and est is not None:
            total += est
            if est > MID_TEST_LIMIT_S:
                problems.append("%s: tier=mid but guest_estimate_s=%.0f > %.0f" % (s.name, est, MID_TEST_LIMIT_S))
    if total > MID_TOTAL_LIMIT_S:
        problems.append("mid set total %.0f s > %.0f s" % (total, MID_TOTAL_LIMIT_S))
    return problems

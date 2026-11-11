"""Helpers shared by workloads. torch is imported lazily inside functions."""
from __future__ import annotations

from ..errors import CheckFailed


def check(cond, msg):
    if not cond:
        raise CheckFailed(msg)


def amp_dtype(torch, cc):
    """bf16 on Ampere+ (cc >= 8.0); fp16 (with GradScaler) on Turing. is_bf16_supported() reports
    emulated bf16 on older cards in recent torch, so the capability is decided by cc here."""
    if cc is None or tuple(cc) >= (8, 0):
        return torch.bfloat16
    return torch.float16


def sync(torch, device):
    if device == "cuda":
        torch.cuda.synchronize()


def seed_all(torch, seed, device):
    import random
    random.seed(seed)
    torch.manual_seed(seed)
    if device == "cuda":
        torch.cuda.manual_seed_all(seed)


def rel_err(torch, a, b):
    """max|a-b| / (max|b| + tiny), in fp32/fp64."""
    a, b = a.double(), b.double()
    return float((a - b).abs().max() / (b.abs().max() + 1e-12))


def free_total_gb(torch):
    free, total = torch.cuda.mem_get_info()
    return free / 2**30, total / 2**30

"""VRAM guard: scale problem size from free VRAM. Documented floor: 8 GB total."""
from __future__ import annotations

FLOOR_GB = 8.0


def scale_factor(free_gb: float, base_gb: float, max_scale: float = 8.0, headroom: float = 0.5) -> float:
    """How many times base_gb of working set fits in headroom*free_gb; clamped to [1, max_scale].

    Never below 1.0: a card at the floor runs the base size. Callers that cannot fit even
    base_gb must have been gated out by min_vram_gb.
    """
    if free_gb <= 0 or base_gb <= 0:
        return 1.0
    s = (free_gb * headroom) / base_gb
    return max(1.0, min(float(max_scale), s))


def pow2_floor(n: int) -> int:
    n = max(1, int(n))
    return 1 << (n.bit_length() - 1)

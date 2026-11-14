"""Choose a benchmark store while reserving at least two GiB for the host."""


def framebuffer_mib(vram_mib):
    if type(vram_mib) not in (int, float) or vram_mib < 6144:
        raise ValueError("adapter cannot fit the minimum store and host headroom")
    return 8192 if vram_mib >= 10240 else 4096

#!/usr/bin/env python3
"""K1 production-launcher configuration and requested arm abort rules."""
from pathlib import Path
import ctypes
import subprocess


def cuda_status():
    """Check the native CUDA driver before any arm is launched."""
    return ctypes.CDLL('libcuda.so.1').cuInit(0)


def reload_nvidia(run=subprocess.run):
    """One owner-authorized module reload, with GDM restoration on failure."""
    try:
        run(['systemctl', 'stop', 'gdm'], check=True)
        run(['rmmod', 'nvidia_uvm', 'nvidia_drm', 'nvidia_modeset', 'nvidia'], check=True)
        run(['modprobe', '-a', 'nvidia', 'nvidia_uvm', 'nvidia_modeset', 'nvidia_drm'], check=True)
    finally:
        run(['systemctl', 'start', 'gdm'], check=True)


def ensure_cuda(probe=cuda_status, reload=reload_nvidia):
    """Require cuInit success; only error 999 permits one reload and recheck."""
    before = probe()
    after = before
    if before == 999:
        reload()
        after = probe()
    if after != 0:
        raise RuntimeError(f'CUDA_HEALTH_REFUSED before={before} after={after}')
    return {'before': before, 'after': after, 'reloaded': before == 999}


def production_environment(owner, root, rev, maximum_seconds):
    root = Path(root)
    if not root.is_absolute() or not str(root).startswith('/data/codex-K1/'):
        raise ValueError('K1 state must be lane local')
    flags = {k: v for k, v in owner['flags'].items() if k != 'KF3_PROF'}
    flags['KF3_DIAG_PROGRESS'] = '1000'
    return {'WIN_DIR': str(root), 'KF3_REV': rev, 'WIN_REUSE': '1',
            'WIN_FLAGS': ' '.join(f'{k}={v}' for k, v in flags.items()),
            'WIN_DROP': 'KF3_TIMER_MAP', 'WIN_FB_MB': '8192',
            'WIN_RAM_MB': '12288', 'WIN_BAR1': '8589934592',
            'WIN_TRACE': '0', 'WIN_GSP_OBSERVER': '0',
            'WIN_MAX_SECONDS': str(maximum_seconds),
            'BROKER_BIN': '/data/claude-cursor6/nvkvm-display-broker',
            'PROD_EXTRA_FLAGS': ''}


def abort_reason(xid_count, reset_times, now, deadline):
    if xid_count:
        return 'NEW_HOST_XID'
    if sum(now - 900 <= t <= now for t in reset_times) > 3:
        return 'TDR_CYCLE_BUDGET'
    if now >= deadline:
        return 'RESTORE_DEADLINE'
    return None

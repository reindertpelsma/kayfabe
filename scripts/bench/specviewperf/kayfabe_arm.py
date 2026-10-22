#!/usr/bin/env python3
"""K1 production-launcher configuration and requested arm abort rules."""
from pathlib import Path


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

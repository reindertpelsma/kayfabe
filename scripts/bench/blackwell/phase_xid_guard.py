#!/usr/bin/env python3
"""Fail-closed campaign guard seam: source-proved native controls, then frozen snapshots."""
import importlib.util
from pathlib import Path

SPEC = importlib.util.spec_from_file_location('native_xid_guard', Path(__file__).resolve().parents[1] / 'native_xid_guard.py')
N = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(N)


def native_baseline(journal, defer, missing):
    return N.approve_native(journal, defer, missing)


def evaluate_snapshot(journal, accepted, phase_lines, arm, pid, init_utc):
    N.check_unchanged(journal, accepted)
    return N.xid_lines(journal)

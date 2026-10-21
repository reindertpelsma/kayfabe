#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Sweep guard seam: existing absolute-zero and count-only checks, before repair.

This extraction preserves the controller's previous decisions so failing-first
regressions can exercise the actual old checks before owner decision 1 is applied.
It is not deployed until the subsequent tested correction.
"""


class GuardRefused(ValueError):
    """The native proof or later kernel snapshot is not accepted."""


def xid_lines(journal):
    return tuple(line for line in journal.splitlines() if "NVRM: Xid" in line)


def approve_native(journal, defer_log, missing_log):
    lines = xid_lines(journal)
    if lines:
        raise GuardRefused("NATIVE_NEW_XID_STOP")
    return lines


def check_unchanged(journal, baseline):
    if len(xid_lines(journal)) != len(baseline):
        raise GuardRefused("GUEST_NEW_XID_STOP")

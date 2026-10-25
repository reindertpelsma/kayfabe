#!/usr/bin/env python3
"""Regression first recorded RED in HM1 commit 883c413d3.

The deployed continuation controller delegates exit status to this tested policy.
Candidate 6564dd597, kf3-bins/6564dd59/qemu-system-x86_64; no hardware access.
"""
from fault_admission import exit_code


def test_host_fault_is_distinct_from_timeout():
    assert exit_code("host-fault", 0) == 3
    assert exit_code("audit-error", 0) == 3
    assert exit_code("budget", 0) == 124


if __name__ == "__main__":
    test_host_fault_is_distinct_from_timeout()

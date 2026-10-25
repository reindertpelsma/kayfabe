#!/usr/bin/env python3
"""EXPECTED RED: a host-fault stop must not become GNU timeout's exit status.

Replay the final exit expression of the private HM1 controller without starting a VM.
Usage: python3 test_stop_reason_expected_red.py /path/to/qmp_runner.py
Evidence: HM1, candidate 6564dd597, kf3-bins/6564dd59/qemu-system-x86_64.
"""
import ast
from pathlib import Path
import sys


def test_host_fault_is_distinct_from_timeout(path: Path) -> None:
    tree = ast.parse(path.read_text())
    final = tree.body[-1]
    assert isinstance(final, ast.Expr)
    call = final.value
    assert isinstance(call, ast.Call)
    assert isinstance(call.func, ast.Attribute) and call.func.attr == "exit"
    expr = call.args[0]
    assert isinstance(expr, ast.IfExp)
    assert isinstance(expr.test, ast.Name) and expr.test.id == "stop_reason"
    # Replay the truthy recorded reason, "host-fault", at wall_seconds < budget.
    host_fault_exit = ast.literal_eval(expr.body)
    assert host_fault_exit != 124, (
        "HOST_FAULT is encoded as 124; the original fast-suite parser reports TIMEOUT. "
        "Preserve a distinct fault reason before another campaign."
    )


if __name__ == "__main__":
    test_host_fault_is_distinct_from_timeout(Path(sys.argv[1]))

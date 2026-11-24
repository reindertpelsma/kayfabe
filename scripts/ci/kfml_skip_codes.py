#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The reason codes a kfml NOT_RUN row may carry to count as a permitted skip under a strict payload instruction.

The vocabulary lives in one place, scripts/kfml/kfml/errors.py (SKIP_CODES), because the rows that emit a code are the row runner's. This module
loads that tuple by file path (kfml's package is not imported, so nothing of the workloads runs) for kfml_stage.py, which keeps a code on a row
only when it is in the tuple, and for vast_payload.py, which grades a strict run by membership in it. Nothing here parses a free-text reason.
"""
import importlib.util
from pathlib import Path

ERRORS = Path(__file__).resolve().parents[1] / "kfml" / "kfml" / "errors.py"


def load(path=ERRORS):
    spec = importlib.util.spec_from_file_location("kfml_row_errors", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    codes = module.SKIP_CODES
    if type(codes) is not tuple or not codes or not all(type(c) is str and c for c in codes):
        raise ValueError("kfml errors.SKIP_CODES is not a non-empty tuple of strings")
    return codes


SKIP_CODES = load()


def permitted(row):
    """True for a NOT_RUN row whose `skip_code` is exactly one of the vocabulary's codes."""
    return type(row) is dict and row.get("status") == "NOT_RUN" and type(row.get("skip_code")) is str and row["skip_code"] in SKIP_CODES

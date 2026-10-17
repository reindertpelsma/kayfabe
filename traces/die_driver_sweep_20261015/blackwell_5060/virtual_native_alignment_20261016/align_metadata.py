#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Evidence entry point for the complete, tested metadata aligner.

The previous500-selected-call implementation is in git at577f15a7e;
aligned-api.tsv retains its original limited result. New full output must be
written separately. No reply-body equality or object identity is inferred.
"""
from pathlib import Path
import runpy

runpy.run_path(str(Path(__file__).resolve().parents[4] / 'tools' / 'align_metadata.py'),
               run_name='__main__')

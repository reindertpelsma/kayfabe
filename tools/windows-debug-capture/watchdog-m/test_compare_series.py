#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Synthetic sequence tests for labelled, incremental journal comparisons."""
import importlib.util
from pathlib import Path
p=Path(__file__).with_name('compare-series.py')
s=importlib.util.spec_from_file_location('series',p)
m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
changed=m.delta([1,2,3],[1,4,3])
assert changed['identical_prefix']==1 and changed['identical_suffix_after_prefix']==1
assert changed['previous_changed']==[2] and changed['current_changed']==[4]
assert m.delta([1,2],[1,2])['current_changed']==[]
assert m.delta([1,2],[1,2,3])['current_changed']==[3]
assert m.delta([1,2,3],[1,2])['previous_changed']==[3]
assert m.delta([],[])['identical_prefix']==0
assert m.delta([1,2],[3,4])['identical_suffix_after_prefix']==0
print('Incremental journal prefix/suffix, equality, extra-tail and empty tests passed')

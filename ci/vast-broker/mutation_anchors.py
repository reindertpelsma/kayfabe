"""Exact formatted replacements, mechanically derived from the original mutants."""

import json
from pathlib import Path

MIGRATIONS = json.loads(Path(__file__).with_name("mutation-anchors.json").read_text())


def apply_mutation(campaign, name, source):
    for before, after in MIGRATIONS[campaign + ":" + name]:
        assert source.count(before) == 1, ("Missing/ambiguous formatted anchor", name)
        source = source.replace(before, after, 1)
    return source

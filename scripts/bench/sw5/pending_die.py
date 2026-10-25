"""Admit only a target die that this sweep still needs to qualify."""

import json
from pathlib import Path
import sys

TARGETS = {"GA102", "GA106", "TU106", "TU116", "AD103", "AD106"}


def require_pending(binding, completed):
    die = binding.get("die")
    if die not in TARGETS:
        raise ValueError("actual die is outside the pending SW5 target set")
    if die in completed:
        raise ValueError("actual die already qualified in this sweep")
    return binding


if __name__ == "__main__":
    binding = json.loads(Path(sys.argv[1]).read_text())
    completed = json.loads(Path(sys.argv[2]).read_text())
    require_pending(binding, completed)

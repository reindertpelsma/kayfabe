#!/usr/bin/env python3
"""Trusted-path bootstrap, also under python3 -I (no site/PYTHONPATH dependency)."""

from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from kfci.cli import main

if __name__ == "__main__":
    sys.exit(main())

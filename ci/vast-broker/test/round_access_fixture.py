"""Capture the runner's optional Access A headers locally; all transport is fake.

The Worker injects no Access credential into boxes (owner ruling 2026-10-23 19:10)."""

import importlib.util
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[3]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


case = sys.argv[1]
if case == "runner":
    m = load("runner", root / "ci/vast-broker/runner.py")
    pair = {"CF_ACCESS_CLIENT_ID": "runner-a.access", "CF_ACCESS_CLIENT_SECRET": "a" * 64}
    assert m.access_headers(pair) == {
        "CF-Access-Client-Id": "runner-a.access",
        "CF-Access-Client-Secret": "a" * 64,
    }
    assert m.access_headers({}) == {}
    for key in pair:
        try:
            m.access_headers({key: pair[key]})
        except m.Invalid:
            pass
        else:
            raise AssertionError("partial Access pair admitted")
else:
    raise SystemExit("unknown case")

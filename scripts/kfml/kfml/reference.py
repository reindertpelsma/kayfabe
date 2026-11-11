"""Recorded native-host references: <dir>/<test>.json written by --record-reference."""
from __future__ import annotations

import json
import os
from pathlib import Path


def write(dirpath, name, env, data: dict):
    Path(dirpath).mkdir(parents=True, exist_ok=True)
    doc = {"test": name, "gpu_name": env.gpu_name, "compute_cap": "%d.%d" % tuple(env.cc) if env.cc else None,
           "torch": env.torch, "cuda": env.cuda, "driver": env.driver, "data": data}
    p = Path(dirpath) / (name + ".json")
    tmp = str(p) + ".tmp"
    with open(tmp, "w") as f:
        json.dump(doc, f, indent=1, sort_keys=True)
    os.replace(tmp, p)
    return str(p)


def load(dirpath, name):
    if not dirpath:
        return None
    p = Path(dirpath) / (name + ".json")
    if not p.is_file():
        return None
    with open(p) as f:
        return json.load(f).get("data")

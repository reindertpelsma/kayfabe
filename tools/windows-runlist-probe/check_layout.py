#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Check the OGKM-sourced layout numbers of crates/kf-abi/src/sw_runlist.rs against public OGKM.

STATUS: LIVE, 2026-10-31. This replaces the old generator (`derive.py`), which also read a closed
Windows driver; that part is gone. This tool reads ONLY the public OGKM 580.65.06 checkout and the
Rust table: it compiles `layout.c` (NV_SWRUNLIST_ALLOCATION_PARAMS) and `rpc-layout.c`
(rpc_alloc_memory_v13_01) with the host C compiler and fails if the numbers in the table differ.
Nothing is executed from, or read out of, any closed binary.

usage: check_layout.py --ogkm /path/to/open-gpu-kernel-modules@580.65.06
"""
import argparse
import importlib.util
from pathlib import Path
import re
import subprocess
import sys
import tempfile

TAG_COMMIT = "307159f2623d3bf45feb9177bd2da52ffbc5ddf9"
HERE = Path(__file__).resolve().parent
TABLE = HERE.parents[1] / "crates/kf-abi/src/sw_runlist.rs"


def rust_table():
    text = TABLE.read_text().split("#[cfg(test)]")[0]
    one = lambda name: int(re.search(rf"^\s*{name}: (\d+),", text, re.M).group(1))
    fields = re.findall(r'\("(\w+)", (\d+), (\d+)\)', text)
    return (
        (one("size"), one("engine"), one("max_tsgs"), one("qos")),
        one("alloc_memory_size"),
        [(n, int(o), int(w)) for n, o, w in fields],
    )


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--ogkm", type=Path, required=True)
    args = p.parse_args()
    commit = subprocess.check_output(["git", "-C", str(args.ogkm), "rev-parse", "HEAD"], text=True).strip()
    if commit != TAG_COMMIT:
        raise SystemExit("requires the exact public OGKM 580.65.06 checkout")
    spec = importlib.util.spec_from_file_location("dm", HERE.parent / "drivermatrix/dm.py")
    dm = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(dm)
    want_params, want_rpc_size, want_fields = rust_table()
    with tempfile.TemporaryDirectory(prefix="kf-runlist-layout-") as tmp:
        exe = Path(tmp) / "layout"
        subprocess.run(["cc", "-std=c11", "-Werror", "-I" + str(args.ogkm / "src/common/sdk/nvidia/inc"),
                        str(HERE / "layout.c"), "-o", str(exe)], check=True)
        got_params = tuple(map(int, subprocess.check_output([str(exe)], text=True).split()))
        rpc = Path(tmp) / "rpc-layout"
        subprocess.run(["cc", "-std=c11", *dm.env_flags(str(args.ogkm), "rm"),
                        str(HERE / "rpc-layout.c"), "-o", str(rpc)], check=True)
        rows = subprocess.check_output([str(rpc)], text=True).splitlines()
    got_rpc_size = int(rows[0].split()[1])
    got_fields = [(n, int(o), int(w)) for n, o, w in (r.split() for r in rows[1:])]
    bad = []
    if got_params != want_params:
        bad.append(f"NV_SWRUNLIST_ALLOCATION_PARAMS (size, engineId, maxTSGs, qos): OGKM {got_params}, table {want_params}")
    if got_rpc_size != want_rpc_size:
        bad.append(f"rpc_alloc_memory_v13_01 size: OGKM {got_rpc_size}, table {want_rpc_size}")
    if got_fields != want_fields:
        bad.append(f"rpc_alloc_memory_v13_01 prefix fields: OGKM {got_fields}, table {want_fields}")
    if bad:
        print("\n".join(bad), file=sys.stderr)
        raise SystemExit(1)
    print("sw_runlist.rs layout numbers match OGKM 580.65.06")


if __name__ == "__main__":
    main()

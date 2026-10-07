#!/usr/bin/env python3
"""Census already compiler-derived display rows; never parses C or infers absent ABI."""
import argparse
import hashlib
import json
import subprocess


def git(repo, *args):
    return subprocess.check_output(["git", "-C", repo, *args])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--revision", default="fdc991c9d3fd51dab005beda2487365661a3db25")
    args = parser.parse_args()
    revision = git(args.repo, "rev-parse", args.revision).decode().strip()
    names = git(args.repo, "ls-tree", "-r", "--name-only", revision,
                "crates/kf-disp/data").decode().splitlines()
    names = [n for n in names if n.rsplit("/", 1)[-1].startswith("classes-") and n.endswith(".tsv")]
    output = {"product_revision": revision, "input_kind": "compiler-derived TSV, not raw C", "tables": []}
    for name in names:
        raw = git(args.repo, "show", f"{revision}:{name}")
        rows = {}
        for line in raw.decode().splitlines():
            cells = line.split("\t")
            if len(cells) >= 3 and cells[0] in ("A", "F", "V"):
                rows[cells[1]] = {"kind": cells[0], "values": [int(x) for x in cells[2:]]}
        classes = {}
        for cl in ("C373", "C573", "C673", "C773", "CA73"):
            requested = ("PRECOMP_WIN_PIPE_HDR_CAPA", "PRECOMP_WIN_PIPE_HDR_CAPA_TMO_PRESENT",
                         "PRECOMP_WIN_PIPE_HDR_CAPA_TMO_PRESENT_TRUE", "PRECOMP_WIN_PIPE_HDR_CAPD",
                         "PRECOMP_WIN_PIPE_HDR_CAPD_TMO_LOGSZ", "PRECOMP_WIN_PIPE_HDR_CAPD_TMO_LOGNR",
                         "PRECOMP_WIN_PIPE_HDR_CAPD_TMO_SFCLOAD", "PRECOMP_WIN_PIPE_HDR_CAPD_TMO_DIRECT")
            classes[cl] = {n: rows.get(f"NV{cl}_{n}") for n in requested}
        methods = {}
        for cl in ("C37E", "C57E", "C67E", "C97E", "CA7E"):
            requested = ("SET_TMO_CONTROL", "SET_CONTEXT_DMA_TMO_LUT", "SET_OFFSET_TMO_LUT",
                         "SET_SURFACE_ADDRESS_HI_TMO_LUT", "SET_SURFACE_ADDRESS_LO_TMO_LUT",
                         "SET_SURFACE_ADDRESS_LO_TMO_LUT_ENABLE",
                         "SET_SURFACE_ADDRESS_LO_TMO_LUT_ENABLE_DISABLE")
            methods[cl] = {n: rows.get(f"NV{cl}_{n}") for n in requested}
        output["tables"].append({"path": name, "sha256": hashlib.sha256(raw).hexdigest(),
                                 "capabilities": classes, "methods": methods})
    print(json.dumps(output, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()

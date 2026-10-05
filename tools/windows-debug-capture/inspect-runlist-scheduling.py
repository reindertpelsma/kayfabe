#!/usr/bin/env python3
"""Record reproducible symbol provenance for the bounded 20801111 review.

The reviewed object is read only as ELF data. No proprietary code is executed.
Semantic observations in the companion note require disassembly review; these
hashes and relocations identify that review's inputs, not a semantic verifier.
"""

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path


REVIEWS = {
    "8c753d0898c478c56e22958d35aa271d671b3eb58f45069369b1df19480433f0": {
        "version": "535.309.01",
        "symbols": ["_nv046624rm", "_nv044643rm", "_nv044631rm",
                    "_nv003173rm", "_nv007675rm", "_nv007676rm",
                    "_nv044666rm", "_nv044667rm", "_nv013280rm",
                    "_nv034640rm", "_nv023008rm"],
    },
    "af6dbeebe6b7d4d5ea63fa954796dc78de69cfaa754922a3425f62dab108b859": {
        "version": "610.43.02",
        "symbols": ["_nv055811rm", "_nv053921rm"],
    },
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--object", type=Path, action="append", required=True)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("runlist_elf",
        Path(__file__).with_name("inspect-linux-runlist.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    results = []
    for path in args.object:
        data = path.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        if digest not in REVIEWS:
            raise ValueError("object is not a reviewed proprietary Linux object")
        review = REVIEWS[digest]
        elf = module.Elf(data, {})
        symbols = []
        for name in review["symbols"]:
            row = next(s for s in elf.symbols if s["name"] == name)
            section, start, size = row["section"], row["offset"], row["size"]
            if elf.sections[section]["name"] != ".text" or not size:
                raise ValueError("reviewed function lacks a bounded text symbol")
            edges = []
            for (source_section, offset), (kind, index, addend) in elf.relocations.items():
                if source_section == section and start <= offset < start + size:
                    target = elf.symbols[index]
                    edges.append({"text_offset": hex(offset), "relocation_type": kind,
                                  "target_symbol": target["name"], "addend": addend})
            symbols.append({"symbol": name, "text_offset": hex(start), "size": size,
                "unrelocated_bytes_sha256": hashlib.sha256(
                    elf.bytes(section)[start:start + size]).hexdigest(),
                "relocations": sorted(edges, key=lambda e: int(e["text_offset"], 16))})
        results.append({"driver_file_version": review["version"],
                        "object_sha256": digest, "symbols": symbols})
    print(json.dumps({"schema": 1, "execution": "none; ELF parsing only",
        "qualification": "Symbol provenance only; semantics separately reviewed with objdump.",
        "reviews": results}, indent=2))


if __name__ == "__main__":
    main()

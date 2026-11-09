#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Derive host semaphore fault spellings from the pinned open driver, never a trace.

Run from the repository after initialising third_party/ogkm-580. Runtime admission
uses the committed output, so neither a submodule nor a GPU is needed on a box.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
PIN = "b81d58ee0224d1d290bef1c080592b619e184042"
FIFO = "src/nvidia/src/kernel/gpu/fifo/"
FILES = (
    FIFO + "kernel_fifo.c",
    FIFO + "arch/ampere/kernel_fifo_ga100.c",
    FIFO + "arch/blackwell/kernel_fifo_gb202.c",
    "src/common/inc/swref/published/ampere/ga100/dev_fault.h",
    "src/common/inc/swref/published/blackwell/gb202/dev_fault.h",
)
OUTPUT = Path(__file__).with_name("native_fault_names.json")


def derive():
    repo = ROOT / "third_party/ogkm-580"
    # Read committed objects at the pin; local source edits cannot enter output.
    sources = {p: subprocess.check_output(["git", "show", PIN + ":" + p], cwd=repo) for p in FILES}
    common, ampere, blackwell, amp_header, bw_header = (sources[p].decode("utf-8") for p in FILES)
    # kfifoPrintFaultingPbdmaEngineName_IMPL: either GR_HOST<grIdx>, or
    # engineInfo.engineName + pbdmaIdString[j]. The GR engine index is the
    # same RM_ENGINE_TYPE_GR_IDX; form GR<grIdx> with each published suffix.
    hosts = set(re.findall(r'"(GR_HOST[0-9]+)"', common))
    suffixes = set(re.findall(r'"(_PBDMA[0-9]+)"', common))
    assert hosts and suffixes, "open driver host/PBDMA name source changed"
    engines = hosts | {h.replace("_HOST", "") + s for h in hosts for s in suffixes}
    clients = set(re.findall(r'return "(HUBCLIENT_ESC[0-9]*)";', ampere + blackwell))
    header_clients = {
        "HUBCLIENT_" + x
        for x in re.findall(
            r"^#define NV_PFAULT_CLIENT_HUB_(ESC[0-9]*)\s", amp_header + bw_header, re.M
        )
    }
    assert clients and clients == header_clients, "ESC definitions and printer disagree"
    return (
        json.dumps(
            {
                "source": "NVIDIA/open-gpu-kernel-modules",
                "revision": PIN,
                "sha256": {p: hashlib.sha256(sources[p]).hexdigest() for p in FILES},
                "engines": sorted(engines),
                "clients": sorted(clients),
            },
            indent=2,
        )
        + "\n"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    data = derive()
    if args.check:
        if OUTPUT.read_text() != data:
            raise SystemExit("native fault names differ from the open driver derivation")
    else:
        OUTPUT.write_text(data)


if __name__ == "__main__":
    main()

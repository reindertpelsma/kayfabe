#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Derive host semaphore fault spellings from the pinned open driver, never a trace.

Run from the repository after initialising third_party/ogkm-580. Runtime admission
uses the committed output, so neither a submodule nor a GPU is needed on a box.

Two spellings of the same deliberate fault come from two printers in the open driver:
  Ampere and later (kfifoPrintFaultingPbdmaEngineName_IMPL, ga100/gb202 client printers):
    GR_HOST<i> or GR<i>_PBDMA<j>, with client HUBCLIENT_ESC*.
  Turing (kfifoPrintPbdmaId_TU102, the TU102 client printer): HOST<i>, the PBDMA id string, with
    client HUBCLIENT_HOST_CPU (the host CPU hub client, NV_PFAULT_CLIENT_HUB_HOST_CPU).
Only the host-side client of each family is derived; the arm's other conditions (PID, channel, VA,
fault and access type, count) are matched by native_xid_guard.py and are not touched here.
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
    FIFO + "arch/turing/kernel_fifo_tu102.c",
    "src/common/inc/swref/published/ampere/ga100/dev_fault.h",
    "src/common/inc/swref/published/blackwell/gb202/dev_fault.h",
    "src/common/inc/swref/published/turing/tu102/dev_fault.h",
    "src/common/inc/swref/published/turing/tu102/hwproject.h",
)
OUTPUT = Path(__file__).with_name("native_fault_names.json")


def read_sources(repo=None):
    repo = Path(repo) if repo else ROOT / "third_party/ogkm-580"
    # Read committed objects at the pin; local source edits cannot enter output.
    return {p: subprocess.check_output(["git", "show", PIN + ":" + p], cwd=repo) for p in FILES}


def turing_names(tu102, header, hwproject):
    """Turing host engine and client spellings, each cross-checked against a second source."""
    # kfifoPrintPbdmaId_TU102: pbdmaIdString[NV_HOST_NUM_PBDMA] = { "HOST0", ..., "HOST<n-1>" }.
    table = re.search(r"pbdmaIdString\s*\[[^\]]*\]\s*=\s*\{([^}]*)\}", tu102)
    count = re.search(r"^#define NV_HOST_NUM_PBDMA\s+([0-9]+)\b", hwproject, re.M)
    assert table and count, "open driver Turing PBDMA name source changed"
    names = re.findall(r'"([^"]*)"', table.group(1))
    # The ids are the PBDMA index, so the table must be HOST0..HOST<n-1> in order and as long as
    # the hardware's PBDMA count; anything else means the printer is not the one derived here.
    assert names == ["HOST%d" % i for i in range(int(count.group(1)))], "Turing PBDMA names changed"
    # The TU102 client printer must map the header's HOST_CPU id to the spelling the kernel logs.
    printed = re.findall(
        r"case NV_PFAULT_CLIENT_HUB_HOST_CPU:\s*return \"(HUBCLIENT_HOST_CPU)\";", tu102
    )
    defined = re.findall(r"^#define NV_PFAULT_CLIENT_HUB_HOST_CPU\s+0x[0-9a-fA-F]+", header, re.M)
    assert len(printed) == len(defined) == 1, "Turing HOST_CPU printer and header disagree"
    return set(names), set(printed)


def derive_names(sources):
    """The engine and client sets from the source texts of FILES (a dict, path to bytes)."""
    common, ampere, blackwell, turing, amp_header, bw_header, tu_header, tu_project = (
        sources[p].decode("utf-8") for p in FILES
    )
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
    turing_engines, turing_clients = turing_names(turing, tu_header, tu_project)
    return engines | turing_engines, clients | turing_clients


def derive(sources=None):
    sources = read_sources() if sources is None else sources
    engines, clients = derive_names(sources)
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

#!/usr/bin/env python3
"""Compare a reviewed retail engine table with compiled public OGKM source.

The proprietary object is read as data only. The executable compiled here is
the public gpuGetRmEngineType_IMPL function plus a tiny local print harness.
The retail function's branch semantics were reviewed separately with objdump;
this script does not execute or emulate proprietary machine code.
"""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--proprietary-object", type=Path, required=True)
    parser.add_argument("--ogkm-git", type=Path, required=True)
    parser.add_argument("--sdk-include", type=Path, required=True)
    args = parser.parse_args()
    data = args.proprietary_object.read_bytes()
    expected = "af6dbeebe6b7d4d5ea63fa954796dc78de69cfaa754922a3425f62dab108b859"
    if hashlib.sha256(data).hexdigest() != expected:
        raise ValueError("expected reviewed Linux 610.43.02 proprietary kernel object")

    def source(path):
        return subprocess.check_output(
            ["git", "-C", str(args.ogkm_git), "show", "610.43.02:" + path], text=True)

    body = source("src/nvidia/src/kernel/gpu/gpu_engine_type.c")
    start = body.index("RM_ENGINE_TYPE gpuGetRmEngineType_IMPL")
    body = body[start:body.index("\n}", start) + 2]
    enum = source("src/nvidia/inc/kernel/gpu/gpu_engine_type.h")
    start = enum.index("typedef enum")
    enum = enum[start:enum.index("} RM_ENGINE_TYPE;", start) + len("} RM_ENGINE_TYPE;")]
    with tempfile.TemporaryDirectory(prefix="kf-runlist-engine-") as work:
        work = Path(work)
        (work / "engine-notification.h").write_text(source(
            "src/common/sdk/nvidia/inc/class/cl2080_notification.h"))
        (work / "probe.c").write_text(
            '#include <stdio.h>\n#include "nvtypes.h"\n#include "engine-notification.h"\n'
            '#define NV_CHECK_OR_RETURN(level,condition,value) '
            'do { if (!(condition)) return (value); } while(0)\n'
            + enum + "\n" + body + "\n"
            'int main(void) { for (unsigned i=0;i<=NV2080_ENGINE_TYPE_LAST;i++) '
            'printf("%u\\n",gpuGetRmEngineType_IMPL(i)); }\n')
        subprocess.run(["cc", "-I" + str(args.sdk_include), str(work / "probe.c"),
                        "-o", str(work / "probe")], check=True)
        compiled = list(map(int, subprocess.check_output([str(work / "probe")],
                                                         text=True).splitlines()))
    # .rodata file offset + section-relative table offset in the hash-pinned ELF.
    table_start = 0xf6f5e0 + 0x55954c0
    # Reviewed _nv029646rm: 0 -> 0, 1..83 -> table[index-1], >=84 -> 84.
    retail = [0] + list(data[table_start:table_start + 83]) + [84]
    if compiled != retail:
        raise ValueError("retail engine conversion differs from public source")
    print(json.dumps({"public_tag": "610.43.02", "proprietary_object_sha256": expected,
        "public_function": "gpuGetRmEngineType_IMPL", "retail_symbol": "_nv029646rm",
        "retail_text_offset": "0x556f80", "table_rodata_offset": "0x55954c0",
        "inputs_compared": "0..84 inclusive", "all_85_match": True,
        "public_function_sha256": hashlib.sha256(body.encode()).hexdigest(),
        "qualification": "Retail edge branches reviewed by disassembly; "
                         "only public source was compiled and executed."}, indent=2))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Research-only NV01_TIMER textual spot-check across local measured tags.

This uses regex/string matching, not a C parser. It cannot prove C semantics,
preprocessor conditions, or callable behavior. It is not the v3 ABI generator
and MUST NOT produce product policy. See V3_WINDOWS_POOL_EXPERIMENT.md.
"""
import argparse
import re
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ogkm", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    matrix = (root / "crates/kf-abi/src/generated/matrix.rs").read_text()
    measured = matrix.split("pub const MEASURED", 1)[1].split("];", 1)[0]
    tags = re.findall(r"// (\d+\.\d+(?:\.\d+)?)", measured)
    assert tags, "no measured tags found"

    def git(*arguments):
        return subprocess.check_output(
            ["git", "-C", str(args.ogkm), *arguments], text=True
        )

    print("tag\tcommit\tunprivileged\tparams\tparent\tmulti_instance"
          "\tconstructor\tdestructor\trpc_to_phys")
    for tag in tags:
        resources = git("show", tag + ":src/nvidia/src/kernel/rmapi/resource_list.h")
        entry = next(e for e in resources.split("RS_ENTRY(")
                     if re.search(r"\*/\s*NV01_TIMER,", e))
        for value in ("*/ NV_FALSE,", "RS_LIST(classId(Subdevice))", "*/ RS_NONE,",
                      "RS_FLAGS_ALLOC_NON_PRIVILEGED", "RS_ACCESS_NONE"):
            assert value in entry, (tag, value)
        timer = git("show", tag + ":src/nvidia/src/kernel/gpu/timer/timer.c")
        constructor = re.search(
            r"tmrapiConstruct_IMPL\s*\([^)]*\)\s*\{([^}]*)\}", timer, re.S)
        destructor = re.search(
            r"tmrapiDestruct_IMPL\s*\([^)]*\)\s*\{([^}]*)\}", timer, re.S)
        assert constructor and constructor[1].strip() == "return NV_OK;", tag
        assert destructor and not destructor[1].strip(), tag
        commit = git("rev-parse", tag + "^{commit}").strip()
        routed = int("RS_FLAGS_ALLOC_RPC_TO_PHYS_RM" in entry)
        print(f"{tag}\t{commit}\tyes\tRS_NONE\tSubdevice\tfalse"
              f"\treturn_NV_OK\tempty\t{routed}")


if __name__ == "__main__":
    main()

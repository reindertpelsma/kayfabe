#!/usr/bin/env python3
"""Measure ALLOC_MEMORY from public OGKM headers at explicit local Git tags.

Only this repository's C probe is executed. C facts come from compiled public
declarations, including bitfield values initialized and copied by the compiler.
No NVIDIA binary, driver, firmware, or installer is executed.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "drivermatrix"))
import dm

DEFAULT_TAGS = ["515.43.04", "535.309.01", "550.90.12", "560.35.03", "565.57.01", "570.86.15",
                "580.65.06", "580.126.09", "580.159.04", "580.173.02",
                "595.91.07", "610.43.02", "615.71.09"]


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args])


def measure_pitch_headers(root, paths):
    """Compile published header definitions; do not infer absent-family support."""
    rows = {}
    for header in sorted(p for p in paths if "/swref/published/" in p and p.endswith("/dev_mmu.h")):
        probe = root / "kind-probe.c"
        probe.write_text('#include <stdio.h>\n#include ' + json.dumps(str(root / header)) + '''
#ifdef NV_MMU_PTE_KIND_PITCH
int main(void) { printf("%u\\n", (unsigned)NV_MMU_PTE_KIND_PITCH); return 0; }
#else
int main(void) { puts("NOT_DEFINED"); return 0; }
#endif
''')
        result = subprocess.run(["cc", "-m64", "-std=gnu11", str(probe),
                                 "-o", str(root / "kind-probe")], capture_output=True, text=True)
        if result.returncode:
            rows[header] = {"status": "MISSING", "diagnostics": result.stderr.replace(str(root), "<source>")}
            continue
        value = subprocess.check_output([str(root / "kind-probe")], text=True).strip()
        rows[header] = ({"status": "NOT_DEFINED"} if value == "NOT_DEFINED" else
                        {"status": "MEASURED", "NV_MMU_PTE_KIND_PITCH": int(value)})
    return rows


def measure(repo, tag):
    commit = git(repo, "rev-parse", tag + "^{commit}").decode().strip()
    all_paths = git(repo, "ls-tree", "-r", "--name-only", commit).decode().splitlines()
    paths = [p for p in all_paths if any(p == x.strip("/") or
             p.startswith(x.strip("/") + "/") for x in dm.SPARSE)]
    source_paths = [p for p in all_paths if p.endswith((
        "/g_rpc-structures.h", "/sdk-structures.h", "/rpc_headers.h",
        "/rm_page_size.h", "/rpc.c", "/mem_list.c", "/mem.c", "/mem_desc.c",
        "/nvos.h", "/cl84a0.h", "/rmapi_deprecated_utils.c", "/resource_list.h",
        "/dev_mmu.h", "/g_mem_mgr_nvoc.c", "/mem_mgr_gm107.c",
        "/mem_mgr_tu102.c", "/system_mem.c"))
        or p in ("src/nvidia/arch/nvalloc/unix/src/os.c", "kernel-open/nvidia/nv.c")]
    row = {"tag": tag, "tag_object": git(repo, "rev-parse", tag).decode().strip(), "commit": commit,
           "source_blobs": {p: git(repo, "rev-parse", commit + ":" + p).decode().strip()
                            for p in source_paths}}
    with tempfile.TemporaryDirectory(prefix="kf-alloc-memory-") as work:
        root = Path(work)
        with subprocess.Popen(["git", "-C", str(repo), "archive", commit, *paths],
                              stdout=subprocess.PIPE) as archive:
            subprocess.run(["tar", "-x", "-C", str(root)], stdin=archive.stdout, check=True)
            if archive.wait() != 0:
                raise ValueError("public Git archive failed")
        command = ["cc", "-m64", "-std=gnu11", *dm.env_flags(str(root), "rm"),
                   str(HERE / "alloc-memory-layout.c"), "-o", str(root / "probe")]
        compile_result = subprocess.run(command, capture_output=True, text=True)
        row["compiler_diagnostics"] = compile_result.stderr.replace(str(root), "<source>")
        if compile_result.returncode:
            row["status"] = "MISSING"
            return row
        facts = json.loads(subprocess.check_output([str(root / "probe")], text=True))
        for group in facts.values():
            if isinstance(group, dict):
                group.pop("end", None)
        row.update(status="MEASURED", facts=facts)
        mask = 0
        for field in facts["flags"].values():
            mask |= field["mask"]
        row["reviewed_flag_field_mask"] = mask
        row["observed_0x48002000_fields"] = {
            name: (0x48002000 & field["mask"]) >> field["shift"]
            for name, field in facts["flags"].items()}
        row["pitch_header_facts"] = measure_pitch_headers(root, paths)
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ogkm-git", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--tags", nargs="+", default=DEFAULT_TAGS)
    args = parser.parse_args()
    result = {"schema": 1, "source_repository":
              "https://github.com/NVIDIA/open-gpu-kernel-modules",
              "compiler": subprocess.check_output(["cc", "--version"], text=True).splitlines()[0],
              "target": "x86_64 Linux; little endian; -m64 -std=gnu11",
              "probe_sha256": hashlib.sha256((HERE / "alloc-memory-layout.c").read_bytes()).hexdigest(),
              "rows": []}
    for tag in args.tags:
        row = measure(args.ogkm_git, tag)
        result["rows"].append(row)
        print(tag, row["status"], file=sys.stderr, flush=True)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()

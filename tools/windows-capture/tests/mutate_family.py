#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Require two distinct failing regression tests per family/revisit mutant (host only)."""
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
MUTANTS = [
    ("nearby_forward_only", "lib/kfwc_engine.c",
     "if (pos < span) {", "if (1) {"),
    ("pending_scan_uses_sibling_size", "lib/kfwc_discover.c",
     "stat_pages = i - 1u - cmd_pages;", "stat_pages = cmd_pages;"),
    ("pending_region_uses_sibling_size", "lib/kfwc_discover.c",
     "!b->q[1].valid && s1 - s0 < KFWC_QUEUE_MIN_BYTES",
     "!b->q[1].valid && s1 - s0 < b->q[0].size"),
    ("architecture_high_bit", "lib/kfwc_discover.c",
     "| (((boot0 >> 8) & 1u) << 5)", "| 0u"),
    ("fsp_as_direct_mailbox", "lib/kfwc_discover.c",
     "d->caps.boot_path != KFWC_BOOT_LIBOS", "d->caps.boot_path == KFWC_BOOT_UNKNOWN"),
    ("no_family_report", "lib/kfwc_engine.c",
     "e->stats.family = diag.caps.family;", "e->stats.family = 0;"),
    ("no_geometry_report", "lib/kfwc_engine.c",
     "e->stats.cmd_bytes = bd->q[0].size;", "e->stats.cmd_bytes = 0;"),
    ("no_early_moved_table", "lib/kfwc_engine.c",
     "(void)recent_scan_step(e);", "/* no nearby scan */"),
    ("no_same_table_revisit", "lib/kfwc_engine.c",
     "bind_fresh(e, e->cfg.recent_table_pa, KFWC_METHOD_REVISIT)", "0"),
    ("stale_epoch_accepted", "lib/kfwc_engine.c",
     "info.seq != 0 ||", "0 ||"),
    ("either_old_queue_may_seed", "lib/kfwc_engine.c",
     "e->dir[0].next_seq <= e->bind.q[0].count || "
     "e->dir[1].next_seq <= e->bind.q[1].count",
     "(e->dir[0].next_seq <= e->bind.q[0].count && "
     "e->dir[1].next_seq <= e->bind.q[1].count)"),
]


def run(root, out):
    sources = [root / "tests/test_main.c"]
    sources += [root / "lib" / f"kfwc_{name}.c"
                for name in ("record", "elem", "discover", "engine")]
    exe = out / "test"
    built = subprocess.run(
        ["gcc", "-std=c99", "-O1", "-g", "-Wall", "-Wextra", "-Werror",
         "-Wno-unused-function", *map(str, sources), "-o", str(exe)],
        capture_output=True, text=True, check=False,
    )
    if built.returncode:
        raise RuntimeError("build failure is not a mutation kill: " + built.stderr[-2000:])
    result = subprocess.run([str(exe)], capture_output=True, text=True, timeout=120,
                            check=False)
    killed = sorted(set(re.findall(r"^FAIL (\S+)$", result.stdout, re.M)))
    return result.returncode, killed


def main():
    out = Path(sys.argv[1]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    if run(ROOT, out)[0]:
        raise SystemExit("baseline fails")
    rows = []
    for name, rel, old, new in MUTANTS:
        with tempfile.TemporaryDirectory(dir=out, prefix="mut-") as tmp:
            work = Path(tmp)
            for sub in ("lib", "driver", "tests", "user"):
                shutil.copytree(ROOT / sub, work / sub)
            path = work / rel
            text = path.read_text()
            if text.count(old) != 1:
                raise RuntimeError("mutation location is not unique: " + name)
            text = text.replace(old, new, 1)
            if name == "stale_epoch_accepted":
                text = text.replace("copied.seq != 0 ||", "0 ||", 1)
            path.write_text(text)
            rc, killed = run(work, out)
            rows.append({"mutant": name, "exit": rc, "killing_tests": killed,
                         "pass": rc != 0 and len(killed) >= 2})
    with tempfile.TemporaryDirectory(dir=out, prefix="mut-file-") as tmp:
        work = Path(tmp)
        for sub in ("lib", "driver", "tests", "user"):
            shutil.copytree(ROOT / sub, work / sub)
        path = work / "user/kfwc_file.h"
        text = path.read_text().replace(" || fflush(out) != 0", "", 1)
        path.write_text(text)
        exe = out / "test_file"
        subprocess.run(["gcc", "-std=c99", "-O1", "-Wall", "-Wextra", "-Werror",
                        str(work / "tests/test_file.c"), str(work / "lib/kfwc_record.c"),
                        "-o", str(exe)], check=True)
        result = subprocess.run([str(exe)], capture_output=True, text=True, check=False)
        killed = sorted(set(re.findall(r"^FAIL (\S+)$", result.stdout, re.M)))
        rows.append({"mutant": "buffered_header_not_published", "exit": result.returncode,
                     "killing_tests": killed, "pass": result.returncode != 0 and len(killed) >= 2})
    with tempfile.TemporaryDirectory(dir=out, prefix="mut-pull-") as tmp:
        work = Path(tmp)
        for sub in ("rehearse", "tests"):
            shutil.copytree(ROOT / sub, work / sub)
        path = work / "rehearse/verify_pull.py"
        text = path.read_text().replace("if not OUTPUT_NAME.fullmatch(name):", "if False:", 1)
        path.write_text(text)
        result = subprocess.run([sys.executable, str(work / "tests/test_verify_pull.py")],
                                capture_output=True, text=True, check=False)
        killed = sorted(set(re.findall(r"^FAIL: (\w+)", result.stderr, re.M)))
        rows.append({"mutant": "unlisted_private_output_allowed", "exit": result.returncode,
                     "killing_tests": killed, "pass": result.returncode != 0 and len(killed) >= 2})
    print(json.dumps({"mutants": rows, "pass": all(r["pass"] for r in rows)}, indent=2))
    return 0 if all(r["pass"] for r in rows) else 1


if __name__ == "__main__":
    sys.exit(main())

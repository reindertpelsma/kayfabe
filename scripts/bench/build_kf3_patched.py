#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Build the kf3 QEMU with an optional QEMU patch, beside the plain build. STATUS: LIVE, 2026-11-24.

    python3 scripts/bench/build_kf3_patched.py --with-patch hyperv-wake <qemu-source-tree>

The patches are plain diffs in tools/qemu-patches/ (the kit scripts/release/build_qemu.py knows the names and applies them). The
plain build, scripts/bench/build_kf3.sh, never applies one and never sees one: this script copies the tree to
`<tree's parent>/patched-<names>/<tree name>`, puts exactly the named patches into the copy, and runs build_kf3.sh on the copy.
So the copy has its own build directory (`qemu-build-kf3` in `patched-<names>/`), its own lock and its own `kf3-bins/<rev>/qemu-system-x86_64`, and the plain tree, build
directory and binary are untouched. The launch scripts run the plain binary; name the patched one with `QEMU_BIN`.
A patch is meant only for a run that needs what it fixes (tools/qemu-patches/<name>.patch says who that is).
"""
import argparse
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "scripts" / "release"))
import build_qemu as kit  # noqa: E402


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--with-patch", action="append", required=True, choices=sorted(kit.OPTIONAL_PATCHES), metavar="NAME", dest="patches",
                   help="an optional QEMU patch to build in (%s); repeatable" % ", ".join(sorted(kit.OPTIONAL_PATCHES)))
    p.add_argument("tree", metavar="QEMU_TREE", help="the extracted QEMU 10.2.4 source tree (it is not modified)")
    a = p.parse_args(argv)
    names = kit.patch_names(a.patches)
    tree = Path(a.tree).resolve()
    if not (tree / "VERSION").is_file():
        print("%s is not a QEMU source tree" % tree, file=sys.stderr)
        return 1
    work = tree.parent / ("patched-" + "+".join(names))
    copy = work / tree.name
    try:
        if not copy.exists():
            work.mkdir(exist_ok=True)
            shutil.copytree(tree, copy, symlinks=True)
        done = kit.set_tree_patches(copy, names)
    except kit.KitRefused as e:
        print("KIT_REFUSED: %s" % e, file=sys.stderr)
        return 1
    print("KIT_PATCHES %s in %s" % (" ".join("%s=%s" % kv for kv in sorted(done.items())), copy))
    build = work / "qemu-build-kf3"  # not a choice: another build directory could put the binary over the plain one
    return subprocess.run(["bash", str(ROOT / "scripts" / "bench" / "build_kf3.sh"), str(copy), str(build)]).returncode


if __name__ == "__main__":
    sys.exit(main())

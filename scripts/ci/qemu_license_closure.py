#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# STATUS: LIVE, 2026-11-20 (task I7; LICENSE exception 2; docs/design/V3_GUEST_INGRESS_TABLE.md section 6).
"""Licence closure of the kf3 QEMU device: nothing Apache-only and nothing gVisor-derived may be linked in.

The device (qemu/hw/misc/kf3/) links the static archive of crates/kf-qemu, so its closure is kf-qemu's
normal (non-dev, non-build) cargo dependency closure plus the C device sources and the committed CUDA walk
kernel. For every file of every workspace crate in that closure, and for the C/PTX sources:
  - an SPDX tag, where present, must allow GPL-2.0 (an Apache-2.0-only file fails);
  - the file must not be one LICENSE exception 2 lists (gVisor-derived);
  - the file must not name gVisor or nvproxy (kayfabe's rule since I7: no gVisor code, comment, table or
    citation in the QEMU-linked tree; THIRD_PARTY_NOTICES.md and the credits page carry the attribution).
Every third-party crate in the closure must declare a licence expression with a GPL-2.0-compatible option
(MIT, BSD-2/3-Clause, ISC, Zlib, Unicode-*, or Apache-2.0 only together with one of those).

Supply chain (stage-2 security audit S2-04, docs/audits/2026-11-23-v3-stage2.md; stage 1 S1-14, S1-28): the closure
also follows build-dependency edges (a build script runs on the build machine, and kf-gop-image's output is embedded),
and it includes the nested firmware workspace that kf-gop-image's build script builds and embeds
(firmware/kf-gop, its own Cargo.lock) and the display PTX kf-cuda embeds (cuda/display). Every third-party crate
in that closure must be named in EXTERNAL_PINNED; a new one fails until a reviewer adds it there.

usage: qemu_license_closure.py [--metadata FILE]     (default: runs `cargo metadata --format-version 1`)
exit 0 = clean, 1 = a finding (each printed as QEMU_CLOSURE_FAIL ...), 2 = cannot run.
"""
import argparse
import json
import os
import re
import subprocess
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "../.."))
EXTRA_DIRS = ["qemu/hw/misc/kf3", "cuda/walk", "cuda/display", "firmware/kf-gop"]
ROOT_CRATE = "kf-qemu"
# Workspaces outside the root Cargo.toml whose build output is embedded in the kf3 binary.
NESTED_LOCKS = ["firmware/kf-gop/Cargo.lock"]
# The third-party crates the kf3 binary may contain or run at build time. Adding one is a reviewed supply-chain change.
EXTERNAL_PINNED = {"libc"}
# Edges followed: normal and build (a build script runs at build time); dev edges reach only tests.
FOLLOWED_KINDS = {None, "build"}
GPL_OK_SPDX = re.compile(r"GPL-2\.0|MIT|BSD-[23]-Clause|ISC|Zlib")
GPL_OK_CRATE = {"MIT", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "Unicode-3.0", "Unicode-DFS-2016",
                "0BSD", "Unlicense", "CC0-1.0", "BSL-1.0"}
FORBIDDEN_WORDS = re.compile(r"gvisor|nvproxy", re.I)
SPDX = re.compile(r"SPDX-License-Identifier:\s*([^\n*]+?)\s*(?:\*/|-->|$)", re.M)
TEXT_EXT = {".rs", ".c", ".h", ".cu", ".ptx", ".toml", ".build", ".md", ".txt", ".tsv", ".json", ".py", ".sh"}


def exception2_files():
    """The files LICENSE exception 2 names (gVisor-derived, Apache-2.0 only)."""
    text = open(os.path.join(ROOT, "LICENSE"), encoding="utf-8").read()
    m = re.search(r"\n  2\. (.*?)\n  3\. ", text, re.S)
    if not m:
        raise SystemExit("QEMU_CLOSURE_ERROR LICENSE has no exception 2 block")
    return set(re.findall(r"^\s+- ((?:crates|archive|qemu|cuda)/\S+)", m.group(1), re.M))


def crate_licence_ok(expr):
    if not expr:
        return False
    for alt in re.split(r"\s+OR\s+|/", expr.replace("(", "").replace(")", "")):
        parts = [p.strip() for p in re.split(r"\s+AND\s+", alt)]
        if all(p in GPL_OK_CRATE or p.startswith("GPL-2.0") or p.startswith("LGPL") for p in parts):
            return True
    return False


def closure(meta):
    pk = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    start = [p["id"] for p in meta["packages"] if p["name"] == ROOT_CRATE and p["source"] is None]
    if not start:
        raise SystemExit("QEMU_CLOSURE_ERROR no workspace crate %s" % ROOT_CRATE)
    seen, stack = set(), list(start)
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i]["deps"]:
            if any(k["kind"] in FOLLOWED_KINDS for k in d["dep_kinds"]):
                stack.append(d["pkg"])
    return [pk[i] for i in sorted(seen)]


def nested_lock_externals(text):
    """(name, version) of every package in a Cargo.lock that comes from a registry or git (has a `source`)."""
    out = []
    for block in re.split(r"^\[\[package\]\]\s*$", text, flags=re.M)[1:]:
        name = re.search(r'^name = "([^"]+)"', block, re.M)
        version = re.search(r'^version = "([^"]+)"', block, re.M)
        if name and re.search(r"^source = ", block, re.M):
            out.append((name.group(1), version.group(1) if version else "?"))
    return out


def unpinned(external):
    """Findings for third-party crates, given as (name, version, where), that EXTERNAL_PINNED does not name."""
    return ["crate %s %s (%s) is not in EXTERNAL_PINNED: a new third-party crate in the kf3 binary is a reviewed "
            "supply-chain change" % (n, v, where) for n, v, where in external if n not in EXTERNAL_PINNED]


def check_file(rel, banned, findings):
    path = os.path.join(ROOT, rel)
    if rel in banned:
        findings.append("%s is listed in LICENSE exception 2 (gVisor-derived)" % rel)
    if os.path.splitext(rel)[1] not in TEXT_EXT:
        return
    text = open(path, encoding="utf-8", errors="replace").read()
    for m in SPDX.finditer(text[:4000]):
        if not GPL_OK_SPDX.search(m.group(1)):
            findings.append("%s: SPDX '%s' has no GPL-2.0-compatible option" % (rel, m.group(1).strip()))
    if FORBIDDEN_WORDS.search(text):
        line = text[:FORBIDDEN_WORDS.search(text).start()].count("\n") + 1
        findings.append("%s:%d names gVisor/nvproxy inside the QEMU-linked tree" % (rel, line))


def tracked(dirpath):
    out = subprocess.run(["git", "-C", ROOT, "ls-files", "--", dirpath], capture_output=True, text=True,
                         check=True).stdout
    return [ln for ln in out.splitlines() if ln]


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--metadata", help="a saved `cargo metadata --format-version 1` output")
    a = ap.parse_args(argv)
    if a.metadata:
        meta = json.load(open(a.metadata))
    else:
        p = subprocess.run(["cargo", "metadata", "--format-version", "1", "--locked"], cwd=ROOT,
                           capture_output=True, text=True)
        if p.returncode:
            print("QEMU_CLOSURE_ERROR cargo metadata failed: " + p.stderr[-400:], file=sys.stderr)
            return 2
        meta = json.loads(p.stdout)
    banned = exception2_files()
    findings, files, local, external = [], 0, [], []
    for pkg in closure(meta):
        if pkg["source"] is None:
            crate_dir = os.path.relpath(os.path.dirname(pkg["manifest_path"]), ROOT)
            local.append(pkg["name"])
            for rel in tracked(crate_dir):
                # a nested crate (e.g. kf-abi/gen) is its own package, checked only if it is in the closure
                if any(rel.startswith(crate_dir + "/" + sub + "/") for sub in ("gen",)) and crate_dir.endswith("kf-abi"):
                    continue
                files += 1
                check_file(rel, banned, findings)
        else:
            external.append((pkg["name"], pkg["version"], pkg.get("license")))
            if not crate_licence_ok(pkg.get("license")):
                findings.append("crate %s %s: licence '%s' has no GPL-2.0-compatible option"
                                % (pkg["name"], pkg["version"], pkg.get("license")))
    for d in EXTRA_DIRS:
        for rel in tracked(d):
            files += 1
            check_file(rel, banned, findings)
    pinned_input = [(n, v, "cargo metadata") for n, v, _ in external]
    for lock in NESTED_LOCKS:
        nested = nested_lock_externals(open(os.path.join(ROOT, lock), encoding="utf-8").read())
        pinned_input += [(n, v, lock) for n, v in nested]
        external += [(n, v, "from " + lock) for n, v in nested]
    findings += unpinned(pinned_input)
    for f in findings:
        print("QEMU_CLOSURE_FAIL " + f)
    print("QEMU_CLOSURE %s: %d workspace crates (%s), %d files, %d external crates (%s); findings=%d"
          % ("CLEAN" if not findings else "DIRTY", len(local), " ".join(sorted(local)), files, len(external),
             ", ".join("%s %s %s" % e for e in external), len(findings)))
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())

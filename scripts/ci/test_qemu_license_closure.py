#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Tests of the supply-chain half of qemu_license_closure.py (STATUS: LIVE, 2026-11-23; stdlib only, no cargo).

Stage-2 security audit S2-04 (docs/audits/2026-11-23-v3-stage2.md): the kf3 closure follows build edges, reads the
nested firmware workspace's lock, and refuses a third-party crate that EXTERNAL_PINNED does not name.
"""
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import qemu_license_closure as qlc  # noqa: E402


def meta(edges, sources):
    """A minimal `cargo metadata` document: edges = {pkg: [(dep, kind)]}, sources = {pkg: source or None}."""
    pkgs = [{"id": n, "name": n, "version": "1.0.0", "source": s, "manifest_path": "/x/%s/Cargo.toml" % n}
            for n, s in sources.items()]
    nodes = [{"id": n, "deps": [{"pkg": d, "dep_kinds": [{"kind": k}]} for d, k in edges.get(n, [])]}
             for n in sources]
    return {"packages": pkgs, "resolve": {"nodes": nodes}}


REG = "registry+https://github.com/rust-lang/crates.io-index"


class Closure(unittest.TestCase):
    def test_build_edges_are_followed_and_dev_edges_are_not(self):
        m = meta({qlc.ROOT_CRATE: [("img", None), ("tester", "dev")], "img": [("builder", "build")]},
                 {qlc.ROOT_CRATE: None, "img": None, "builder": REG, "tester": REG})
        names = {p["name"] for p in qlc.closure(m)}
        self.assertIn("builder", names, "a build dependency runs at build time and is part of the supply chain")
        self.assertNotIn("tester", names, "a dev dependency reaches only tests")

    def test_a_crate_outside_the_pin_is_a_finding(self):
        self.assertEqual(qlc.unpinned([("libc", "0.2.189", "cargo metadata")]), [])
        found = qlc.unpinned([("libc", "0.2.189", "cargo metadata"), ("evil", "0.1.0", "firmware/kf-gop/Cargo.lock")])
        self.assertEqual(len(found), 1)
        self.assertIn("evil 0.1.0", found[0])

    def test_the_nested_lock_reader_returns_only_registry_and_git_packages(self):
        lock = ('version = 4\n\n[[package]]\nname = "kf-gop"\nversion = "0.1.0"\ndependencies = [\n "kf-oprom",\n]\n\n'
                '[[package]]\nname = "kf-oprom"\nversion = "0.1.0"\n\n'
                '[[package]]\nname = "uefi"\nversion = "0.30.0"\nsource = "%s"\nchecksum = "00"\n\n'
                '[[package]]\nname = "gitdep"\nversion = "0.2.0"\nsource = "git+https://example.invalid/x#abc"\n' % REG)
        self.assertEqual(qlc.nested_lock_externals(lock), [("uefi", "0.30.0"), ("gitdep", "0.2.0")])

    def test_the_embedded_firmware_and_display_programs_are_in_the_scan(self):
        for d in ("firmware/kf-gop", "cuda/display", "cuda/walk", "qemu/hw/misc/kf3"):
            self.assertIn(d, qlc.EXTRA_DIRS)
        self.assertIn("firmware/kf-gop/Cargo.lock", qlc.NESTED_LOCKS)

    def test_the_committed_firmware_lock_has_no_unpinned_crate(self):
        text = (HERE.parents[1] / "firmware/kf-gop/Cargo.lock").read_text(encoding="utf-8")
        found = qlc.unpinned([(n, v, "firmware") for n, v in qlc.nested_lock_externals(text)])
        self.assertEqual(found, [])


if __name__ == "__main__":
    unittest.main()

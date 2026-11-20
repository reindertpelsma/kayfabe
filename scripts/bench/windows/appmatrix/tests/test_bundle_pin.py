# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The repository's bundle.pin must name the recipe inputs the checkout derives (offline, no ISO).

2026-11-19/20 (MEASURED on a Vast box, tree 7fb1c98a0): the stage `windows_apps` failed in
`package_bundle.verify_image` with "embedded recipe input identity mismatch". The pinned ISO was built from commit
38bbbb2e9 (inputs=ed2ddea1...); apps.json (PRs 470, 471, 475) and guest/py/win_cuda_equiv.py (475) are hash-covered inputs
and changed afterwards, so the checkout derives b8694b93... The failure is deterministic: the pin no longer matched the
tree, and nothing in CI compared them.

FALSIFIER (written before the code): the repository check passes while the pin's `# inputs=` differs from
`package_bundle.input_hash` of the tree, or a hash-covered input class (apps.json, guest .ps1/.py/.html, the six recipe .py
files, the LICENSE files, bundle_tools.SOURCES) can change without the check noticing.

RepoPin is the check on the real tree. It is RED while the bundle has not been republished after a covered input changed;
that is the point of it (a ratchet would hide it). PinDriftDetection runs the same check on a disposable copy of the tree
whose pin is generated from that copy, so it is green and carries the mutation killers (pin_mutants.py).

NOT COVERED (it needs the network and the ISO): that the published ISO really embeds the pinned inputs, and that the
pinned ISO's bytes match bundle.pin's sha256 line; fetch_public_bundle.py checks both when the stage downloads it.
"""
import json
from pathlib import Path
import shutil
import tempfile
import unittest

import helpers as H
import package_bundle as B
import bundle_ci as C
import bundle_tools


class RepoPin(unittest.TestCase):
    def test_bundle_pin_inputs_equal_the_tree(self):
        drift = C.pin_drift()
        if drift:
            self.fail(C.pin_drift_message(drift))


def make_tree(parent):
    """A disposable copy of the appmatrix directory plus the repo-root files the input hash reads (SOURCES, licences)."""
    root = Path(parent) / "scripts/bench/windows/appmatrix"
    shutil.copytree(H.AM, root, ignore=shutil.ignore_patterns("__pycache__"))
    for rel in bundle_tools.SOURCES + ("LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md"):
        dst = Path(parent) / rel
        if not dst.exists():
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(Path(H.REPO) / rel, dst)
    return root


def pin_the_tree(root):
    inputs = B.input_hash(B.load(root / "manifest.json"), B.load(root / "gate_profiles.json"), root=root)
    (root / "bundle.pin").write_text(B.pin_text("a" * 64, inputs))
    return inputs


class PinDriftDetection(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = make_tree(self.tmp.name)
        self.inputs = pin_the_tree(self.root)

    def append(self, rel, text=b"\n# changed\n"):
        path = self.root.parents[3] / rel if rel.startswith(("LICENSE", "THIRD_PARTY", "scripts/", "archive/")) else self.root / rel
        path.write_bytes(path.read_bytes() + text)

    def edit_apps(self, change):
        path = self.root / "apps.json"
        data = json.loads(path.read_text())
        change(data)
        path.write_text(json.dumps(data, indent=1))

    def assert_drift(self):
        drift = C.pin_drift(self.root)
        self.assertIsNotNone(drift, "a hash-covered input changed and the pin check did not notice")
        self.assertEqual(drift["pinned"], self.inputs)
        self.assertNotEqual(drift["derived"], self.inputs)
        message = C.pin_drift_message(drift)
        for needle in (drift["pinned"], drift["derived"], C.PIN_DRIFT_HINT):
            self.assertIn(needle, message)

    # --- the check itself
    def test_a_pin_that_names_the_tree_is_not_drift(self):
        self.assertIsNone(C.pin_drift(self.root))

    def test_a_pin_naming_other_inputs_is_drift(self):
        text = (self.root / "bundle.pin").read_text().replace("# inputs=" + self.inputs, "# inputs=" + "0" * 64)
        (self.root / "bundle.pin").write_text(text)
        drift = C.pin_drift(self.root)
        self.assertEqual((drift["pinned"], drift["derived"]), ("0" * 64, self.inputs))

    def test_a_pin_without_an_inputs_line_is_drift_not_a_pass(self):
        (self.root / "bundle.pin").write_text("winapps-open.iso sha256=" + "a" * 64 + "\n")
        drift = C.pin_drift(self.root)
        self.assertEqual((drift["pinned"], drift["derived"]), ("", self.inputs))

    def test_an_empty_pin_is_drift_not_a_pass(self):
        (self.root / "bundle.pin").write_text("# STATUS: UNBUILT\n")
        drift = C.pin_drift(self.root)
        self.assertEqual((drift["pinned"], drift["derived"]), ("", self.inputs))

    def test_the_message_names_the_remedy(self):
        self.assertEqual(C.PIN_DRIFT_HINT, "republish the bundle (docs/DOWNLOADS.md, Windows app bundle) and update bundle.pin")

    def test_a_crlf_checkout_is_not_drift(self):
        path = self.root / "guest/kf_stage.ps1"
        path.write_bytes(path.read_bytes().replace(b"\r\n", b"\n").replace(b"\n", b"\r\n"))
        self.assertIsNone(C.pin_drift(self.root))

    # --- every hash-covered input class, two members each, so that a wrong file set has two named killers
    def test_apps_json_value_change_is_drift(self):
        self.edit_apps(lambda d: d["apps"][0].update(timeout_s=d["apps"][0]["timeout_s"] + 1))
        self.assert_drift()

    def test_apps_json_new_app_is_drift(self):
        self.edit_apps(lambda d: d["apps"].append(dict(d["apps"][0], id="a_pin_drift_probe")))
        self.assert_drift()

    def test_guest_python_win_cuda_equiv_change_is_drift(self):
        self.append("guest/py/win_cuda_equiv.py")
        self.assert_drift()

    def test_guest_python_other_file_change_is_drift(self):
        other = sorted(p for p in (self.root / "guest").rglob("*.py") if p.name != "win_cuda_equiv.py")[0]
        self.append(other.relative_to(self.root).as_posix())
        self.assert_drift()

    def test_guest_powershell_stage_change_is_drift(self):
        self.append("guest/kf_stage.ps1")
        self.assert_drift()

    def test_guest_powershell_setup_change_is_drift(self):
        self.append("guest/kf_guest_setup.ps1")
        self.assert_drift()

    def test_guest_html_change_is_drift(self):
        pages = sorted((self.root / "guest").rglob("*.html"))
        self.assertGreaterEqual(len(pages), 2)
        self.append(pages[0].relative_to(self.root).as_posix(), b"\n<!-- changed -->\n")
        self.assert_drift()

    def test_guest_second_html_change_is_drift(self):
        pages = sorted((self.root / "guest").rglob("*.html"))
        self.append(pages[1].relative_to(self.root).as_posix(), b"\n<!-- changed -->\n")
        self.assert_drift()

    def test_recipe_iso_writer_change_is_drift(self):
        self.append("bundle_iso.py")
        self.assert_drift()

    def test_recipe_wheels_change_is_drift(self):
        self.append("bundle_wheels.py")
        self.assert_drift()

    def test_licence_change_is_drift(self):
        self.append("LICENSE", b"changed")
        self.assert_drift()

    def test_third_party_notices_change_is_drift(self):
        self.append("THIRD_PARTY_NOTICES.md", b"changed")
        self.assert_drift()

    def test_probe_source_change_is_drift(self):
        self.append("scripts/bench/cup8.c", b"/* changed */")
        self.assert_drift()

    def test_dx_probe_source_change_is_drift(self):
        self.append("scripts/bench/windows/appmatrix/tools/src/kf_dxprobe.cpp", b"/* changed */")
        self.assert_drift()

    # --- the manifest and the profile are inputs too
    def test_manifest_pin_change_is_drift(self):
        path = self.root / "manifest.json"
        man = json.loads(path.read_text())
        next(p for p in man["packages"] if p["id"] == "python_embed")["files"][0]["sha256"] = "0" * 64
        path.write_text(json.dumps(man))
        self.assert_drift()

    def test_open_package_set_change_is_drift(self):
        path = self.root / "gate_profiles.json"
        prof = json.loads(path.read_text())
        prof["open_packages"].remove("hf_model")
        path.write_text(json.dumps(prof))
        self.assert_drift()


if __name__ == "__main__":
    unittest.main()

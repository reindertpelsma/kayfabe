#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/trust_lint.py (STATUS: LIVE, 2026-11-17), on fixture trees written to a scratch directory.
The names are the killers named by scripts/ci/trust_lint_mutants.py: renaming a test is a change of that file."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import kf_trust as T  # noqa: E402
import trust_lint as L  # noqa: E402

SHA = "a" * 64
SHA2 = "b" * 64


def pin(sha=SHA, **kw):
    a = {"class": "upstream-pin", "sha256": sha, "size": 10, "pin_origin": "vendor-published", "scope": "upstream-artifact",
         "pin_source_url": "https://download.example.org/x.sha256", "status": "MEASURED", "measured": "2026-11-17"}
    a.update(kw)
    return a


def entry(anchors, consumers=("scripts/bench/use.sh",), routing="helper", **kw):
    e = {"row": 99, "kind": "file", "routing": routing, "origin": "https://download.example.org/pkg/tool.run", "runs": "host",
         "consumers": list(consumers), "anchors": anchors}
    e.update(kw)
    return e


class Tree(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        self.addCleanup(self._t.cleanup)
        self.root = Path(self._t.name)
        self.write("scripts/bench/use.sh", "#!/bin/bash\npython3 -I scripts/ci/kf_trust.py fetch tool --out /tmp/x\n")

    def write(self, rel, text):
        p = self.root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)

    def manifest(self, artifacts=None, unrouted=None):
        doc = {"schema": T.SCHEMA, "artifacts": artifacts if artifacts is not None else {"tool": entry([pin()])}, "unrouted": unrouted or {}}
        self.write("scripts/ci/trust_manifest.json", json.dumps(doc))
        return doc

    def lint(self):
        return L.lint(self.root)

    def codes(self):
        return sorted(c for c, _, _ in self.lint())

    def assertCodes(self, *codes):
        self.assertEqual(self.codes(), sorted(codes), self.lint())


def unrouted(count=1, cls="operator", reason="a reason that is long enough", row=None):
    return {"count": count, "class": cls, "reason": reason, "row": row}


class Anchors(Tree):
    def test_a_clean_fixture_has_no_finding(self):
        self.manifest()
        self.assertCodes()

    def test_a_forbidden_anchor_is_a_finding(self):
        self.manifest({"tool": entry([{"class": "git-sha256-of-our-own-artifact", "sha256": SHA}])})
        self.assertCodes("FORBIDDEN_ANCHOR")

    def test_a_forbidden_anchor_beside_a_valid_pin_is_still_a_finding(self):
        self.manifest({"tool": entry([pin(), {"class": "git-sha256-of-our-own-artifact", "sha256": SHA}])})
        self.assertIn("FORBIDDEN_ANCHOR", self.codes())

    def test_an_artifact_with_an_empty_anchor_list_is_a_finding(self):
        self.manifest({"tool": entry([])})
        self.assertCodes("NO_ANCHOR")

    def test_an_artifact_with_no_anchor_key_is_a_finding(self):
        e = entry([pin()])
        del e["anchors"]
        self.manifest({"tool": e})
        self.assertCodes("NO_ANCHOR")

    def test_a_checksum_url_on_a_host_we_run_is_a_manifest_finding(self):
        self.manifest({"tool": entry([{"class": "official-checksum-url", "url": "https://bundles.kayfabe.dev/x.sha256", "format": "sha256sum"}])})
        self.assertCodes("MANIFEST_INVALID")

    def test_a_pin_whose_source_is_ours_is_a_manifest_finding(self):
        self.manifest({"tool": entry([pin(pin_source_url="https://logs.kayfabe.dev/x")])})
        self.assertCodes("MANIFEST_INVALID")

    def test_an_unknown_anchor_class_is_a_manifest_finding(self):
        self.manifest({"tool": entry([{"class": "trust-me"}])})
        self.assertCodes("MANIFEST_INVALID")

    def test_an_unreadable_manifest_is_one_finding(self):
        self.write("scripts/ci/trust_manifest.json", "{not json")
        self.assertCodes("MANIFEST_INVALID")

    def test_an_artifact_with_one_confirmed_anchor_is_not_pending(self):
        self.manifest({"d": entry([pin(pin_origin="measured-from-upstream"), pin(SHA2, pin_origin="vendor-published")])})
        self.assertEqual(L.pending(self.root / "scripts/ci/trust_manifest.json"), [])

    def test_the_pending_list_names_only_artifacts_resting_on_a_measured_binary(self):
        self.manifest({"a": entry([pin(pin_origin="measured-from-upstream")]),
                       "b": entry([pin(pin_origin="vendor-published")]),
                       "c": entry([pin(pin_origin="measured-from-upstream", scope="source")]),
                       "d": entry([pin(pin_origin="measured-from-upstream"), pin(SHA2, pin_origin="vendor-published")])})
        self.assertEqual(L.pending(self.root / "scripts/ci/trust_manifest.json"), ["a"])


class Routing(Tree):
    def test_a_missing_consumer_is_a_finding(self):
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/gone.sh"])})
        self.assertCodes("CONSUMER_MISSING")

    def test_a_missing_consumer_of_a_native_entry_is_a_finding_too(self):
        self.write("scripts/bench/native.sh", "#!/bin/bash\nPIN=" + SHA + "\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/native.sh", "scripts/bench/gone.sh"], routing="native",
                                     pin_files=[{"file": "scripts/bench/native.sh", "contains": [SHA]}])})
        self.assertCodes("CONSUMER_MISSING")

    def test_a_helper_routed_consumer_that_does_not_mention_kf_trust_is_a_finding(self):
        self.write("scripts/bench/other.sh", "#!/bin/bash\necho nothing here\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/other.sh"])})
        self.assertCodes("NOT_ROUTED")

    def test_a_second_consumer_that_names_no_routed_file_is_a_finding_too(self):
        self.write("scripts/bench/other.sh", "#!/bin/bash\necho nothing here\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/use.sh", "scripts/bench/other.sh"])})
        self.assertCodes("NOT_ROUTED")

    def test_a_consumer_that_calls_a_routed_sibling_counts_as_routed(self):
        self.write("scripts/bench/outer.sh", "#!/bin/bash\nbash scripts/bench/use.sh\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/use.sh", "scripts/bench/outer.sh"])})
        self.assertCodes()

    def test_a_consumer_that_still_spells_the_origin_url_is_a_finding(self):
        self.write("scripts/bench/use.sh", "#!/bin/bash\npython3 -I scripts/ci/kf_trust.py fetch tool --out x\n"
                                           "echo 'https://download.example.org/pkg/tool.run'\n")
        self.manifest()
        self.assertCodes("RAW_FETCH_OF_ROUTED")

    def test_a_python_consumer_that_still_spells_the_origin_url_is_a_finding(self):
        self.write("scripts/bench/use.py", "import kf_trust\nURL = 'https://download.example.org/pkg/tool.run'\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/use.py"])})
        self.assertCodes("RAW_FETCH_OF_ROUTED")

    def test_a_python_comment_that_names_the_origin_is_not_a_raw_fetch(self):
        self.write("scripts/bench/use.py", "import kf_trust\n# was https://download.example.org/pkg/tool.run\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/use.py"])})
        self.assertCodes()

    def test_a_python_docstring_that_names_the_origin_is_not_a_raw_fetch(self):
        self.write("scripts/bench/use.py", '"""Fetches https://download.example.org/pkg/tool.run through kf_trust."""\nimport kf_trust\n')
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/use.py"])})
        self.assertCodes()

    def test_a_comment_that_names_the_origin_is_not_a_raw_fetch(self):
        self.write("scripts/bench/use.sh", "#!/bin/bash\n# was https://download.example.org/pkg/tool.run\npython3 -I scripts/ci/kf_trust.py fetch tool\n")
        self.manifest()
        self.assertCodes()

    def test_a_native_consumer_needs_no_kf_trust_but_its_pin_file_must_hold_the_pin(self):
        self.write("scripts/bench/native.sh", "#!/bin/bash\nPIN=" + SHA + "\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/native.sh"], routing="native",
                                     pin_files=[{"file": "scripts/bench/native.sh", "contains": [SHA]}])})
        self.assertCodes()

    def test_a_pin_that_left_its_file_is_drift(self):
        self.write("scripts/bench/native.sh", "#!/bin/bash\nPIN=" + SHA2 + "\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/native.sh"], routing="native",
                                     pin_files=[{"file": "scripts/bench/native.sh", "contains": [SHA]}])})
        self.assertCodes("PIN_DRIFT")

    def test_a_string_missing_among_several_is_drift(self):
        self.write("scripts/bench/native.sh", "#!/bin/bash\nPIN=" + SHA + "\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/native.sh"], routing="native",
                                     pin_files=[{"file": "scripts/bench/native.sh", "contains": [SHA, "SIZE=999"]}])})
        self.assertCodes("PIN_DRIFT")

    def test_every_pin_file_of_an_entry_is_checked_not_only_the_first(self):
        self.write("scripts/bench/native.sh", "#!/bin/bash\nPIN=" + SHA + "\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/native.sh"], routing="native",
                                     pin_files=[{"file": "scripts/bench/native.sh", "contains": [SHA]},
                                                {"file": "scripts/bench/gone.sh", "contains": [SHA]}])})
        self.assertCodes("PIN_DRIFT")

    def test_a_pin_file_that_is_gone_is_drift(self):
        self.write("scripts/bench/native.sh", "#!/bin/bash\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/native.sh"], routing="native",
                                     pin_files=[{"file": "scripts/bench/gone.sh", "contains": [SHA]}])})
        self.assertCodes("PIN_DRIFT")

    def test_a_native_entry_must_name_its_pin_file(self):
        self.write("scripts/bench/native.sh", "#!/bin/bash\n")
        self.manifest({"tool": entry([pin()], consumers=["scripts/bench/native.sh"], routing="native")})
        self.assertCodes("MANIFEST_INVALID")

    def test_an_image_digest_on_another_images_from_line_is_drift(self):
        digest = "sha256:" + "c" * 64
        self.write("scripts/img/Dockerfile", "FROM ubuntu:22.04@" + digest + "\n")
        a = {"class": "upstream-pin", "digest": digest, "pin_origin": "registry-digest", "scope": "upstream-artifact",
             "pin_source_url": "https://hub.example.org/r", "status": "MEASURED", "measured": "2026-11-17"}
        e = {"row": 20, "kind": "oci-image", "routing": "native", "origin": "docker.io/nvidia/cuda:12.8.1-devel", "runs": "build",
             "consumers": ["scripts/img/Dockerfile"], "anchors": [a], "pin_files": [{"file": "scripts/img/Dockerfile", "contains": [digest]}]}
        self.manifest({"img": e})
        self.assertCodes("PIN_DRIFT")

    def test_an_image_entry_needs_its_digest_in_the_dockerfile_from_line(self):
        digest = "sha256:" + "c" * 64
        self.write("scripts/img/Dockerfile", "FROM nvidia/cuda:12.8.1-devel@" + digest + "\nRUN true\n")
        a = {"class": "upstream-pin", "digest": digest, "pin_origin": "registry-digest", "scope": "upstream-artifact",
             "pin_source_url": "https://hub.example.org/r", "status": "MEASURED", "measured": "2026-11-17"}
        e = {"row": 20, "kind": "oci-image", "routing": "native", "origin": "docker.io/nvidia/cuda:12.8.1-devel", "runs": "build",
             "consumers": ["scripts/img/Dockerfile"], "anchors": [a], "pin_files": [{"file": "scripts/img/Dockerfile", "contains": [digest]}]}
        self.manifest({"img": e})
        self.assertCodes()
        # the digest is held in a notes file, so only the FROM line check can notice that the Dockerfile moved on
        self.write("scripts/img/pins.txt", digest + "\n")
        e["pin_files"] = [{"file": "scripts/img/pins.txt", "contains": [digest]}]
        self.manifest({"img": e})
        self.assertCodes()
        self.write("scripts/img/Dockerfile", "FROM nvidia/cuda:12.8.1-devel@sha256:" + "d" * 64 + "\n")
        self.assertCodes("PIN_DRIFT")


class Scan(Tree):
    def test_a_new_script_with_a_curl_download_is_a_finding(self):
        self.write("scripts/bench/new.sh", "#!/bin/bash\ncurl -fsSL -o /tmp/x https://example.org/x\n")
        self.manifest()
        self.assertCodes("UNROUTED_DOWNLOAD")

    def test_a_new_python_urlopen_is_a_finding(self):
        self.write("scripts/bench/new.py", "import urllib.request\nurllib.request.urlopen('https://example.org')\n")
        self.manifest()
        self.assertCodes("UNROUTED_DOWNLOAD")

    def test_pip_install_wget_and_git_clone_are_downloads_too(self):
        for text in ("pip install torch\n", "wget -q https://example.org/x\n", "git clone https://example.org/r.git\n",
                     "git -C d fetch origin\n", "python3 -m pip install x\n"):
            with self.subTest(text):
                self.write("scripts/bench/new.sh", "#!/bin/bash\n" + text)
                self.manifest()
                self.assertCodes("UNROUTED_DOWNLOAD")

    def test_a_comment_that_mentions_curl_is_not_a_download(self):
        self.write("scripts/bench/c.sh", "#!/bin/bash\n# curl -fsSL https://example.org/x | sh   (the old way)\nTRUE=1\n")
        self.write("scripts/bench/c.py", "# urlopen('https://example.org')\nx = 1\n")
        self.manifest()
        self.assertCodes()

    def test_a_python_docstring_that_mentions_urlopen_is_not_a_download(self):
        self.write("scripts/bench/d.py", '"""This module never calls urlopen or curl -s."""\n\n\ndef f():\n    """urllib.request is not used."""\n    return 1\n')
        self.manifest()
        self.assertCodes()

    def test_a_package_name_in_an_apt_line_is_not_a_download(self):
        self.write("scripts/bench/p.sh", "#!/bin/bash\napt-get install -y git curl wget python3\nfor t in curl python3 git; do command -v $t; done\n")
        self.manifest()
        self.assertCodes()

    def test_test_files_and_test_directories_are_not_scanned(self):
        self.write("scripts/ci/test_x.py", "import urllib.request\n")
        self.write("scripts/bench/tests/t.sh", "#!/bin/bash\ncurl -s x\n")
        self.manifest()
        self.assertCodes()

    def test_mutant_files_and_markdown_are_not_scanned(self):
        self.write("scripts/ci/x_mutants.py", "import urllib.request\n")
        self.write("scripts/bench/README.md", "run `curl -fsSL https://x | sh`\n")
        self.manifest()
        self.assertCodes()

    def test_kf_trust_itself_is_the_one_exempt_downloader(self):
        self.write("scripts/ci/kf_trust.py", "import urllib.request\nurllib.request.urlopen('https://example.org')\n")
        self.manifest()
        self.assertCodes()

    def test_the_exempt_downloader_may_also_shell_out(self):
        self.write("scripts/ci/kf_trust.py", "import subprocess\nsubprocess.run(['curl', '-s', 'https://example.org'])\nX = 'git clone x'\n")
        self.manifest()
        self.assertCodes()

    def test_a_file_named_like_the_exempt_one_elsewhere_is_scanned(self):
        self.write("scripts/bench/kf_trust.py", "import urllib.request\nurllib.request.urlopen('https://example.org')\n")
        self.manifest()
        self.assertCodes("UNROUTED_DOWNLOAD")

    def test_a_new_python_urlopen_is_a_finding_in_a_file_called_kf_trust(self):
        self.write("scripts/tools/kf_trust.py", "import urllib.request\nurllib.request.urlopen('https://example.org')\n")
        self.manifest()
        self.assertCodes("UNROUTED_DOWNLOAD")

    def test_an_extensionless_python_script_is_scanned_too(self):
        self.write("scripts/ci/kf-box-y", "#!/usr/bin/env python3\nimport urllib.request\nurllib.request.urlopen('https://example.org')\n")
        self.manifest()
        self.assertCodes("UNROUTED_DOWNLOAD")

    def test_a_dependency_table_entry_is_not_a_download(self):
        self.write("scripts/bench/dep.sh", "#!/bin/bash\ndeclare -A PKG=([curl]=curl [git]=git)\nif command -v curl >/dev/null; then :; fi\n")
        self.manifest()
        self.assertCodes()

    def test_an_extensionless_script_with_a_shebang_is_scanned(self):
        self.write("scripts/ci/kf-box-x", "#!/bin/bash\ncurl --silent https://example.org\n")
        self.manifest()
        self.assertCodes("UNROUTED_DOWNLOAD")

    def test_a_listed_file_at_its_count_is_clean(self):
        self.write("scripts/bench/new.sh", "#!/bin/bash\ncurl -fsSL https://example.org/x\n")
        self.manifest(unrouted={"scripts/bench/new.sh": unrouted(1)})
        self.assertCodes()

    def test_a_listed_file_that_gained_a_download_is_a_finding(self):
        self.write("scripts/bench/new.sh", "#!/bin/bash\ncurl -fsSL https://example.org/x\ncurl -fsSL https://example.org/y\n")
        self.manifest(unrouted={"scripts/bench/new.sh": unrouted(1)})
        self.assertCodes("UNROUTED_DOWNLOAD")

    def test_a_listed_python_file_that_gained_a_download_is_a_finding(self):
        self.write("scripts/bench/new.py", "import urllib.request\nurllib.request.urlopen('https://example.org')\nurllib.request.urlopen('https://example.org/2')\n")
        self.manifest(unrouted={"scripts/bench/new.py": unrouted(3)})
        self.assertCodes()
        self.write("scripts/bench/new.py", "import urllib.request\nurllib.request.urlopen('https://a')\nurllib.request.urlopen('https://b')\nhttp.client.x\n")
        self.assertCodes("UNROUTED_DOWNLOAD")

    def test_a_listed_python_file_that_lost_a_download_is_stale(self):
        self.write("scripts/bench/new.py", "import urllib.request\n")
        self.manifest(unrouted={"scripts/bench/new.py": unrouted(2)})
        self.assertCodes("STALE_UNROUTED")

    def test_a_listed_file_that_lost_a_download_is_stale(self):
        self.write("scripts/bench/new.sh", "#!/bin/bash\ncurl -fsSL https://example.org/x\n")
        self.manifest(unrouted={"scripts/bench/new.sh": unrouted(2)})
        self.assertCodes("STALE_UNROUTED")

    def test_a_listed_file_without_any_download_left_is_stale(self):
        self.write("scripts/bench/new.sh", "#!/bin/bash\necho hi\n")
        self.manifest(unrouted={"scripts/bench/new.sh": unrouted(1)})
        self.assertCodes("STALE_UNROUTED")

    def test_a_listed_file_that_is_gone_is_stale(self):
        self.manifest(unrouted={"scripts/bench/gone.sh": unrouted(1)})
        self.assertCodes("STALE_UNROUTED")

    def test_a_short_reason_is_refused(self):
        self.write("scripts/bench/new.sh", "#!/bin/bash\ncurl -fsSL https://example.org/x\n")
        self.manifest(unrouted={"scripts/bench/new.sh": unrouted(reason="x")})
        self.assertIn("BAD_UNROUTED", self.codes())

    def test_an_entry_needs_a_class_a_count_and_a_reason(self):
        self.write("scripts/bench/new.sh", "#!/bin/bash\ncurl -fsSL https://example.org/x\n")
        for bad in (unrouted(cls="whatever"), unrouted(reason="short"), unrouted(count=0), {"class": "operator", "reason": "long enough reason"}):
            self.manifest(unrouted={"scripts/bench/new.sh": bad})
            self.assertIn("BAD_UNROUTED", self.codes(), bad)

    def test_a_directory_entry_counts_the_total_of_its_files(self):
        self.write("scripts/exp/a.sh", "#!/bin/bash\ngit fetch origin x\n")
        self.write("scripts/exp/b.sh", "#!/bin/bash\ngit fetch origin y\n")
        self.manifest(unrouted={"scripts/exp/": unrouted(2, "experiment-only")})
        self.assertCodes()
        self.write("scripts/exp/c.sh", "#!/bin/bash\ngit fetch origin z\n")
        self.assertCodes("UNROUTED_DOWNLOAD")

    def test_a_directory_entry_at_its_total_is_clean(self):
        self.write("scripts/exp/a.sh", "#!/bin/bash\ngit fetch origin x\n")
        self.manifest(unrouted={"scripts/exp/": unrouted(1, "experiment-only")})
        self.assertCodes()

    def test_a_directory_entry_that_lost_downloads_is_stale(self):
        self.write("scripts/exp/a.sh", "#!/bin/bash\ngit fetch origin x\n")
        self.manifest(unrouted={"scripts/exp/": unrouted(5, "experiment-only")})
        self.assertCodes("STALE_UNROUTED")

    def test_a_file_under_a_directory_entry_may_not_also_be_listed(self):
        self.write("scripts/exp/a.sh", "#!/bin/bash\ngit fetch origin x\n")
        self.manifest(unrouted={"scripts/exp/": unrouted(1, "experiment-only"), "scripts/exp/a.sh": unrouted(1)})
        self.assertIn("BAD_UNROUTED", self.codes())

    def test_a_routed_consumer_with_a_leftover_download_still_needs_its_own_entry(self):
        self.write("scripts/bench/use.sh", "#!/bin/bash\npython3 -I scripts/ci/kf_trust.py fetch tool --out x\ncurl -fsSL https://example.org/z\n")
        self.manifest()
        self.assertCodes("UNROUTED_DOWNLOAD")


class Dockerfiles(Tree):
    def test_a_from_without_a_digest_is_a_finding(self):
        self.write("scripts/img/Dockerfile", "FROM ubuntu:22.04\n")
        self.manifest()
        self.assertCodes("DOCKERFILE_UNPINNED")

    def test_a_from_with_a_digest_a_scratch_stage_or_an_earlier_stage_is_fine(self):
        self.write("scripts/img/a.Dockerfile", "FROM ubuntu:22.04@sha256:" + "e" * 64 + " AS build\nFROM build\nFROM scratch\n")
        self.manifest()
        self.assertCodes()

    def test_a_later_unpinned_stage_is_a_finding(self):
        self.write("scripts/img/Dockerfile", "FROM ubuntu:22.04@sha256:" + "e" * 64 + " AS build\nFROM alpine:3\n")
        self.manifest()
        self.assertCodes("DOCKERFILE_UNPINNED")


class RealTree(unittest.TestCase):
    def test_the_committed_tree_is_clean(self):
        findings = L.lint(L.ROOT)
        self.assertEqual(findings, [], "\n".join("%s %s: %s" % f for f in findings))

    def test_the_committed_manifest_lists_every_row_this_change_fixed(self):
        arts = json.loads((L.ROOT / "scripts/ci/trust_manifest.json").read_text())["artifacts"]
        rows = {e["row"] for e in arts.values()}
        self.assertTrue({3, 4, 16, 20, 24, 25, 29} <= rows, rows)

    def test_every_nvidia_installer_has_its_vendor_checksum_and_its_matching_pin(self):
        arts = json.loads((L.ROOT / "scripts/ci/trust_manifest.json").read_text())["artifacts"]
        runs = {n: e for n, e in arts.items() if n.startswith("nvidia-linux-run-")}
        self.assertGreaterEqual(len(runs), 14)
        for n, e in runs.items():
            classes = [a["class"] for a in e["anchors"]]
            self.assertIn("upstream-pin", classes, n)
            if n == "nvidia-linux-run-570.148.08":
                # served only from tesla/ and with no .sha256sum: the pin of the measured upstream binary, awaiting the owner
                self.assertIn("/tesla/", e["origin"])
                self.assertNotIn("official-checksum-url", classes)
                continue
            self.assertNotIn("/tesla/", e["origin"], n)
            self.assertIn("official-checksum-url", classes, n)
            pin_anchor = next(a for a in e["anchors"] if a["class"] == "upstream-pin")
            self.assertEqual(pin_anchor["pin_origin"], "vendor-published", n)
            official = next(a for a in e["anchors"] if a["class"] == "official-checksum-url")
            self.assertEqual(official["url"], e["origin"] + ".sha256sum", n)

    def test_the_pins_of_the_pending_artifacts_are_listed_for_the_owner(self):
        pending = L.pending()
        self.assertIn("virtualgl-3.1.3-amd64-deb", pending)
        self.assertNotIn("nvidia-linux-run-580.159.04", pending)


class Cli(Tree):
    def run_cli(self, *args):
        return subprocess.run([sys.executable, "-I", "-B", str(HERE / "trust_lint.py"), "--root", str(self.root), *args], capture_output=True, text=True)

    def test_exit_zero_when_clean_and_one_with_a_named_line_when_not(self):
        self.manifest()
        ok = self.run_cli()
        self.assertEqual((ok.returncode, ok.stdout.strip()), (0, "TRUST_LINT OK (0 finding(s))"))
        self.write("scripts/bench/new.sh", "#!/bin/bash\ncurl -fsSL https://example.org/x\n")
        bad = self.run_cli()
        self.assertEqual(bad.returncode, 1)
        self.assertIn("TRUST_LINT UNROUTED_DOWNLOAD scripts/bench/new.sh:", bad.stdout)

    def test_a_forbidden_anchor_exits_one_with_its_name(self):
        self.manifest({"tool": entry([{"class": "git-sha256-of-our-own-artifact", "sha256": SHA}])})
        r = self.run_cli()
        self.assertEqual(r.returncode, 1)
        self.assertIn("TRUST_LINT FORBIDDEN_ANCHOR tool:", r.stdout)

    def test_pending_flag_lists_names_and_exits_zero(self):
        self.manifest({"a": entry([pin(pin_origin="measured-from-upstream")])})
        r = self.run_cli("--pending")
        self.assertEqual((r.returncode, r.stdout.split()), (0, ["a"]))


if __name__ == "__main__":
    unittest.main()

# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""open_rows.json policy and the download anchors (kfopen_pins.py). GPU-free; the owner rule of 2026-11-17 is what these tests pin."""
import copy
import json
import os
import re
import unittest

import helpers as H
import kfopen_pins as P
from kfopen_common import OpenRowError, Trust

REL = H.fixture("github_release_mingit.json")
PREFIX = "https://github.com/git-for-windows/git/releases/download/v2.56.1.windows.1/"
A = "a" * 64


def pins():
    return copy.deepcopy(P.load_pins())


class PinsPolicy(unittest.TestCase):
    def test_committed_pins_are_valid(self):
        doc = pins()
        self.assertEqual({"cts", "dxs"}, set(doc["sources"]))
        self.assertEqual(10, len(doc["rows"]))

    def test_sources_are_anchored_by_a_pinned_commit(self):
        for name, src in pins()["sources"].items():
            self.assertRegex(src["commit"], r"^[0-9a-f]{40}$", name)
            self.assertEqual("git-commit", src["anchor"])

    def test_dxs_commit_is_the_one_of_the_existing_source_built_rows(self):
        rel = json.load(open(os.path.join(H.AM, "release_software.json")))
        self.assertEqual(rel["directx"]["commit"], pins()["sources"]["dxs"]["commit"])

    def test_no_binary_digest_is_pinned_in_git(self):
        doc = pins()
        raw = open(os.path.join(H.OR, "open_rows.json")).read()
        found = set(re.findall(r"\b[0-9a-fA-F]{64}\b", raw))
        self.assertEqual(set(doc["nuget_trust"]["fingerprints_sha256"]), found, "only the nuget.org certificate fingerprints are 64-hex values")
        self.assertIsNone(re.search(r"\b[0-9a-fA-F]{64}\b", json.dumps(doc["tools"])), "a tool entry holds a 64-hex digest")

    def test_a_binary_digest_in_the_tool_data_is_refused(self):
        doc = pins()
        doc["tools"]["git"]["note"] = "sha " + A
        with self.assertRaisesRegex(OpenRowError, "binary digest"):
            P.validate_pins(doc)

    def test_a_digest_key_in_the_tool_data_is_refused(self):
        doc = pins()
        doc["tools"]["nuget"]["sha256"] = "pending"
        with self.assertRaisesRegex(OpenRowError, "binary digest"):
            P.validate_pins(doc)

    def test_a_tool_cannot_use_the_attestation_or_commit_anchor_yet(self):
        doc = pins()
        doc["tools"]["nuget"]["anchor"] = "ci-attestation"
        with self.assertRaises(OpenRowError):
            P.validate_pins(doc)

    def test_http_tool_url_refused(self):
        doc = pins()
        doc["tools"]["nuget"]["url"] = "http://dist.nuget.org/win-x86-commandline/v7.9.0/nuget.exe"
        with self.assertRaisesRegex(OpenRowError, "https"):
            P.validate_pins(doc)

    def test_http_pypi_json_refused(self):
        doc = pins()
        doc["tools"]["lxml"]["pypi_json"] = "http://pypi.org/pypi/lxml/6.1.3/json"
        with self.assertRaisesRegex(OpenRowError, "https"):
            P.validate_pins(doc)

    def test_authenticode_needs_a_named_signer(self):
        doc = pins()
        del doc["tools"]["msvc"]["signer"]
        with self.assertRaisesRegex(OpenRowError, "signer"):
            P.validate_pins(doc)

    def test_nuget_trust_needs_fingerprints_and_required_mode(self):
        doc = pins()
        doc["nuget_trust"]["fingerprints_sha256"] = []
        with self.assertRaises(OpenRowError):
            P.validate_pins(doc)
        doc = pins()
        doc["nuget_trust"]["signature_validation_mode"] = "accept"
        with self.assertRaises(OpenRowError):
            P.validate_pins(doc)

    def test_fingerprints_must_be_uppercase_sha256_hex(self):
        doc = pins()
        doc["nuget_trust"]["fingerprints_sha256"] = ["abcdef"]
        with self.assertRaises(OpenRowError):
            P.validate_pins(doc)
        doc = pins()
        doc["nuget_trust"]["fingerprints_sha256"] = [f.lower() for f in doc["nuget_trust"]["fingerprints_sha256"]]
        with self.assertRaises(OpenRowError):
            P.validate_pins(doc)

    def test_a_not_run_row_names_its_reason(self):
        doc = pins()
        doc["rows"]["dxsamples_multiadapter"]["reason"] = "Needs two"
        with self.assertRaisesRegex(OpenRowError, "reason"):
            P.validate_pins(doc)

    def test_sparse_dirs_cover_every_sample_project(self):
        doc = pins()
        dirs = P.sparse_dirs(doc, "dxs")
        for key, spec in doc["dxs_samples"].items():
            self.assertTrue(any(spec["project"].startswith(d + "/") for d in dirs), key)
        self.assertIsNone(P.sparse_dirs(doc, "cts"))

    def test_a_sample_checkout_must_hold_its_project(self):
        doc = pins()
        doc["dxs_samples"]["hdr"]["checkout"] = "Samples/Desktop/D3D12nBodyGravity"
        with self.assertRaisesRegex(OpenRowError, "checkout"):
            P.validate_pins(doc)

    def test_unknown_row_kind_and_sample_refused(self):
        doc = pins()
        doc["rows"]["dxsamples_hdr"]["sample"] = "nope"
        with self.assertRaisesRegex(OpenRowError, "sample"):
            P.validate_pins(doc)
        doc = pins()
        doc["rows"]["vkglcts_vk"]["kind"] = "cts-metal"
        with self.assertRaisesRegex(OpenRowError, "kind"):
            P.validate_pins(doc)


class CommitPin(unittest.TestCase):
    def test_check_commit_refuses_mismatch(self):
        with self.assertRaisesRegex(P.PinMismatch, "PIN_MISMATCH"):
            P.check_commit("cts", "f" * 40 + "\n", "0" * 40)

    def test_check_commit_accepts_the_pin_and_trims_the_newline(self):
        self.assertTrue(P.check_commit("cts", "0" * 40 + "\n", "0" * 40))

    def test_an_override_replaces_the_pin_only_when_given(self):
        doc = pins()
        self.assertEqual(doc["sources"]["cts"]["commit"], P.resolve_commit(doc, "cts", Trust()))
        self.assertEqual("1" * 40, P.resolve_commit(doc, "cts", Trust(commit_overrides={"cts": "1" * 40})))
        self.assertEqual(doc["sources"]["dxs"]["commit"], P.resolve_commit(doc, "dxs", Trust(commit_overrides={"cts": "1" * 40})))

    def test_override_flag_syntax_is_strict(self):
        class A_:
            source_commit_override = ["cts=1234"]
            tool = accept_unanchored_downloads = prebuilt = None
        with self.assertRaises(OpenRowError):
            Trust.from_args(A_)

    def test_lifting_needs_a_reason(self):
        class A_:
            source_commit_override = tool = prebuilt = None
            accept_unanchored_downloads = "  "
        with self.assertRaises(OpenRowError):
            Trust.from_args(A_)

    def test_trust_record_says_git_only_until_something_is_lifted(self):
        self.assertEqual("git-only", Trust().record()["mode"])
        self.assertEqual("lifted", Trust(accept_unanchored="mirror").record()["mode"])
        self.assertEqual("lifted", Trust(user_tools={"git": "C:\\g"}).record()["mode"])


class Anchors(unittest.TestCase):
    def test_table_reads_markdown_and_sha256sum_formats(self):
        self.assertEqual(A, P.parse_checksum_table("| MinGit-1.zip | %s |" % A, "MinGit-1.zip"))
        self.assertEqual(A, P.parse_checksum_table("%s  MinGit-1.zip\n" % A, "MinGit-1.zip"))
        self.assertEqual(A, P.parse_checksum_table("%s *MinGit-1.zip" % A.upper(), "MinGit-1.zip"))

    def test_table_does_not_match_longer_asset_name(self):
        text = "%s  MinGit-1.zip.sig\n" % ("b" * 64)
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_MISSING"):
            P.parse_checksum_table(text, "MinGit-1.zip")

    def test_sig_file_hash_not_taken_for_the_asset(self):
        text = "%s  MinGit-1.zip.sig\n%s  MinGit-1.zip\n" % ("b" * 64, A)
        self.assertEqual(A, P.parse_checksum_table(text, "MinGit-1.zip"))

    def test_table_with_two_different_hashes_refused(self):
        text = "%s  MinGit-1.zip\n%s  MinGit-1.zip\n" % (A, "b" * 64)
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_AMBIGUOUS"):
            P.parse_checksum_table(text, "MinGit-1.zip")

    def test_release_gives_url_and_the_agreed_hash(self):
        url, sha = P.expected_from_github_release(REL, "MinGit-2.56.1-64-bit.zip", PREFIX)
        self.assertEqual(PREFIX + "MinGit-2.56.1-64-bit.zip", url)
        self.assertEqual(A, sha)

    def test_release_asset_url_outside_pinned_release_refused(self):
        rel = json.loads(REL)
        rel["assets"][0]["browser_download_url"] = "https://evil.example/MinGit-2.56.1-64-bit.zip"
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_ORIGIN"):
            P.expected_from_github_release(rel, "MinGit-2.56.1-64-bit.zip", PREFIX)

    def test_release_asset_from_other_tag_refused(self):
        rel = json.loads(REL)
        rel["assets"][0]["browser_download_url"] = rel["assets"][0]["browser_download_url"].replace("v2.56.1.windows.1", "v9.9.9.windows.1")
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_ORIGIN"):
            P.expected_from_github_release(rel, "MinGit-2.56.1-64-bit.zip", PREFIX)

    def test_conflict_between_notes_and_digest_refused(self):
        rel = json.loads(REL)
        rel["assets"][0]["digest"] = "sha256:" + "d" * 64
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_CONFLICT"):
            P.expected_from_github_release(rel, "MinGit-2.56.1-64-bit.zip", PREFIX)

    def test_release_without_any_checksum_refused(self):
        rel = json.loads(REL)
        rel["body"], rel["assets"][0]["digest"] = "no table", None
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_MISSING"):
            P.expected_from_github_release(rel, "MinGit-2.56.1-64-bit.zip", PREFIX)

    def test_ambiguous_notes_are_not_rescued_by_the_digest_field(self):
        rel = json.loads(REL)
        rel["body"] += "MinGit-2.56.1-64-bit.zip | " + "e" * 64 + "\n"
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_AMBIGUOUS"):
            P.expected_from_github_release(rel, "MinGit-2.56.1-64-bit.zip", PREFIX)

    def test_notes_alone_or_digest_alone_are_enough(self):
        rel = json.loads(REL)
        rel["assets"][0]["digest"] = None
        self.assertEqual(A, P.expected_from_github_release(rel, "MinGit-2.56.1-64-bit.zip", PREFIX)[1])
        rel = json.loads(REL)
        rel["body"] = ""
        self.assertEqual(A, P.expected_from_github_release(rel, "MinGit-2.56.1-64-bit.zip", PREFIX)[1])

    def test_pypi_picks_only_the_pinned_wheel(self):
        name, url, sha = P.pypi_wheel_hash(H.fixture("pypi_lxml.json"), "6.1.3")
        self.assertEqual("lxml-6.1.3-cp312-cp312-win_amd64.whl", name)
        self.assertEqual("3" * 64, sha)

    def test_pypi_json_of_another_version_refused(self):
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_MISSING"):
            P.pypi_wheel_hash(H.fixture("pypi_lxml.json"), "6.1.2")

    def test_pypi_wheel_from_other_host_refused(self):
        doc = H.fixture_json("pypi_lxml.json")
        doc["urls"][0]["url"] = "https://mirror.example/lxml-6.1.3-cp312-cp312-win_amd64.whl"
        with self.assertRaisesRegex(OpenRowError, "ANCHOR_ORIGIN"):
            P.pypi_wheel_hash(doc, "6.1.3")

    def test_pypi_two_matching_wheels_refused(self):
        doc = H.fixture_json("pypi_lxml.json")
        doc["urls"].append(copy.deepcopy(doc["urls"][0]))
        with self.assertRaisesRegex(OpenRowError, "need exactly one"):
            P.pypi_wheel_hash(doc, "6.1.3")

    def test_pypi_wheel_without_sha256_refused(self):
        doc = H.fixture_json("pypi_lxml.json")
        doc["urls"][0]["digests"] = {"md5": "x"}
        with self.assertRaisesRegex(OpenRowError, "no sha256"):
            P.pypi_wheel_hash(doc, "6.1.3")

    def test_requirements_line_carries_the_hash(self):
        self.assertEqual("lxml==6.1.3 --hash=sha256:%s\n" % A, P.requirements_line("lxml", "6.1.3", A))

    def test_valid_signature_of_the_pinned_signer_passes(self):
        rep = {"Status": "Valid", "Subject": "CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US"}
        self.assertTrue(P.authenticode_check(rep, "Microsoft Corporation"))
        self.assertTrue(P.authenticode_check(json.dumps(dict(rep, Status="0")), "Microsoft Corporation"))

    def test_invalid_signature_refused(self):
        for status in ("NotSigned", "HashMismatch", "UnknownError", ""):
            with self.assertRaisesRegex(OpenRowError, "SIGNATURE_INVALID"):
                P.authenticode_check({"Status": status, "Subject": "CN=Microsoft Corporation"}, "Microsoft Corporation")

    def test_wrong_signer_refused(self):
        with self.assertRaisesRegex(OpenRowError, "SIGNATURE_SIGNER"):
            P.authenticode_check({"Status": "Valid", "Subject": "CN=Totally Legit Software Ltd, O=Totally Legit Software Ltd"}, "Microsoft Corporation")
        with self.assertRaisesRegex(OpenRowError, "SIGNATURE_SIGNER"):
            P.authenticode_check({"Status": "Valid", "Subject": "CN=Not Microsoft Corporation Inc"}, "Microsoft Corporation")

    def test_nuget_config_requires_signatures(self):
        xml = P.nuget_config_xml(pins()["nuget_trust"])
        self.assertIn('key="signatureValidationMode" value="require"', xml)
        self.assertIn("<clear />", xml)

    def test_nuget_config_lists_all_fingerprints(self):
        trust = pins()["nuget_trust"]
        xml = P.nuget_config_xml(trust)
        for f in trust["fingerprints_sha256"]:
            self.assertIn(f, xml)
        self.assertIn('allowUntrustedRoot="false"', xml)
        self.assertIn("https://api.nuget.org/v3/index.json", xml)

    def test_nuget_config_is_well_formed_xml(self):
        import xml.dom.minidom
        xml.dom.minidom.parseString(P.nuget_config_xml(pins()["nuget_trust"]))

    def test_hash_mismatch_refused(self):
        with self.assertRaisesRegex(OpenRowError, "HASH_MISMATCH"):
            P.verify_sha256("a" * 64, "b" * 64)
        self.assertTrue(P.verify_sha256("A" * 64, "a" * 64))


if __name__ == "__main__":
    unittest.main()

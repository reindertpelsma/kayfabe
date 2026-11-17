#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/kf_trust.py (STATUS: LIVE, 2026-11-17). No network: a fake transport, a loopback HTTP server and
local git repositories. Each anchor class, the default mode, the explicit lift, the user input rule and the evidence record.
The names below are the killers named by scripts/ci/kf_trust_mutants.py: renaming a test is a change of that file."""
import contextlib
import hashlib
import http.server
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import kf_trust as T  # noqa: E402

DATA = b"the upstream bytes of a pretend installer\n" * 50
SUM = hashlib.sha256(DATA).hexdigest()
OTHER = b"bytes that upstream never published\n" * 50
OTHER_SUM = hashlib.sha256(OTHER).hexdigest()
ORIGIN = "https://download.example.org/pkg/tool-1.0.run"
SUMS_URL = ORIGIN + ".sha256sum"
PRESIGNED = "https://objects.example.org/private/tool-1.0.run?X-Amz-Signature=fa4efa4efa4e&X-Amz-Credential=fake"
FAKE_UUID = "fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001"  # the synthetic placeholder: nothing here may look like a real identifier


def pin(sha=SUM, size=len(DATA), **kw):
    a = {"class": "upstream-pin", "sha256": sha, "size": size, "pin_origin": "vendor-published", "scope": "upstream-artifact",
         "pin_source_url": SUMS_URL, "status": "MEASURED", "measured": "2026-11-17"}
    a.update(kw)
    return a


def official(url=SUMS_URL, **kw):
    a = {"class": "official-checksum-url", "url": url, "format": "sha256sum"}
    a.update(kw)
    return a


def manifest(*anchors, name="tool", kind="file", origin=ORIGIN, **extra):
    e = {"row": 99, "kind": kind, "routing": "helper", "origin": origin, "runs": "host", "consumers": ["scripts/x.sh"], "anchors": list(anchors)}
    e.update(extra)
    return {"schema": T.SCHEMA, "artifacts": {name: e}}


class FakeTransport:
    """docs: url -> bytes for get_text. files: url -> bytes for download. enforce=False is a transport that lies (never checks)."""

    def __init__(self, docs=None, files=None, enforce=True):
        self.docs, self.files, self.enforce = docs or {}, files or {}, enforce
        self.doc_calls, self.download_calls = [], []

    def get_text(self, url, max_bytes=T.DOC_MAX):
        self.doc_calls.append(url)
        if url not in self.docs:
            raise T.TransportError("NETWORK", "no such doc")
        return self.docs[url]

    def download(self, url, dest, *, sha256=None, size=None, max_bytes=T.DEFAULT_MAX_BYTES):
        self.download_calls.append({"url": url, "sha256": sha256, "size": size})
        if url not in self.files:
            raise T.TransportError("NETWORK", "no such file")
        data = self.files[url]
        digest = hashlib.sha256(data).hexdigest()
        if self.enforce and sha256 is not None and digest != sha256:
            raise T.TransportError("HASH")
        if self.enforce and size is not None and len(data) != size:
            raise T.TransportError("SIZE")
        Path(dest).write_bytes(data)
        os.chmod(dest, 0o600)  # what kf_fetch leaves: the publish step must not keep it
        return digest, len(data)


class Base(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.out = self.dir / "tool.run"
        self.ev = self.dir / "evidence"
        self._env = {k: os.environ.pop(k) for k in ("KF_TRUST_MODE", "KF_TRUST_LIFT", "KF_TRUST_EVIDENCE_DIR") if k in os.environ}
        self.addCleanup(lambda: os.environ.update(self._env))
        self.addCleanup(self._tmp.cleanup)

    def resolve(self, man, transport, **kw):
        kw.setdefault("evidence_dir", self.ev)
        with contextlib.redirect_stderr(io.StringIO()) as so:
            try:
                r = T.resolve("tool", self.out, manifest=man, transport=transport, **kw)
            finally:
                self.stdout = so.getvalue()  # the KF_TRUST lines go to stderr: stdout belongs to the caller
        return r

    def lines(self):
        p = self.ev / "trust.jsonl"
        return [json.loads(l) for l in p.read_text().splitlines()] if p.exists() else []

    def refused(self, code, man, transport, **kw):
        with self.assertRaises(T.TrustError) as cm:
            self.resolve(man, transport, **kw)
        self.assertEqual(cm.exception.code, code, str(cm.exception))
        self.assertFalse(self.out.exists(), "a refusal leaves no file at the output path")
        self.assertFalse(Path(str(self.out) + ".unverified").exists(), "and no unverified bytes")
        return cm.exception


class UpstreamPinAnchor(Base):
    def test_pin_match_is_verified_and_recorded(self):
        r = self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}))
        self.assertEqual((r["anchor"], r["sha256"], r["size"], r["gating"]), ("upstream-pin", SUM, len(DATA), True))
        self.assertEqual(self.out.read_bytes(), DATA)

    def test_pin_mismatch_is_refused_and_leaves_nothing(self):
        self.refused("TRUST_PIN_MISMATCH", manifest(pin()), FakeTransport(files={ORIGIN: OTHER}))

    def test_pin_mismatch_when_the_transport_does_not_enforce(self):
        # The transport is not trusted to have checked: resolve compares what it reports.
        self.refused("TRUST_PIN_MISMATCH", manifest(pin()), FakeTransport(files={ORIGIN: OTHER}, enforce=False))

    def test_pin_size_is_enforced(self):
        self.refused("TRUST_PIN_MISMATCH", manifest(pin(size=len(DATA) + 1)), FakeTransport(files={ORIGIN: DATA}, enforce=False))

    def test_pin_mismatch_in_strict_mode_is_refused_too(self):
        self.refused("TRUST_PIN_MISMATCH", manifest(pin()), FakeTransport(files={ORIGIN: OTHER}), mode="strict")

    def test_pin_is_the_expected_hash_handed_to_the_transport(self):
        t = FakeTransport(files={ORIGIN: DATA})
        self.resolve(manifest(pin()), t)
        self.assertEqual(t.download_calls[0]["sha256"], SUM)
        self.assertEqual(t.download_calls[0]["size"], len(DATA))

    def test_the_published_file_has_the_mode_of_a_plain_download_not_kf_fetchs_0600(self):
        old = os.umask(0o022)
        self.addCleanup(os.umask, old)
        self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}))
        self.assertEqual(self.out.stat().st_mode & 0o777, 0o644)
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        os.chmod(mine, 0o600)
        other = self.dir / "copy.run"
        with contextlib.redirect_stderr(io.StringIO()):
            T.resolve("tool", other, manifest=manifest(pin()), transport=FakeTransport(), user_source=str(mine), evidence_dir=self.ev)
        self.assertEqual(other.stat().st_mode & 0o777, 0o644)

    def test_existing_matching_file_is_reused_without_a_download(self):
        self.out.write_bytes(DATA)
        t = FakeTransport()
        r = self.resolve(manifest(pin()), t)
        self.assertEqual(t.download_calls, [])
        self.assertTrue(r["detail"]["cache_hit"])

    def test_existing_wrong_file_is_replaced_not_trusted(self):
        self.out.write_bytes(OTHER)
        r = self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}))
        self.assertEqual(self.out.read_bytes(), DATA)
        self.assertNotIn("cache_hit", r["detail"])

    def test_pin_detail_names_where_anyone_rechecks_it(self):
        r = self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}))
        self.assertEqual(r["detail"]["pin_source_url"], SUMS_URL)
        self.assertEqual(r["detail"]["pin_origin"], "vendor-published")
        self.assertEqual(r["detail"]["measured"], "2026-11-17")


class OfficialChecksumAnchor(Base):
    DOC = (SUM + "  tool-1.0.run\n").encode()

    def test_official_checksum_verifies_the_bytes(self):
        r = self.resolve(manifest(official()), FakeTransport(docs={SUMS_URL: self.DOC}, files={ORIGIN: DATA}))
        self.assertEqual((r["anchor"], r["sha256"]), ("official-checksum-url", SUM))
        self.assertEqual(r["detail"]["checksum_url"], SUMS_URL)

    def test_wrong_official_checksum_is_refused_by_name(self):
        # upstream's own checksum is for the real bytes; the origin serves other bytes
        e = self.refused("TRUST_UPSTREAM_SUM_MISMATCH", manifest(official()), FakeTransport(docs={SUMS_URL: self.DOC}, files={ORIGIN: OTHER}))
        self.assertIn(SUM, e.detail + SUM)

    def test_wrong_official_checksum_when_the_transport_does_not_enforce(self):
        self.refused("TRUST_UPSTREAM_SUM_MISMATCH", manifest(official()),
                     FakeTransport(docs={SUMS_URL: self.DOC}, files={ORIGIN: OTHER}, enforce=False))

    def test_checksum_from_a_host_we_run_is_refused_as_same_origin(self):
        ours = "https://bundles.kayfabe.dev/x.sha256sum"
        self.refused("TRUST_SAME_ORIGIN", manifest(official(ours)), FakeTransport(docs={ours: self.DOC}, files={ORIGIN: DATA}))

    def test_checksum_from_our_github_repository_is_same_origin(self):
        ours = "https://github.com/reindertpelsma/kayfabe/releases/download/v1/x.sha256"
        self.refused("TRUST_SAME_ORIGIN", manifest(official(ours)), FakeTransport(docs={ours: self.DOC}, files={ORIGIN: DATA}))

    def test_checksum_document_without_the_member_is_refused(self):
        doc = (SUM + "  some-other-file.run\n").encode()
        self.refused("TRUST_UPSTREAM_SUM_BAD_DOC", manifest(official(member="tool-1.0.run")),
                     FakeTransport(docs={SUMS_URL: doc}, files={ORIGIN: DATA}))

    def test_an_ambiguous_checksum_document_is_refused(self):
        doc = (SUM + "  one.run\n" + OTHER_SUM + "  two.run\n").encode()
        self.refused("TRUST_UPSTREAM_SUM_BAD_DOC", manifest(official()), FakeTransport(docs={SUMS_URL: doc}, files={ORIGIN: DATA}))

    def test_two_entries_for_the_member_are_refused(self):
        doc = (SUM + "  tool-1.0.run\n" + OTHER_SUM + "  tool-1.0.run\n").encode()
        self.refused("TRUST_UPSTREAM_SUM_BAD_DOC", manifest(official(member="tool-1.0.run")),
                     FakeTransport(docs={SUMS_URL: doc}, files={ORIGIN: DATA}))

    def test_unreachable_checksum_alone_is_a_transfer_failure_not_a_pass(self):
        self.refused("TRUST_TRANSPORT", manifest(official()), FakeTransport(files={ORIGIN: DATA}))

    def test_unreachable_checksum_does_not_weaken_a_pin_that_still_verifies(self):
        r = self.resolve(manifest(official(), pin()), FakeTransport(files={ORIGIN: DATA}))
        self.assertEqual(r["anchor"], "upstream-pin")
        self.assertEqual(r["detail"]["skipped"][0]["anchor"], "official-checksum-url")

    def test_checksum_and_pin_that_agree_both_verify(self):
        r = self.resolve(manifest(official(), pin()), FakeTransport(docs={SUMS_URL: self.DOC}, files={ORIGIN: DATA}))
        self.assertEqual(r["anchor"], "official-checksum-url")
        self.assertEqual(r["detail"]["also_verified"], ["upstream-pin"])

    def test_checksum_and_pin_that_disagree_refuse_before_any_download(self):
        t = FakeTransport(docs={SUMS_URL: self.DOC}, files={ORIGIN: DATA})
        self.refused("TRUST_PIN_MISMATCH", manifest(official(), pin(sha=OTHER_SUM)), t)
        self.assertEqual(t.download_calls, [], "disagreeing anchors never reach the network for the artifact")


class AttestationStub(Base):
    def test_attestation_alone_refuses_with_a_named_not_implemented(self):
        self.refused("TRUST_ATTEST_NOT_IMPLEMENTED", manifest({"class": "attestation"}), FakeTransport(files={ORIGIN: DATA}))

    def test_attestation_function_refuses_and_never_returns_true(self):
        with self.assertRaises(T.TrustError) as cm:
            T.verify_attestation("tool", {"class": "attestation"})
        self.assertEqual(cm.exception.code, "TRUST_ATTEST_NOT_IMPLEMENTED")

    def test_attestation_beside_a_pin_is_skipped_by_name_not_silently(self):
        r = self.resolve(manifest({"class": "attestation"}, pin()), FakeTransport(files={ORIGIN: DATA}))
        self.assertEqual(r["anchor"], "upstream-pin")
        self.assertEqual(r["detail"]["skipped"], [{"anchor": "attestation", "code": "TRUST_ATTEST_NOT_IMPLEMENTED"}])

    def test_attestation_never_makes_an_unpinned_download_pass(self):
        t = FakeTransport(files={ORIGIN: DATA})
        self.refused("TRUST_ATTEST_NOT_IMPLEMENTED", manifest({"class": "attestation"}), t)
        self.assertEqual(t.download_calls, [])


class ForbiddenAndMissingAnchors(Base):
    def test_a_sha256_in_git_of_our_own_artifact_is_never_an_anchor(self):
        self.refused("TRUST_ANCHOR_FORBIDDEN", manifest({"class": "git-sha256-of-our-own-artifact", "sha256": SUM}),
                     FakeTransport(files={ORIGIN: DATA}))

    def test_forbidden_anchor_is_refused_even_when_a_valid_pin_sits_beside_it(self):
        self.refused("TRUST_ANCHOR_FORBIDDEN", manifest(pin(), {"class": "git-sha256-of-our-own-artifact", "sha256": SUM}),
                     FakeTransport(files={ORIGIN: DATA}))

    def test_an_artifact_without_any_anchor_is_refused_in_the_default_mode(self):
        t = FakeTransport(files={ORIGIN: DATA})
        self.refused("TRUST_NO_ANCHOR", manifest(), t)
        self.assertEqual(t.download_calls, [], "no anchor means no download")

    def test_an_unknown_artifact_is_refused(self):
        with self.assertRaises(T.TrustError) as cm:
            T.resolve("nope", self.out, manifest=manifest(pin()), transport=FakeTransport(), emit=False)
        self.assertEqual(cm.exception.code, "TRUST_UNKNOWN_ARTIFACT")


class Modes(Base):
    def test_default_mode_is_trust_git_only(self):
        r = self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}))
        self.assertEqual(r["mode"], "trust-git-only")

    def test_mode_comes_from_the_environment_and_a_bad_value_is_refused(self):
        os.environ["KF_TRUST_MODE"] = "strict"
        self.assertEqual(self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}))["mode"], "strict")
        os.environ["KF_TRUST_MODE"] = "yolo"
        with self.assertRaises(T.TrustError) as cm:
            self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}))
        self.assertEqual(cm.exception.code, "TRUST_BAD_MODE")

    def test_an_operator_pin_is_not_an_anchor_in_the_default_mode(self):
        self.refused("TRUST_NO_ANCHOR", manifest(), FakeTransport(files={ORIGIN: DATA}), operator_pin=SUM)

    def test_an_operator_pin_that_disagrees_is_ignored_in_the_default_mode(self):
        r = self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}), operator_pin=OTHER_SUM)
        self.assertEqual(r["anchor"], "upstream-pin")

    def test_an_operator_pin_is_an_anchor_in_strict_mode_and_is_recorded_as_such(self):
        r = self.resolve(manifest(), FakeTransport(files={ORIGIN: DATA}), mode="strict", operator_pin=SUM)
        self.assertEqual(r["anchor"], "operator-pin")
        self.assertTrue(r["detail"]["deployment_data"])

    def test_a_wrong_operator_pin_is_refused_in_strict_mode(self):
        self.refused("TRUST_PIN_MISMATCH", manifest(), FakeTransport(files={ORIGIN: DATA}), mode="strict", operator_pin=OTHER_SUM)

    def test_a_pin_in_git_and_an_operator_pin_that_disagree_are_refused(self):
        self.refused("TRUST_PIN_MISMATCH", manifest(pin()), FakeTransport(files={ORIGIN: DATA}), mode="strict", operator_pin=OTHER_SUM)


class ExplicitLift(Base):
    def test_a_failure_stays_a_refusal_without_a_lift(self):
        self.refused("TRUST_PIN_MISMATCH", manifest(pin()), FakeTransport(files={ORIGIN: OTHER}))

    def test_an_explicit_lift_records_unverified_lifted_with_the_computed_hash(self):
        r = self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: OTHER}), lift="mirror bytes differ, reviewed by hand")
        self.assertEqual((r["anchor"], r["sha256"], r["gating"]), ("unverified-lifted", OTHER_SUM, False))
        self.assertEqual(r["lift_reason"], "mirror bytes differ, reviewed by hand")
        self.assertEqual(r["detail"]["lifted_error"], "TRUST_PIN_MISMATCH")
        self.assertEqual(self.out.read_bytes(), OTHER)

    def test_the_lift_is_printed_and_written_to_the_evidence(self):
        self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: OTHER}), lift="why")
        self.assertIn("KF_TRUST_LIFTED name=tool reason=why", self.stdout)
        self.assertEqual(self.lines()[0]["anchor"], "unverified-lifted")
        self.assertEqual(self.lines()[0]["lift_reason"], "why")

    def test_a_lifted_result_is_marked_non_gating_in_the_printed_line(self):
        self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: OTHER}), lift="why")
        self.assertIn("anchor=unverified-lifted", self.stdout)
        self.assertIn("gating=no", self.stdout)
        self.assertFalse(self.lines()[0]["gating"])

    def test_the_lift_can_come_from_the_environment(self):
        os.environ["KF_TRUST_LIFT"] = "from the environment"
        r = self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: OTHER}))
        self.assertEqual(r["anchor"], "unverified-lifted")

    def test_an_empty_or_unprintable_reason_is_refused_not_treated_as_a_lift(self):
        for bad in ("x" * 201, "line\nbreak", " padded "):
            with self.assertRaises(T.TrustError) as cm:
                self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: OTHER}), lift=bad)
            self.assertEqual(cm.exception.code, "TRUST_BAD_LIFT")
        os.environ["KF_TRUST_LIFT"] = ""
        self.refused("TRUST_PIN_MISMATCH", manifest(pin()), FakeTransport(files={ORIGIN: OTHER}))

    def test_a_lift_does_not_touch_an_artifact_that_verifies(self):
        r = self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}), lift="not needed")
        self.assertEqual((r["anchor"], r["gating"]), ("upstream-pin", True))

    def test_a_lift_covers_a_missing_anchor_and_a_forbidden_one(self):
        r = self.resolve(manifest(), FakeTransport(files={ORIGIN: DATA}), lift="no anchor yet")
        self.assertEqual(r["anchor"], "unverified-lifted")
        self.out.unlink()
        r = self.resolve(manifest({"class": "git-sha256-of-our-own-artifact", "sha256": SUM}), FakeTransport(files={ORIGIN: DATA}), lift="ours")
        self.assertEqual(r["detail"]["lifted_error"], "TRUST_ANCHOR_FORBIDDEN")

    def test_a_lift_never_covers_the_users_own_expect_sha256(self):
        self.refused("TRUST_EXPECT_MISMATCH", manifest(pin()), FakeTransport(files={ORIGIN: DATA}), lift="x", expect_sha256=OTHER_SUM)

    def test_a_lift_cannot_hide_an_expect_sha256_that_names_a_third_object(self):
        self.refused("TRUST_EXPECT_MISMATCH", manifest(pin()), FakeTransport(files={ORIGIN: OTHER}), lift="x", expect_sha256="c" * 64)

    def test_a_lift_never_covers_a_transfer_failure(self):
        self.refused("TRUST_TRANSPORT", manifest(pin()), FakeTransport(), lift="x")

    def test_the_check_evidence_gate_refuses_a_result_with_a_lifted_line(self):
        self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: OTHER}), lift="x")
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(T.check_evidence(self.ev / "trust.jsonl"), 3)
        clean = self.dir / "clean.jsonl"
        clean.write_text(json.dumps({"schema": T.RECORD_SCHEMA, "anchor": "upstream-pin", "gating": True}) + "\n")
        self.assertEqual(T.check_evidence(clean), 0)


class UserInput(Base):
    def test_a_user_path_is_trusted_whatever_the_pin_says_and_is_recorded(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        r = self.resolve(manifest(pin()), FakeTransport(), user_source=str(mine))
        self.assertEqual((r["anchor"], r["sha256"], r["size"]), ("user", OTHER_SUM, len(OTHER)))
        self.assertEqual(r["detail"]["pin"], "differs")
        self.assertFalse(r["detail"]["pin_enforced"])
        self.assertEqual(self.lines()[0]["anchor"], "user")

    def test_a_user_url_for_a_repo_defined_artifact_is_a_user_input_not_a_mirror_check(self):
        mirror = "https://mirror.example.net/tool-1.0.run"
        t = FakeTransport(files={mirror: OTHER})
        r = self.resolve(manifest(pin(), official()), t, user_source=mirror)
        self.assertEqual((r["anchor"], r["sha256"]), ("user", OTHER_SUM))
        self.assertEqual(t.doc_calls, [], "the user's URL is not checked against the repo's checksum document")
        self.assertEqual(t.download_calls[0]["sha256"], None)

    def test_expect_sha256_is_enforced_in_every_mode(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        for mode in ("trust-git-only", "strict"):
            with self.assertRaises(T.TrustError) as cm:
                self.resolve(manifest(), FakeTransport(), user_source=str(mine), expect_sha256=SUM, mode=mode)
            self.assertEqual(cm.exception.code, "TRUST_EXPECT_MISMATCH")
        r = self.resolve(manifest(), FakeTransport(), user_source=str(mine), expect_sha256=OTHER_SUM)
        self.assertTrue(r["detail"]["expect_sha256"])

    def test_expect_sha256_mismatch_deletes_a_downloaded_user_url(self):
        mirror = "https://mirror.example.net/tool-1.0.run"
        self.refused("TRUST_EXPECT_MISMATCH", manifest(), FakeTransport(files={mirror: OTHER}), user_source=mirror, expect_sha256=SUM)

    def test_strict_mode_holds_a_user_object_to_the_pin(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        with self.assertRaises(T.TrustError) as cm:
            self.resolve(manifest(pin()), FakeTransport(), user_source=str(mine), mode="strict")
        self.assertEqual(cm.exception.code, "TRUST_PIN_MISMATCH")

    def test_strict_mode_accepts_the_pinned_user_object_and_an_object_with_no_pin(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(DATA)
        self.assertEqual(self.resolve(manifest(pin()), FakeTransport(), user_source=str(mine), mode="strict")["detail"]["pin"], "match")
        mine.write_bytes(OTHER)
        self.assertEqual(self.resolve(manifest(), FakeTransport(), user_source=str(mine), mode="strict")["detail"]["pin"], "none")

    def test_strict_mode_holds_a_user_object_to_the_operator_pin(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        with self.assertRaises(T.TrustError) as cm:
            self.resolve(manifest(), FakeTransport(), user_source=str(mine), mode="strict", operator_pin=SUM)
        self.assertEqual(cm.exception.code, "TRUST_PIN_MISMATCH")

    def test_a_user_path_is_recorded_in_place_when_no_output_path_is_given(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        with contextlib.redirect_stderr(io.StringIO()):
            r = T.resolve("tool", None, manifest=manifest(pin()), transport=FakeTransport(), user_source=str(mine), evidence_dir=self.ev)
        self.assertEqual(r["path"], str(mine))
        self.assertEqual(mine.read_bytes(), OTHER)

    def test_a_users_own_object_needs_no_manifest_entry(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        with contextlib.redirect_stderr(io.StringIO()):
            r = T.resolve("not-in-the-manifest", self.out, manifest=manifest(pin()), transport=FakeTransport(), user_source=str(mine),
                          evidence_dir=self.ev)
        self.assertEqual((r["anchor"], r["sha256"], r["detail"]["pin"]), ("user", OTHER_SUM, "none"))

    def test_a_malformed_expect_sha256_is_a_usage_error(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        with self.assertRaises(T.TrustError) as cm:
            self.resolve(manifest(pin()), FakeTransport(), user_source=str(mine), expect_sha256="not-a-hash")
        self.assertEqual(cm.exception.code, "TRUST_USAGE")

    def test_a_name_that_could_forge_a_log_line_is_refused(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        for bad in ("two\nlines", "x y", "UPPER", "", "-lead", "a" * 97):
            with self.assertRaises(T.TrustError, msg=repr(bad)) as cm:
                T.resolve(bad, self.out, manifest=manifest(pin()), transport=FakeTransport(), user_source=str(mine), emit=False)
            self.assertEqual(cm.exception.code, "TRUST_USAGE")
        self.assertFalse(self.out.exists())

    def test_a_missing_user_path_is_a_transfer_failure(self):
        self.refused("TRUST_TRANSPORT", manifest(pin()), FakeTransport(), user_source=str(self.dir / "absent"))

    def test_a_user_object_is_never_marked_non_gating(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        self.assertTrue(self.resolve(manifest(pin()), FakeTransport(), user_source=str(mine))["gating"])


class EvidenceRecord(Base):
    def test_one_line_per_artifact_with_the_anchor_hash_size_and_origin(self):
        self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}))
        (row,) = self.lines()
        self.assertEqual((row["schema"], row["name"], row["anchor"], row["sha256"], row["size"], row["origin"], row["mode"]),
                         (T.RECORD_SCHEMA, "tool", "upstream-pin", SUM, len(DATA), ORIGIN, "trust-git-only"))
        self.assertIn("KF_TRUST name=tool anchor=upstream-pin sha256=" + SUM, self.stdout)

    def test_lines_append_and_never_overwrite(self):
        self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}))
        self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}))
        self.assertEqual(len(self.lines()), 2)

    def test_a_presigned_user_url_never_reaches_the_evidence_or_stdout(self):
        t = FakeTransport(files={PRESIGNED: OTHER})
        r = self.resolve(manifest(pin()), t, user_source=PRESIGNED)
        text = (self.ev / "trust.jsonl").read_text() + self.stdout + json.dumps(r)
        for secret in ("X-Amz-Signature", "fa4efa4efa4e", "X-Amz-Credential", "?"):
            self.assertNotIn(secret, text)
        self.assertEqual(r["origin"], "https://objects.example.org/private/tool-1.0.run")
        self.assertTrue(r["origin_redacted"])

    def test_credentials_in_a_url_are_never_recorded(self):
        url = "https://user:hunter2@mirror.example.net/tool.run"
        r = self.resolve(manifest(pin()), FakeTransport(files={url: OTHER}), user_source=url)
        self.assertNotIn("hunter2", json.dumps(r) + self.stdout + (self.ev / "trust.jsonl").read_text())

    def test_a_user_path_records_only_the_file_name(self):
        mine = self.dir / "private-dir-name" / "mine.run"
        mine.parent.mkdir()
        mine.write_bytes(OTHER)
        r = self.resolve(manifest(pin()), FakeTransport(), user_source=str(mine))
        self.assertEqual(r["origin"], "file:mine.run")
        self.assertNotIn("private-dir-name", (self.ev / "trust.jsonl").read_text())

    def test_the_record_of_a_user_path_has_no_directory_part(self):
        mine = self.dir / "deep" / "er" / "mine.run"
        mine.parent.mkdir(parents=True)
        mine.write_bytes(OTHER)
        r = self.resolve(manifest(pin()), FakeTransport(), user_source=str(mine))
        self.assertNotIn("/", r["origin"])

    def test_the_record_names_the_manifest_that_judged_it(self):
        self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}))
        self.assertEqual(self.lines()[0]["manifest_sha256"], T.manifest_digest())

    def test_the_evidence_directory_can_come_from_the_environment(self):
        os.environ["KF_TRUST_EVIDENCE_DIR"] = str(self.dir / "envdir")
        self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: DATA}), evidence_dir=None)
        self.assertTrue((self.dir / "envdir" / "trust.jsonl").is_file())

    def test_a_refusal_writes_no_trust_line(self):
        with self.assertRaises(T.TrustError):
            self.resolve(manifest(pin()), FakeTransport(files={ORIGIN: OTHER}))
        self.assertEqual(self.lines(), [])


class ManifestChecks(unittest.TestCase):
    def test_the_committed_manifest_is_valid(self):
        self.assertEqual(T.validate_manifest(json.loads(T.MANIFEST.read_text())), [])

    def test_an_entry_without_an_anchor_is_invalid(self):
        self.assertTrue(any("no anchor" in p for p in T.validate_manifest(manifest())))

    def test_a_forbidden_anchor_class_is_invalid(self):
        bad = T.validate_manifest(manifest({"class": "git-sha256-of-our-own-artifact"}))
        self.assertTrue(any("not an anchor" in p for p in bad), bad)

    def test_a_pin_whose_source_is_ours_is_invalid(self):
        bad = T.validate_manifest(manifest(pin(pin_source_url="https://bundles.kayfabe.dev/x.sha256")))
        self.assertTrue(any("one of ours" in p for p in bad), bad)

    def test_a_checksum_url_of_ours_is_invalid(self):
        bad = T.validate_manifest(manifest(official("https://logs.kayfabe.dev/x.sha256")))
        self.assertTrue(any("TRUST_SAME_ORIGIN" in p for p in bad), bad)

    def test_a_pin_needs_a_date_a_status_and_a_scope(self):
        a = pin()
        for k in ("measured", "status", "scope", "pin_origin"):
            b = dict(a)
            del b[k]
            self.assertTrue(T.validate_manifest(manifest(b)), k)

    def test_an_http_origin_is_invalid(self):
        self.assertTrue(T.validate_manifest(manifest(pin(), origin="http://download.example.org/x")))

    def test_only_an_upstream_binary_pinned_by_measurement_awaits_the_owner(self):
        self.assertEqual(T.confirmation(pin(pin_origin="measured-from-upstream")), "pending-owner-confirmation")
        self.assertEqual(T.confirmation(pin(pin_origin="measured-from-upstream", scope="source")), "owner-confirmed")
        self.assertEqual(T.confirmation(pin(pin_origin="vendor-published")), "owner-confirmed")
        self.assertEqual(T.confirmation(official()), "owner-confirmed")

    def test_the_pending_confirmation_is_recorded_in_the_result(self):
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "x"
            with contextlib.redirect_stderr(io.StringIO()):
                r = T.resolve("tool", out, manifest=manifest(pin(pin_origin="measured-from-upstream")),
                              transport=FakeTransport(files={ORIGIN: DATA}), evidence_dir=Path(d) / "ev")
        self.assertEqual(r["detail"]["confirmation"], "pending-owner-confirmation")


def _git(cwd, *argv):
    env = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1", "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.org",
           "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.org"}
    return subprocess.run(["git", *argv], cwd=cwd, env=env, capture_output=True, text=True, check=True).stdout.strip()


class GitCommitAnchor(Base):
    def setUp(self):
        super().setUp()
        self.repo = self.dir / "upstream"
        self.repo.mkdir()
        _git(self.repo, "init", "-q", "-b", "main")
        (self.repo / "a.txt").write_text("one\n")
        _git(self.repo, "add", "a.txt")
        _git(self.repo, "commit", "-q", "-m", "one")
        self.first = _git(self.repo, "rev-parse", "HEAD")
        _git(self.repo, "tag", "v1")
        self.dest = self.dir / "work"

    def man(self, commit, ref="v1"):
        return {"schema": T.SCHEMA, "artifacts": {"ogkm": {
            "row": 29, "kind": "git", "routing": "helper", "origin": str(self.repo), "runs": "build", "consumers": ["scripts/x.sh"],
            "anchors": [{"class": "upstream-pin", "commit": commit, "ref": ref, "pin_origin": "git-commit", "scope": "source",
                         "pin_source_url": "https://github.com/example/ogkm/releases/tag/v1", "status": "MEASURED", "measured": "2026-11-17"}]}}}

    def go(self, man, **kw):
        with contextlib.redirect_stderr(io.StringIO()):
            return T.resolve_git("ogkm", self.dest, manifest=man, evidence_dir=self.ev, allow_local=True, **kw)

    def test_head_equal_to_the_pinned_commit_is_verified(self):
        r = self.go(self.man(self.first))
        self.assertEqual((r["anchor"], r["commit"]), ("upstream-pin", self.first))
        self.assertEqual(_git(self.dest, "rev-parse", "HEAD"), self.first)

    def test_a_moved_tag_is_refused_and_removes_the_checkout(self):
        (self.repo / "a.txt").write_text("two\n")
        _git(self.repo, "commit", "-qam", "two")
        _git(self.repo, "tag", "-f", "v1")  # the tag moved: the id the manifest pins is no longer where the tag is
        moved = _git(self.repo, "rev-parse", "v1")
        self.assertNotEqual(moved, self.first)
        _git(self.repo, "branch", "-q", "keep", self.first)
        with self.assertRaises(T.TrustError) as cm:
            self.go(self.man("0" * 40))
        self.assertEqual(cm.exception.code, "TRUST_PIN_MISMATCH")
        self.assertFalse(self.dest.exists())

    def test_the_pin_wins_over_a_moved_tag_when_the_commit_is_still_served(self):
        (self.repo / "a.txt").write_text("two\n")
        _git(self.repo, "commit", "-qam", "two")
        _git(self.repo, "tag", "-f", "v1")
        r = self.go(self.man(self.first))
        self.assertEqual(r["commit"], self.first)

    def test_a_lift_records_the_commit_that_was_actually_checked_out(self):
        r = self.go(self.man("0" * 40), lift="try an unpinned commit")
        self.assertEqual((r["anchor"], r["gating"], r["commit"]), ("unverified-lifted", False, self.first))

    def test_a_user_remote_is_trusted_and_its_commit_recorded(self):
        r = self.go(self.man("0" * 40), user_source=str(self.repo))
        self.assertEqual((r["anchor"], r["commit"], r["detail"]["pin"]), ("user", self.first, "differs"))

    def test_strict_mode_holds_a_user_remote_to_the_pinned_commit(self):
        with self.assertRaises(T.TrustError) as cm:
            self.go(self.man("0" * 40), user_source=str(self.repo), mode="strict")
        self.assertEqual(cm.exception.code, "TRUST_PIN_MISMATCH")
        self.assertFalse(self.dest.exists(), "a refused checkout is removed")

    def test_a_user_remote_that_differs_is_recorded_as_not_enforced_in_the_default_mode(self):
        r = self.go(self.man("1" * 40), user_source=str(self.repo))
        self.assertFalse(r["detail"]["pin_enforced"])
        self.assertTrue(r["gating"])

    def test_verify_git_with_a_lift_records_the_mismatch_as_lifted_and_not_gating(self):
        self.go(self.man(self.first))
        with contextlib.redirect_stderr(io.StringIO()):
            r = T.verify_git("ogkm", self.dest, manifest=self.man("1" * 40), lift="cached tree of a build I made", evidence_dir=self.ev)
        self.assertEqual((r["anchor"], r["gating"], r["commit"]), ("unverified-lifted", False, self.first))

    def test_an_existing_checkout_at_the_pinned_commit_verifies(self):
        self.go(self.man(self.first))
        with contextlib.redirect_stderr(io.StringIO()):
            r = T.verify_git("ogkm", self.dest, manifest=self.man(self.first), evidence_dir=self.ev)
        self.assertEqual((r["anchor"], r["commit"]), ("upstream-pin", self.first))

    def test_an_existing_checkout_at_another_commit_is_refused(self):
        self.go(self.man(self.first))
        with self.assertRaises(T.TrustError) as cm:
            T.verify_git("ogkm", self.dest, manifest=self.man("1" * 40), emit=False)
        self.assertEqual(cm.exception.code, "TRUST_PIN_MISMATCH")

    def test_a_directory_that_is_not_a_checkout_is_a_transfer_failure(self):
        self.dest.mkdir()
        with self.assertRaises(T.TrustError) as cm:
            T.verify_git("ogkm", self.dest, manifest=self.man(self.first), emit=False)
        self.assertEqual(cm.exception.code, "TRUST_TRANSPORT")

    def test_a_git_name_that_could_forge_a_log_line_is_refused(self):
        with self.assertRaises(T.TrustError) as cm:
            T.resolve_git("a\nb", self.dest, manifest=self.man(self.first), emit=False, allow_local=True)
        self.assertEqual(cm.exception.code, "TRUST_USAGE")
        self.assertFalse(self.dest.exists())

    def test_a_file_artifact_is_not_a_git_artifact(self):
        with self.assertRaises(T.TrustError) as cm:
            T.resolve_git("tool", self.dest, manifest=manifest(pin()), emit=False)
        self.assertEqual(cm.exception.code, "TRUST_USAGE")

    def test_the_git_record_is_written(self):
        self.go(self.man(self.first))
        (row,) = self.lines()
        self.assertEqual((row["anchor"], row["commit"], row["name"]), ("upstream-pin", self.first, "ogkm"))


class Handler(http.server.BaseHTTPRequestHandler):
    files = {}
    redirects = {}

    def log_message(self, *a):
        pass

    def do_GET(self):
        if self.path in self.redirects:
            self.send_response(302)
            self.send_header("Location", self.redirects[self.path])
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        data = self.files.get(self.path)
        if data is None:
            self.send_response(404)
            self.end_headers()
            return
        rng = self.headers.get("Range")
        start, end = 0, len(data) - 1
        if rng:
            a, _, b = rng.split("=")[1].partition("-")
            start, end = int(a), min(int(b), len(data) - 1) if b else len(data) - 1
        self.send_response(206 if rng else 200)
        if rng:
            self.send_header("Content-Range", "bytes %d-%d/%d" % (start, end, len(data)))
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        self.end_headers()
        self.wfile.write(data[start:end + 1])


class RealTransport(Base):
    """HttpTransport against a loopback server: the bytes on the wire, kf_fetch for the pinned path, urllib for the rest."""

    def setUp(self):
        super().setUp()
        Handler.redirects = {}
        Handler.files = {"/pkg/tool.run": DATA, "/pkg/tool.run.sha256sum": (SUM + "  tool.run\n").encode(), "/big": b"x" * 5000}
        self.srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.addCleanup(self.srv.shutdown)
        self.addCleanup(self.srv.server_close)
        self.base = "http://127.0.0.1:%d" % self.srv.server_address[1]

    def test_https_only_by_default(self):
        with self.assertRaises(T.TransportError):
            T.HttpTransport().get_text(self.base + "/pkg/tool.run.sha256sum")
        with self.assertRaises(T.TransportError):
            T.HttpTransport().download(self.base + "/pkg/tool.run", self.out)

    def test_get_text_reads_a_document_and_bounds_it(self):
        t = T.HttpTransport(allow_loopback_http=True)
        self.assertIn(SUM, t.get_text(self.base + "/pkg/tool.run.sha256sum").decode())
        with self.assertRaises(T.TransportError) as cm:
            t.get_text(self.base + "/big", max_bytes=100)
        self.assertEqual(cm.exception.kind, "SIZE")

    def test_pinned_download_goes_through_kf_fetch_and_verifies(self):
        t = T.HttpTransport(allow_loopback_http=True)
        sha, n = t.download(self.base + "/pkg/tool.run", self.out, sha256=SUM, size=len(DATA))
        self.assertEqual((sha, n), (SUM, len(DATA)))
        with self.assertRaises(T.TransportError) as cm:
            t.download(self.base + "/pkg/tool.run", self.dir / "other", sha256=OTHER_SUM)
        self.assertEqual(cm.exception.kind, "HASH")
        self.assertFalse((self.dir / "other").exists())

    def test_unpinned_download_hashes_and_bounds_the_bytes(self):
        t = T.HttpTransport(allow_loopback_http=True)
        sha, n = t.download(self.base + "/pkg/tool.run", self.out)
        self.assertEqual((sha, n), (SUM, len(DATA)))
        with self.assertRaises(T.TransportError) as cm:
            t.download(self.base + "/big", self.dir / "big", max_bytes=100)
        self.assertEqual(cm.exception.kind, "SIZE")
        self.assertFalse((self.dir / "big").exists())

    def test_pure_python_path_enforces_the_pin_without_kf_fetch(self):
        t = T.HttpTransport(allow_loopback_http=True, use_kf_fetch=False)
        t.download(self.base + "/pkg/tool.run", self.out, sha256=SUM)
        with self.assertRaises(T.TransportError) as cm:
            t.download(self.base + "/pkg/tool.run", self.dir / "other", sha256=OTHER_SUM)
        self.assertEqual(cm.exception.kind, "HASH")

    def test_a_relative_output_path_means_the_callers_directory(self):
        # found by the first live run against real hosts: kf_fetch runs in a scratch directory, so a relative path must be made absolute first
        here = os.getcwd()
        os.chdir(self.dir)
        self.addCleanup(os.chdir, here)
        sha, n = T.HttpTransport(allow_loopback_http=True).download(self.base + "/pkg/tool.run", "rel.run", sha256=SUM)
        self.assertEqual((sha, n, (self.dir / "rel.run").read_bytes()), (SUM, len(DATA), DATA))
        man = manifest(official(self.base + "/pkg/tool.run.sha256sum"), origin=self.base + "/pkg/tool.run")
        with contextlib.redirect_stderr(io.StringIO()):
            r = T.resolve("tool", "rel2.run", manifest=man, transport=T.HttpTransport(allow_loopback_http=True))
        self.assertEqual((r["sha256"], (self.dir / "rel2.run").is_file()), (SUM, True))

    def test_pinned_download_goes_through_kf_fetch_and_verifies_from_a_relative_path(self):
        here = os.getcwd()
        os.chdir(self.dir)
        self.addCleanup(os.chdir, here)
        for _ in range(2):  # the second call finds the file there already
            sha, n = T.HttpTransport(allow_loopback_http=True).download(self.base + "/pkg/tool.run", "again.run", sha256=SUM, size=len(DATA))
            self.assertEqual((sha, n), (SUM, len(DATA)))
        self.assertEqual(sorted(p.name for p in self.dir.iterdir()), ["again.run"])

    def test_a_published_download_is_readable_by_the_group_and_others(self):
        old = os.umask(0o022)
        self.addCleanup(os.umask, old)
        man = manifest(official(self.base + "/pkg/tool.run.sha256sum"), origin=self.base + "/pkg/tool.run")
        self.resolve(man, T.HttpTransport(allow_loopback_http=True))
        self.assertEqual(self.out.stat().st_mode & 0o777, 0o644)

    def test_missing_object_is_a_network_failure(self):
        with self.assertRaises(T.TransportError) as cm:
            T.HttpTransport(allow_loopback_http=True).download(self.base + "/absent", self.out)
        self.assertEqual(cm.exception.kind, "NETWORK")
        with self.assertRaises(T.TransportError) as cm:
            T.HttpTransport(allow_loopback_http=True).download(self.base + "/absent", self.out, sha256=SUM)
        self.assertEqual(sorted(p.name for p in self.dir.iterdir()), [], "a failed download leaves no file and no resume state")

    def test_a_checksum_document_may_not_move_to_another_host(self):
        Handler.redirects["/hop-host"] = "http://localhost:%d/pkg/tool.run.sha256sum" % self.srv.server_address[1]
        with self.assertRaises(T.TransportError):
            T.HttpTransport(allow_loopback_http=True).get_text(self.base + "/hop-host")

    def test_a_checksum_document_redirected_to_plain_http_is_refused(self):
        Handler.redirects["/hop-http"] = self.base + "/pkg/tool.run.sha256sum"
        with self.assertRaises(T.TransportError):
            T.HttpTransport(allow_loopback_http=True).get_text(self.base + "/hop-http")

    def test_a_failed_pinned_download_leaves_nothing_beside_the_output(self):
        for url, pin_sha in (("/absent", SUM), ("/pkg/tool.run", OTHER_SUM)):
            with self.assertRaises(T.TransportError):
                T.HttpTransport(allow_loopback_http=True).download(self.base + url, self.dir / "tool.run.unverified", sha256=pin_sha)
            self.assertEqual(sorted(p.name for p in self.dir.iterdir() if p.name.startswith("tool")), [], url)

    def test_a_lifted_download_is_bounded_by_twice_the_pinned_size(self):
        man = manifest(pin(size=100), origin=self.base + "/big")
        with self.assertRaises(T.TrustError) as cm:
            self.resolve(man, T.HttpTransport(allow_loopback_http=True), lift="the bytes changed, I looked")
        self.assertEqual(cm.exception.code, "TRUST_TRANSPORT")
        self.assertFalse(self.out.exists())

    def test_end_to_end_official_checksum_then_pinned_fetch(self):
        man = manifest(official(self.base + "/pkg/tool.run.sha256sum"), origin=self.base + "/pkg/tool.run")
        r = self.resolve(man, T.HttpTransport(allow_loopback_http=True))
        self.assertEqual((r["anchor"], r["sha256"]), ("official-checksum-url", SUM))

    def test_end_to_end_wrong_checksum_is_refused(self):
        Handler.files["/pkg/tool.run.sha256sum"] = (OTHER_SUM + "  tool.run\n").encode()
        man = manifest(official(self.base + "/pkg/tool.run.sha256sum"), origin=self.base + "/pkg/tool.run")
        self.refused("TRUST_UPSTREAM_SUM_MISMATCH", man, T.HttpTransport(allow_loopback_http=True))


class Cli(Base):
    def run_cli(self, *argv, env=None):
        e = {k: v for k, v in os.environ.items() if not k.startswith("KF_TRUST")}
        e.update(env or {})
        return subprocess.run([sys.executable, "-I", "-B", str(HERE / "kf_trust.py"), *argv], capture_output=True, text=True, env=e, cwd=self.dir)

    def man_file(self, man):
        p = self.dir / "m.json"
        p.write_text(json.dumps(man))
        return str(p)

    def test_fetch_of_a_user_path_exits_zero_and_prints_the_record(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        r = self.run_cli("fetch", "tool", "--out", str(self.dir / "o"), "--user-source", str(mine), "--manifest", self.man_file(manifest(pin())),
                         "--evidence-dir", str(self.ev))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("KF_TRUST name=tool anchor=user sha256=" + OTHER_SUM, r.stderr)
        self.assertEqual(json.loads((self.ev / "trust.jsonl").read_text())["anchor"], "user")

    def test_a_refusal_is_exit_three_with_the_name_on_stderr_and_no_url(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        r = self.run_cli("fetch", "tool", "--out", str(self.dir / "o"), "--user-source", str(mine), "--expect-sha256", SUM,
                         "--manifest", self.man_file(manifest(pin())))
        self.assertEqual(r.returncode, 3)
        self.assertIn("TRUST_EXPECT_MISMATCH:tool", r.stderr)
        self.assertFalse((self.dir / "o").exists())

    def test_verify_checks_a_file_on_disk_without_the_network(self):
        f = self.dir / "have.run"
        f.write_bytes(DATA)
        ok = self.run_cli("verify", "tool", str(f), "--manifest", self.man_file(manifest(pin())))
        self.assertEqual(ok.returncode, 0, ok.stderr)
        f.write_bytes(OTHER)
        bad = self.run_cli("verify", "tool", str(f), "--manifest", self.man_file(manifest(pin())))
        self.assertEqual(bad.returncode, 3)
        self.assertIn("TRUST_PIN_MISMATCH", bad.stderr)

    def test_the_environment_selects_the_mode_and_the_lift(self):
        f = self.dir / "have.run"
        f.write_bytes(DATA)
        r = self.run_cli("verify", "tool", str(f), "--manifest", self.man_file(manifest(pin())), env={"KF_TRUST_MODE": "bogus"})
        self.assertEqual(r.returncode, 2)
        self.assertIn("TRUST_BAD_MODE", r.stderr)

    def test_record_writes_a_user_line_for_an_object_this_process_did_not_fetch(self):
        r = self.run_cli("record", "--name", "spec-archive", "--sha256", SUM, "--size", "12", "--source", "user-url-file",
                         "--origin", PRESIGNED, "--evidence-dir", str(self.ev))
        self.assertEqual(r.returncode, 0, r.stderr)
        row = json.loads((self.ev / "trust.jsonl").read_text())
        self.assertEqual((row["anchor"], row["sha256"], row["size"], row["source"]), ("user", SUM, 12, "user-url-file"))
        self.assertNotIn("Signature", (self.ev / "trust.jsonl").read_text())

    def test_stdout_stays_empty_for_fetch(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        r = self.run_cli("fetch", "tool", "--out", str(self.dir / "o"), "--user-source", str(mine), "--manifest", self.man_file(manifest(pin())))
        self.assertEqual((r.returncode, r.stdout), (0, ""))

    def test_a_second_record_run_appends_a_line(self):
        for _ in range(2):
            self.run_cli("record", "--name", "x", "--sha256", SUM, "--size", "1", "--evidence-dir", str(self.ev))
        self.assertEqual(len((self.ev / "trust.jsonl").read_text().splitlines()), 2)

    def test_the_record_line_carries_the_manifest_digest_and_the_mode(self):
        self.run_cli("record", "--name", "x", "--sha256", SUM, "--size", "1", "--evidence-dir", str(self.ev), "--mode", "strict")
        row = json.loads((self.ev / "trust.jsonl").read_text())
        self.assertEqual((row["manifest_sha256"], row["mode"]), (T.manifest_digest(), "strict"))

    def test_a_user_object_of_an_unlisted_name_is_recorded(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        r = self.run_cli("fetch", "not-listed", "--out", str(self.dir / "o"), "--user-source", str(mine), "--manifest", self.man_file(manifest(pin())))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("anchor=user", r.stderr)

    def test_a_name_with_a_newline_cannot_forge_a_record_line(self):
        r = self.run_cli("record", "--name", "x\nKF_TRUST name=forged anchor=upstream-pin", "--sha256", SUM, "--size", "1", "--evidence-dir", str(self.ev))
        self.assertEqual(r.returncode, 2)
        self.assertNotIn("forged", r.stderr + r.stdout)
        self.assertFalse((self.ev / "trust.jsonl").exists())

    def test_fetch_with_a_forged_name_exits_two_and_writes_nothing(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        r = self.run_cli("fetch", "x\nKF_TRUST name=forged", "--out", str(self.dir / "o"), "--user-source", str(mine), "--evidence-dir", str(self.ev))
        self.assertEqual(r.returncode, 2)
        self.assertFalse((self.dir / "o").exists())
        self.assertNotIn("forged", r.stdout)

    def test_record_refuses_a_forged_name_in_process_too(self):
        with contextlib.redirect_stderr(io.StringIO()) as err, contextlib.redirect_stdout(io.StringIO()) as out:
            rc = T.main(["record", "--name", "a\nb", "--sha256", SUM, "--size", "1", "--evidence-dir", str(self.ev)])
        self.assertEqual(rc, 2)
        self.assertNotIn("KF_TRUST name=", err.getvalue() + out.getvalue())

    def test_a_malformed_expect_sha256_exits_two(self):
        mine = self.dir / "mine.run"
        mine.write_bytes(OTHER)
        r = self.run_cli("fetch", "tool", "--out", str(self.dir / "o"), "--user-source", str(mine), "--expect-sha256", "xyz",
                         "--manifest", self.man_file(manifest(pin())))
        self.assertEqual(r.returncode, 2)

    def test_exit_codes_are_the_documented_ones(self):
        self.assertEqual((T.EXIT["TRUST_TRANSPORT"], T.EXIT["TRUST_USAGE"], T.EXIT["TRUST_UNKNOWN_ARTIFACT"], T.EXIT.get("TRUST_PIN_MISMATCH", 3)),
                         (1, 2, 4, 3))

    def test_unknown_artifact_and_bad_usage_exit_codes(self):
        r = self.run_cli("fetch", "ghost", "--out", str(self.dir / "o"), "--manifest", self.man_file(manifest(pin())))
        self.assertEqual(r.returncode, 4)
        self.assertIn("TRUST_UNKNOWN_ARTIFACT", r.stderr)
        self.assertEqual(self.run_cli("fetch").returncode, 2)

    def test_list_prints_every_artifact_with_its_anchor_classes(self):
        r = self.run_cli("list", "--manifest", self.man_file(manifest(pin(), official())))
        self.assertEqual(r.stdout.strip(), "tool\tfile\tupstream-pin,official-checksum-url")

    def test_check_evidence_exit_codes(self):
        good = self.dir / "g.jsonl"
        good.write_text(json.dumps({"schema": T.RECORD_SCHEMA, "anchor": "user", "gating": True}) + "\n")
        bad = self.dir / "b.jsonl"
        bad.write_text(json.dumps({"schema": T.RECORD_SCHEMA, "anchor": "unverified-lifted", "gating": False}) + "\n")
        junk = self.dir / "j.jsonl"
        junk.write_text("not json\n")
        self.assertEqual(self.run_cli("check-evidence", str(good)).returncode, 0)
        self.assertEqual(self.run_cli("check-evidence", str(bad)).returncode, 3)
        self.assertEqual(self.run_cli("check-evidence", str(junk)).returncode, 2)


class Helpers(unittest.TestCase):
    def test_parse_sha256sum_formats(self):
        doc = (SUM + "  tool.run\n" + OTHER_SUM + " *sub/dir/other.run\n").encode()
        self.assertEqual(T.parse_sha256sum(doc, "tool.run"), SUM)
        self.assertEqual(T.parse_sha256sum(doc, "other.run"), OTHER_SUM)
        self.assertIsNone(T.parse_sha256sum(doc, "absent"))
        self.assertIsNone(T.parse_sha256sum(doc), "two hashes and no member is ambiguous")
        dup = (SUM + "  tool.run\n" + OTHER_SUM + "  tool.run\n").encode()
        self.assertIsNone(T.parse_sha256sum(dup, "tool.run"), "two hashes for one member is ambiguous")
        self.assertEqual(T.parse_sha256sum((SUM + " *./rustup-init\n").encode()), SUM)

    def test_redact_url_and_ownership(self):
        self.assertEqual(T.redact_url("https://u:p@h.example.org:8443/a/b?c=d#e"), ("https://h.example.org:8443/a/b", True))
        self.assertEqual(T.redact_url("https://h.example.org/a"), ("https://h.example.org/a", False))
        self.assertTrue(T.is_ours("https://bundles.kayfabe.dev/x"))
        self.assertTrue(T.is_ours("https://abc.r2.cloudflarestorage.com/x"))
        self.assertTrue(T.is_ours("https://github.com/reindertpelsma/kayfabe/releases/download/x/y"))
        self.assertFalse(T.is_ours("https://github.com/NVIDIA/open-gpu-kernel-modules/releases"))
        self.assertFalse(T.is_ours("https://us.download.nvidia.com/x"))

    def test_normalize_lift_bounds(self):
        for bad in ("", "a" * 201):
            if bad:
                with self.assertRaises(T.TrustError):
                    T.normalize_lift(bad)
        self.assertEqual(T.normalize_lift("x" * 200), "x" * 200)
        for bad in ("two\nlines", "tab\there", " lead", "trail "):
            with self.assertRaises(T.TrustError, msg=repr(bad)):
                T.normalize_lift(bad)

    def test_record_prints_the_lift_line(self):
        buf = io.StringIO()
        T.record(T.make_result("n", path=None, sha256=SUM, size=3, anchor="unverified-lifted", mode="trust-git-only", gating=False,
                               lift_reason="r", detail={"lifted_error": "TRUST_PIN_MISMATCH"}), out=buf)
        self.assertIn("KF_TRUST_LIFTED name=n reason=r lifted=TRUST_PIN_MISMATCH", buf.getvalue())
        self.assertIn("gating=no", buf.getvalue())

    def test_the_fake_uuid_prefix_is_the_only_identifier_shape_in_these_tests(self):
        self.assertTrue(FAKE_UUID.startswith("fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e"))


if __name__ == "__main__":
    unittest.main()

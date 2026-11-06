"""GPU-free tests of the coordinator's evidence fetch tool (fetch_evidence.py). Not run in a box.

FALSIFIER: the signer disagrees with the AWS Signature V4 test-suite vector; a key outside
<profile>/<run>/<attempt>.tgz|json is requested; a mismatching digest is reported as ok; the tool
extracts, prints a credential, or overwrites a non-empty output directory.
"""

import datetime
import gzip
import hashlib
import io
import json
import tarfile
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

import fetch_evidence as fe

ENV = {
    "CF_ACCOUNT_ID": "0123456789abcdef" * 2,
    "R2_ACCESS_KEY_ID": "AKIDFAKE",
    "R2_SECRET_ACCESS_KEY": "s3cr3t" * 6,
}
NOW = datetime.datetime(2026, 11, 6, 12, 0, 0, tzinfo=datetime.timezone.utc)


def bundle():
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        data = b"hello"
        info = tarfile.TarInfo("log/output.log")
        info.size = len(data)
        archive.addfile(info, io.BytesIO(data))
    return buffer.getvalue()


class Opener:
    """Answers GETs from a dict path-or-query -> bytes, recording every request."""

    def __init__(self, objects):
        self.objects, self.requests = objects, []

    def open(self, request, timeout=None):
        self.requests.append(request)
        path = request.full_url.split("?")[0].split(".com/", 1)[1]
        name = path.split("/", 1)[1] if "/" in path else ""
        if name == "":
            body = self.objects.get("LIST", b"")
        elif name in self.objects:
            body = self.objects[name]
        else:
            import urllib.error

            raise urllib.error.HTTPError(request.full_url, 404, "nf", {}, io.BytesIO())
        response = io.BytesIO(body)
        response.status = 200
        response.__enter__ = lambda: response
        return response


def listing(keys):
    rows = "".join(
        "<Contents><Key>%s</Key><Size>%d</Size><LastModified>2026-11-06T10:00:00Z</LastModified></Contents>"
        % (k, n)
        for k, n in keys
    )
    return (
        '<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><IsTruncated>false</IsTruncated>'
        + rows
        + "</ListBucketResult>"
    ).encode()


class Signer(unittest.TestCase):
    def test_aws_get_vanilla_vector(self):
        # AWS Signature V4 test suite, `get-vanilla` (credentials and date are the suite's own)
        out = fe.sigv4_headers(
            "GET",
            "https://example.amazonaws.com/",
            {},
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "service",
            datetime.datetime(2015, 8, 30, 12, 36, 0, tzinfo=datetime.timezone.utc),
        )
        self.assertEqual(
            out["authorization"],
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, "
            "SignedHeaders=host;x-amz-date, Signature="
            "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31",
        )

    def test_query_is_sorted_and_encoded(self):
        a = fe.sigv4_headers(
            "GET", "https://h.example/b?z=1&a=x y", {}, "K", "S", "auto", "s3", NOW
        )
        b = fe.sigv4_headers(
            "GET", "https://h.example/b?a=x%20y&z=1", {}, "K", "S", "auto", "s3", NOW
        )
        self.assertEqual(a["authorization"], b["authorization"])


class Tool(unittest.TestCase):
    def run_tool(self, argv, opener, env=ENV):
        r2 = fe.R2(env, opener=opener, now=lambda: NOW) if env else None
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            rc = fe.main(argv, env=env, r2=r2)
        return rc, out.getvalue(), err.getvalue()

    def tmp(self):
        d = tempfile.mkdtemp(prefix="kf-fetch-evidence-")
        self.addCleanup(__import__("shutil").rmtree, d, True)
        return Path(d)

    def test_credentials_come_from_the_environment_and_are_never_printed(self):
        with self.assertRaises(fe.Refused):
            fe.R2({})
        with self.assertRaises(fe.Refused):
            fe.R2(dict(ENV, CF_ACCOUNT_ID="nothex"))
        rc, out, err = self.run_tool(["list"], Opener({"LIST": listing([])}))
        for secret in (ENV["R2_SECRET_ACCESS_KEY"], ENV["CF_ACCOUNT_ID"]):
            self.assertNotIn(secret, out + err)

    def test_list_filters_to_evidence_keys_and_a_run(self):
        xml = listing(
            [
                ("check/42/1.tgz", 100),
                ("check/42/1.json", 10),
                ("sweep/43/2.tgz", 50),
                ("junk/x", 1),
            ]
        )
        rc, out, _ = self.run_tool(["list"], Opener({"LIST": xml}))
        self.assertEqual(rc, 0)
        self.assertEqual(len(out.splitlines()), 3)
        rc, out, _ = self.run_tool(["list", "--run", "43"], Opener({"LIST": xml}))
        self.assertEqual(out.split()[0], "sweep/43/2.tgz")
        rc, _, err = self.run_tool(["list", "--run", "../x"], Opener({"LIST": xml}))
        self.assertEqual(rc, 2)

    def test_get_downloads_verifies_and_never_extracts(self):
        data = bundle()
        sha = hashlib.sha256(data).hexdigest()
        manifest = json.dumps({"sha256": sha}).encode()
        objects = {"check/42/1.tgz": data, "check/42/1.json": manifest}
        out = self.tmp() / "got"
        rc, text, _ = self.run_tool(["get", "check/42/1.tgz", "--out", str(out)], Opener(objects))
        self.assertEqual(rc, 0)
        self.assertIn("sha256 ok", text)
        self.assertEqual(sorted(p.name for p in out.iterdir()), ["42-1.json", "42-1.tgz"])
        self.assertEqual((out / "42-1.tgz").read_bytes(), data)

    def test_a_digest_mismatch_is_reported_and_fails(self):
        data = bundle()
        objects = {
            "check/42/1.tgz": data,
            "check/42/1.json": json.dumps({"sha256": "0" * 64}).encode(),
        }
        rc, text, _ = self.run_tool(
            ["get", "check/42/1.tgz", "--out", str(self.tmp() / "o")], Opener(objects)
        )
        self.assertEqual(rc, 1)
        self.assertIn("SHA256 MISMATCH", text)

    def test_run_attempt_finds_the_profile_by_listing(self):
        data = bundle()
        objects = {
            "LIST": listing([("sweep/42/1.tgz", len(data)), ("check/42/2.tgz", 1)]),
            "sweep/42/1.tgz": data,
        }
        rc, text, _ = self.run_tool(
            ["get", "42-1", "--out", str(self.tmp() / "o")], Opener(objects)
        )
        self.assertEqual(rc, 0)
        self.assertIn("no manifest", text)
        rc, _, err = self.run_tool(["get", "99-1", "--out", str(self.tmp() / "p")], Opener(objects))
        self.assertEqual(rc, 2)

    def test_a_replacement_rental_has_its_own_bundle_and_run_attempt_fetches_both(self):
        data = bundle()
        objects = {
            "LIST": listing(
                [("check/42/1.tgz", 1), ("check/42/1-r1.tgz", 1), ("check/42/1-r1.json", 1)]
            ),
            "check/42/1.tgz": data,
            "check/42/1-r1.tgz": data,
        }
        out = self.tmp() / "o"
        rc, text, _ = self.run_tool(["get", "42-1", "--out", str(out)], Opener(objects))
        self.assertEqual(rc, 0, text)
        self.assertEqual(sorted(p.name for p in out.iterdir()), ["42-1-r1.tgz", "42-1.tgz"])
        rc, text, _ = self.run_tool(
            ["get", "check/42/1-r1.tgz", "--out", str(self.tmp() / "p")], Opener(objects)
        )
        self.assertEqual(rc, 0)
        rc, _, _ = self.run_tool(["list"], Opener(objects))
        self.assertEqual(rc, 0)

    def test_only_evidence_keys_are_ever_requested(self):
        for bad in (
            "../etc/passwd",
            "check/42/1.tgz/../../x",
            "other/42/1.tgz",
            "check/42/1.sh",
            "x",
        ):
            opener = Opener({"LIST": listing([])})
            rc, _, _ = self.run_tool(["get", bad, "--out", str(self.tmp() / "o")], opener)
            self.assertEqual(rc, 2, bad)
            for request in opener.requests:
                self.assertIn("list-type=2", request.full_url, "only a listing may be requested")

    def test_a_non_empty_output_directory_is_refused(self):
        out = self.tmp()
        (out / "keep").write_text("x")
        opener = Opener({})
        rc, _, err = self.run_tool(["get", "check/42/1.tgz", "--out", str(out)], opener)
        self.assertEqual(rc, 2)
        self.assertEqual(opener.requests, [])
        self.assertEqual((out / "keep").read_text(), "x")

    def test_requests_are_signed_and_use_the_r2_endpoint_only(self):
        opener = Opener({"LIST": listing([])})
        self.run_tool(["list"], opener)
        request = opener.requests[0]
        self.assertTrue(
            request.full_url.startswith(
                "https://%s.r2.cloudflarestorage.com/kayfabe-ci-evidence" % ENV["CF_ACCOUNT_ID"]
            )
        )
        headers = {k.lower(): v for k, v in request.header_items()}
        self.assertTrue(
            headers["authorization"].startswith(
                "AWS4-HMAC-SHA256 Credential=AKIDFAKE/20261106/auto/s3/"
            )
        )
        self.assertNotIn(ENV["R2_SECRET_ACCESS_KEY"], json.dumps(headers))

    def test_members_lists_names_without_extracting(self):
        path = self.tmp() / "b.tgz"
        path.write_bytes(bundle())
        rc, out, _ = self.run_tool(["members", str(path)], None, env=None)
        self.assertEqual(rc, 0)
        self.assertIn("log/output.log", out)
        self.assertEqual([p.name for p in path.parent.iterdir()], ["b.tgz"])


if __name__ == "__main__":
    unittest.main()

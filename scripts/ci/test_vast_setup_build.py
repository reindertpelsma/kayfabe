"""Recorded APT URI shape and deterministic parallel-prefetch contracts."""

import hashlib
import io
from pathlib import Path
import tempfile
import unittest

import setup_cache_build as build
from vast_setup import CacheRefused

BODY = b"deb-data"
SHA = hashlib.sha256(BODY).hexdigest()
URI = "'http://archive.ubuntu.com/ubuntu/pool/main/f/foo/foo_1.2_amd64.deb' foo_1.2_amd64.deb 8 MD5Sum:ignored\n"
CONTROL = {
    "Package": "foo",
    "Version": "1.2",
    "Architecture": "amd64",
    "Size": "8",
    "Filename": "pool/main/f/foo/foo_1.2_amd64.deb",
    "SHA256": SHA,
}


class Response(io.BytesIO):
    status = 200


class Opener:
    def __init__(self, body=BODY):
        self.body = body
        self.request = None

    def open(self, request, timeout):
        self.request = request
        return Response(self.body)


class SetupBuild(unittest.TestCase):
    def plan(self, text=URI, change=None):
        data = dict(CONTROL)
        if change:
            change(data)
        return build.parse_uris(text, lambda name, version, arch: data)

    def test_real_print_uris_shape_uses_signed_metadata_sha256(self):
        rows = self.plan()
        self.assertEqual(rows[0]["sha256"], SHA)
        self.assertEqual(rows[0]["size"], 8)
        self.assertEqual(rows[0]["control"]["Package"], "foo")

    def test_md5_field_is_never_the_authority(self):
        with self.assertRaises(CacheRefused):
            self.plan(change=lambda x: x.pop("SHA256"))

    def test_bad_sha256_refuses(self):
        with self.assertRaises(CacheRefused):
            self.plan(change=lambda x: x.update(SHA256="not-sha256"))

    def test_duplicate_destination_refuses(self):
        with self.assertRaises(CacheRefused):
            self.plan(URI + URI)

    def test_non_apt_host_refuses(self):
        with self.assertRaises(CacheRefused):
            self.plan(URI.replace("archive.ubuntu.com", "evil.example"))

    def test_package_index_path_must_match_uri(self):
        with self.assertRaises(CacheRefused):
            self.plan(change=lambda x: x.update(Filename="pool/other.deb"))

    def test_package_index_size_must_match_uri(self):
        with self.assertRaises(CacheRefused):
            self.plan(change=lambda x: x.update(Size="9"))

    def probe(self, status=200, data=100, seconds=1):
        return [
            {"host": name, "status": status, "bytes": data + i, "seconds": seconds}
            for i, name in enumerate(build.MIRRORS)
        ]

    def test_fastest_valid_mirror_is_deterministic(self):
        self.assertEqual(build.rank_probes(self.probe()), build.MIRRORS[2])

    def test_http_error_cannot_win_on_throughput(self):
        rows = self.probe()
        rows[2].update(status=404, bytes=10000000)
        self.assertEqual(build.rank_probes(rows), build.MIRRORS[1])

    def test_zero_second_measurement_cannot_win(self):
        rows = self.probe()
        rows[2].update(seconds=0)
        self.assertEqual(build.rank_probes(rows), build.MIRRORS[1])

    def test_all_http_failures_refuse(self):
        with self.assertRaises(CacheRefused):
            build.rank_probes(self.probe(status=404))

    def test_no_positive_measurements_refuse(self):
        with self.assertRaises(CacheRefused):
            build.rank_probes(self.probe(seconds=0))

    def fetch(self, body=BODY):
        row = self.plan()[0]
        opener = Opener(body)
        with tempfile.TemporaryDirectory() as d:
            path = build.fetch_deb(row, Path(d), build.MIRRORS[0], opener=opener)
            self.assertEqual(path.read_bytes(), body)
            self.assertEqual(opener.request.headers, {})
            self.assertTrue(opener.request.full_url.startswith("https://"))

    def test_prefetch_verifies_real_sha(self):
        self.fetch()

    def test_tampered_deb_refuses(self):
        with self.assertRaises(CacheRefused):
            self.fetch(b"evil-data")

    def test_same_size_wrong_deb_refuses(self):
        with self.assertRaises(CacheRefused):
            self.fetch(b"xxxxxxxx")

    def test_short_deb_refuses(self):
        with self.assertRaises(CacheRefused):
            self.fetch(b"x")


if __name__ == "__main__":
    unittest.main()

"""GPU-free cache contracts: reviewed identity, private locators, bounded extraction."""

import copy
import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import vast_setup as setup


def sha(data):
    return hashlib.sha256(data).hexdigest()


class Reply(io.BytesIO):
    status = 200


class Opener:
    def __init__(self, body=b"data"):
        self.body = body
        self.requests = []

    def open(self, request, timeout):
        self.requests.append((request, timeout))
        return Reply(self.body)


class SetupCache(unittest.TestCase):
    def fixture(self, root):
        (root / "firmware/kf-gop").mkdir(parents=True)
        (root / "Cargo.lock").write_bytes(b"root-lock")
        (root / "firmware/kf-gop/Cargo.lock").write_bytes(b"firmware-lock")
        return {
            "schema": 1,
            "image": setup.TARGET_IMAGE,
            "platform": {"os": "ubuntu", "version": "22.04", "arch": "x86_64"},
            "package_policy_sha256": sha(b"policy"),
            "locks": {
                "Cargo.lock": sha(b"root-lock"),
                "firmware/kf-gop/Cargo.lock": sha(b"firmware-lock"),
            },
            "tag": "setup-cache-test",
            "assets": [
                {"kind": kind, "name": kind + ".tar.gz", "size": 4, "sha256": sha(b"data")}
                for kind in ["debs", "vendor"]
            ],
        }

    def validate(self, change=None):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            lock = self.fixture(root)
            if change:
                change(lock, root)
            return setup.validate_lock(lock, root, lock["platform"], sha(b"policy"))

    def test_valid_reviewed_identity(self):
        self.assertEqual(self.validate()["schema"], 1)

    def test_wrong_cargo_lock_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.validate(lambda x, r: (r / "Cargo.lock").write_text("changed"))

    def test_wrong_firmware_lock_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.validate(lambda x, r: (r / "firmware/kf-gop/Cargo.lock").write_text("changed"))

    def test_wrong_image_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.validate(lambda x, r: x.update(image="other-image"))

    def test_missing_image_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.validate(lambda x, r: x.pop("image"))

    def test_stale_package_policy_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.validate(lambda x, r: x.update(package_policy_sha256="0" * 64))

    def test_missing_package_policy_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.validate(lambda x, r: x.pop("package_policy_sha256"))

    def test_missing_asset_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.validate(lambda x, r: x["assets"].pop())

    def test_duplicate_kind_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.validate(lambda x, r: x["assets"][1].update(kind="debs"))

    def test_artifact_over_decimal_two_gb_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.validate(lambda x, r: x["assets"][0].update(size=2_000_000_001))

    def test_boolean_asset_size_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.validate(lambda x, r: x["assets"][0].update(size=True))

    def download(self, body=b"data", mutate=None, opener=None):
        desc = {"kind": "debs", "name": "debs.tar.gz", "size": 4, "sha256": sha(b"data")}
        locator = {
            "name": desc["name"],
            "size": 4,
            "url": "https://release-assets.githubusercontent.com/key?secret=SIGNED",
        }
        if mutate:
            mutate(desc, locator)
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / desc["name"]
            setup.download_asset(locator, desc, path, opener=opener or Opener(body))
            self.assertEqual(path.read_bytes(), body)
            self.assertFalse(path.with_suffix(".partial").exists())

    def test_download_has_no_authentication_headers(self):
        opener = Opener()
        self.download(opener=opener)
        self.assertEqual(opener.requests[0][0].headers, {})

    def test_download_url_is_exact_and_redirects_disabled(self):
        opener = Opener()
        with patch.object(setup.urllib.request, "build_opener", return_value=opener) as build:
            self.download(opener=None)
        self.assertIsInstance(build.call_args.args[0], setup.NoRedirect)
        self.assertIn("secret=SIGNED", opener.requests[0][0].full_url)

    def test_download_wrong_hash_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.download(body=b"evil")

    def test_download_altered_equal_length_body_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.download(body=b"xxxx")

    def test_download_short_body_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.download(body=b"dat")

    def test_download_oversized_body_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.download(body=b"datax")

    def test_wrong_locator_size_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.download(mutate=lambda x, y: y.update(size=99))

    def test_wrong_locator_name_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.download(mutate=lambda x, y: y.update(name="other.tar.gz"))

    def test_foreign_url_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.download(mutate=lambda x, y: y.update(url="https://evil.example/key"))

    def test_plaintext_url_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.download(
                mutate=lambda x, y: y.update(url="http://release-assets.githubusercontent.com/x")
            )

    def test_http_error_does_not_print_signed_url(self):
        opener = Opener()
        opener.open = lambda *a, **k: (_ for _ in ()).throw(ValueError("secret=SIGNED"))
        with self.assertRaises(setup.CacheRefused) as error:
            self.download(opener=opener)
        self.assertNotIn("SIGNED", str(error.exception))

    def extract(self, name="vendor/crate/src.rs", kind=None, limit=1024):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            archive = root / "cache.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                info = tarfile.TarInfo(name)
                info.mode = 0o644
                if kind:
                    info.type = kind
                    info.linkname = "/outside"
                    tar.addfile(info)
                else:
                    info.size = 4
                    tar.addfile(info, io.BytesIO(b"data"))
            dest = root / "fresh"
            setup.unpack_checked(archive, dest, "vendor", limit=limit)
            return (dest / name).is_file()

    def test_safe_regular_source_is_extracted(self):
        self.assertTrue(self.extract())

    def test_parent_escape_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.extract("../escape")

    def test_absolute_escape_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.extract("/escape")

    def test_cross_component_path_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.extract("debs/escape")

    def test_symlink_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.extract(kind=tarfile.SYMTYPE)

    def test_hardlink_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.extract(kind=tarfile.LNKTYPE)

    def test_expansion_limit_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.extract(limit=3)

    def test_second_expansion_boundary_refuses(self):
        with self.assertRaises(setup.CacheRefused):
            self.extract(limit=1)

    def test_vendor_config_is_global_and_preserves_unrelated_values(self):
        with tempfile.TemporaryDirectory() as d:
            home = Path(d) / "cargo"
            home.mkdir()
            (home / "config.toml").write_text("[build]\njobs = 2\n")
            setup.configure_vendor(Path(d) / "vendor", home)
            text = (home / "config.toml").read_text()
            self.assertIn("jobs = 2", text)
            self.assertIn('replace-with = "kf-setup-vendor"', text)
            self.assertIn(str(Path(d) / "vendor"), text)

    def test_existing_source_override_refuses(self):
        with tempfile.TemporaryDirectory() as d:
            home = Path(d)
            (home / "config.toml").write_text('[source.crates-io]\nreplace-with="other"\n')
            with self.assertRaises(setup.CacheRefused):
                setup.configure_vendor(home / "vendor", home)

    def test_legacy_config_cannot_override_vendor(self):
        with tempfile.TemporaryDirectory() as d:
            home = Path(d)
            (home / "config").write_text('[source.crates-io]\nreplace-with="other"\n')
            with self.assertRaises(setup.CacheRefused):
                setup.configure_vendor(home / "vendor", home)


if __name__ == "__main__":
    unittest.main()

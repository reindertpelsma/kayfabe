#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline falsifiers for discovery, pinning, probes and bounded hashing."""
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("drv1_generate", HERE / "generate.py")
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)


class FakeClient:
    def __init__(self, replies):
        self.replies = replies
        self.calls = []

    def request(self, url, method="GET"):
        self.calls.append((method, url))
        return self.replies.get((method, url), {
            "status": 404, "body": "", "headers": {}, "checked_at": "2026-11-08T00:00:00Z", "final_url": url,
        })


def response(body="", status=200, length=123):
    return {"body": body, "status": status, "headers": {"content-length": str(length)},
            "checked_at": "2026-11-08T00:00:00Z"}


class Discovery(unittest.TestCase):
    def test_studio_uses_the_documented_studio_query_flags(self):
        from urllib.parse import parse_qs, urlsplit
        q = parse_qs(urlsplit(g.windows_search_url(535, True)).query)
        self.assertEqual((q['isWHQL'], q['upCRD']), (['0'], ['1']))

    def test_game_ready_query_requires_whql(self):
        from urllib.parse import parse_qs, urlsplit
        q = parse_qs(urlsplit(g.windows_search_url(570, False)).query)
        self.assertEqual((q['isWHQL'], q['upCRD'], q['release']), (['1'], ['0'], ['570']))

    def test_exact_published_tesla_url_is_preserved(self):
        u = 'https://us.download.nvidia.com/tesla/570.148.08/NVIDIA-Linux-x86_64-570.148.08.run'
        release = {'570': {'driver_info': [{'release_version': '570.148.08', 'runfile_url': {'x86_64': u}}]}}
        self.assertIn(u, g.linux_urls('570.148.08', release, {}))

    def test_historical_available_url_is_reprobed_without_rewriting_host(self):
        u = 'https://us.download.nvidia.com/tesla/535.104.12/NVIDIA-Linux-x86_64-535.104.12.run'
        self.assertIn(u, g.linux_urls('535.104.12', {}, {'535.104.12': u}))

    def test_index_accepts_single_and_double_quotes(self):
        self.assertEqual(g.versions_from_index("<a href='535.54.03/'>x</a><a href=\"620.70.06/\">y</a>"), ["535.54.03", "620.70.06"])

    def test_index_does_not_confuse_tags_files_or_old_branches(self):
        self.assertEqual(g.versions_from_index("<a href='525.1/'>x</a><a href='580.1.run'>x</a><a href='evil/580.2/'>x</a>"), [])

    def test_archive_extracts_x64_date_without_32bit_leak(self):
        s = '<h4><a href="/en-us/drivers/details/1/">Linux Display Driver - x86_64</a></h4><p>Version: 580.1<br>Operating System: Linux 64-bit<br>Release Date: August 1, 2025</p><h4>x86</h4><p>Version: 535.2<br>Operating System: Linux 32-bit<br>Release Date: May 1, 2023</p>'
        rows = g.parse_linux_archive(s)
        self.assertEqual([(r["version"], r["release_date"]) for r in rows], [("580.1", "2025-08-01")])

    def test_windows_branch_mapping_is_explicit(self):
        s = '<table><tr><td>R580</td><td>U12</td><td><a href="https://www.nvidia.com/en-us/drivers/details/1/">582.99</a></td><td>21-Oct-26</td></tr></table>'
        self.assertEqual(g.parse_windows_history(s)[0], {"branch": 580, "version": "582.99", "release_date": "2026-10-21", "details_url": "https://www.nvidia.com/en-us/drivers/details/1/"})

    def test_history_skips_old_branches(self):
        self.assertEqual(g.parse_windows_history('<tr><td>R525</td><td>U1</td><td>528.1</td><td>1-Jan-23</td></tr>'), [])

    def test_windows_api_uses_official_url_and_whql(self):
        row = {"downloadInfo": {"Version": "595.97", "DownloadURL": "https://us.download.nvidia.com/Windows/595.97/595.97-desktop-win10-win11-64bit-international-dch-whql.exe", "IsWHQL": "1", "ReleaseDate": "Tue Mar 24, 2026", "ID": "1"}}
        rows = g.parse_driver_api({"Success": "1", "IDS": [row]})
        self.assertEqual(rows[0]["version"], "595.97")
        self.assertEqual(rows[0]["release_date"], "2026-03-24")

    def test_windows_api_excludes_non_whql_and_non_desktop(self):
        for suffix in ["-notebook-whql.exe", "-desktop-hotfix.exe", "-datacenter-whql.exe"]:
            rows = g.parse_driver_api({"Success": "1", "IDS": [{"downloadInfo": {"Version": "580.1", "DownloadURL": f"https://us.download.nvidia.com/Windows/580.1/580.1{suffix}", "IsWHQL": "1"}}]})
            self.assertEqual(rows, [])

    def test_api_whql_zero_is_not_certified_by_filename(self):
        u = 'https://us.download.nvidia.com/Windows/580.1/580.1-desktop-win10-win11-64bit-international-dch-whql.exe'
        self.assertEqual(g.parse_driver_api({'Success': '1', 'IDS': [{'downloadInfo': {'Version': '580.1', 'IsWHQL': '0', 'DownloadURL': u}}]}), [])

    def test_enterprise_whql_zero_is_not_certified_by_filename(self):
        u = 'https://us.download.nvidia.com/Windows/580.1/580.1-quadro-rtx-desktop-notebook-win10-win11-64bit-international-dch-whql.exe'
        self.assertEqual(g.parse_driver_api({'Success': '1', 'IDS': [{'downloadInfo': {'Version': '580.1', 'IsWHQL': '0', 'DownloadURL': u}}]}), [])

    def test_no_results_is_not_an_empty_complete_archive(self):
        with self.assertRaisesRegex(ValueError, "no driver results"):
            g.parse_driver_api({"Success": "0", "IDS": []})

    def test_search_success_is_a_count_not_a_boolean(self):
        u = 'https://us.download.nvidia.com/Windows/576.02/576.02-desktop-win10-win11-64bit-international-dch-whql.exe'
        rows = g.parse_driver_api({'Success': '20', 'IDS': [{'downloadInfo': {'Version': '576.02', 'Release': '570', 'IsWHQL': '1', 'DownloadURL': u}}]})
        self.assertEqual(rows[0]['branch'], 570)

    def test_branch_assignment_preserves_release_field(self):
        u = 'https://us.download.nvidia.com/Windows/582.99/582.99-desktop-win10-win11-64bit-international-dch-whql.exe'
        rows = g.parse_driver_api({'Success': '1', 'IDS': [{'downloadInfo': {'Version': '582.99', 'Release': '580', 'IsWHQL': '1', 'DownloadURL': u}}]})
        self.assertEqual(rows[0]['branch'], 580)


class Checksums(unittest.TestCase):
    def test_matches_exact_installer_filename(self):
        self.assertEqual(g.parse_checksum("a" * 64 + "  NVIDIA-Linux-x86_64-580.1.run\n", "NVIDIA-Linux-x86_64-580.1.run"), "a" * 64)

    def test_other_arch_checksum_cannot_pin_x64(self):
        self.assertIsNone(g.parse_checksum("a" * 64 + " NVIDIA-Linux-aarch64-580.1.run", "NVIDIA-Linux-x86_64-580.1.run"))

    def test_no_compat32_checksum_cannot_pin_full_installer(self):
        self.assertIsNone(g.parse_checksum("a" * 64 + " NVIDIA-Linux-x86_64-580.1-no-compat32.run", "NVIDIA-Linux-x86_64-580.1.run"))

    def test_md5_is_not_sha256(self):
        self.assertIsNone(g.parse_checksum("a" * 32 + " NVIDIA-Linux-x86_64-580.1.run", "NVIDIA-Linux-x86_64-580.1.run"))

    def test_bsd_md5_cannot_be_used_as_sha256(self):
        self.assertIsNone(g.parse_checksum('SHA256 (driver.run) = ' + 'a' * 32, 'driver.run'))

    def test_conflicting_sha256s_are_rejected(self):
        name = "NVIDIA-Linux-x86_64-580.1.run"
        with self.assertRaisesRegex(ValueError, "conflicting"):
            g.parse_checksum("a" * 64 + "  " + name + "\n" + "b" * 64 + "  " + name, name)

    def test_conflicting_bsd_and_gnu_checksums_are_rejected(self):
        with self.assertRaisesRegex(ValueError, 'conflicting'):
            g.parse_checksum('a' * 64 + '  driver.run\nSHA256 (driver.run) = ' + 'b' * 64, 'driver.run')

    def test_bsd_checksum_format(self):
        self.assertEqual(g.parse_checksum("SHA256 (driver.run) = " + "A" * 64, "driver.run"), "a" * 64)


class Probes(unittest.TestCase):
    def setUp(self):
        self.url = "https://download.nvidia.com/XFree86/Linux-x86_64/580.1/NVIDIA-Linux-x86_64-580.1.run"

    def test_301_redirect_preserves_head_method(self):
        req = g.urllib.request.Request(self.url, method='HEAD')
        redirected = g.OfficialRedirect().redirect_request(req, None, 301, '', {}, self.url.replace('download.', 'us.download.'))
        self.assertEqual(redirected.get_method(), 'HEAD')

    def test_302_redirect_preserves_head_method(self):
        req = g.urllib.request.Request(self.url, method='HEAD')
        redirected = g.OfficialRedirect().redirect_request(req, None, 302, '', {}, self.url.replace('download.', 'us.download.'))
        self.assertEqual(redirected.get_method(), 'HEAD')

    def collect(self, head, extra=None):
        replies = {("HEAD", self.url): head}
        replies.update(extra or {})
        self.client = FakeClient(replies)
        return g.collect_installer({"version": "580.1", "os": "linux", "branch": 580, "url": self.url, "discovery_source_urls": [g.LINUX_INDEX]}, self.client)

    def test_head_200_and_official_sha256_pins(self):
        r = self.collect(response(), {("GET", self.url + ".sha256sum"): response("a" * 64 + "  NVIDIA-Linux-x86_64-580.1.run")})
        self.assertEqual((r["status"], r["size"], r["sha256"]), ("PINNED", 123, "a" * 64))
        self.assertEqual(r["probe"]["method"], "HEAD")

    def test_403_is_unknown_not_absent(self):
        self.assertEqual(self.collect(response(status=403))["status"], "UNKNOWN:probe-error")

    def test_timeout_is_unknown_not_absent(self):
        self.assertEqual(self.collect(response(status=None))["status"], "UNKNOWN:probe-error")

    def test_404_is_absent(self):
        self.assertEqual(self.collect(response(status=404))["status"], "UNAVAILABLE")

    def test_missing_sha256_stays_unpinned(self):
        self.assertEqual(self.collect(response())["status"], "UNPINNED:no-sha256")

    def test_md5_sidecar_is_not_an_active_pin(self):
        r = self.collect(response(), {("GET", self.url + ".md5sum"): response("a" * 32 + " NVIDIA-Linux-x86_64-580.1.run")})
        self.assertIsNone(r["sha256"])
        self.assertEqual(r['status'], 'UNPINNED:no-sha256')

    def test_missing_content_length_stays_unknown(self):
        h = response(); h["headers"] = {}
        self.assertEqual(self.collect(h)["status"], "UNKNOWN:probe-error")

    def test_probe_does_not_get_installer(self):
        self.collect(response())
        self.assertNotIn(("GET", self.url), self.client.calls)

    def test_fallback_sha256sum_txt(self):
        u = self.url.rsplit("/", 1)[0] + "/sha256sum.txt"
        r = self.collect(response(), {("GET", u): response("a" * 64 + " NVIDIA-Linux-x86_64-580.1.run")})
        self.assertEqual(r["sha256"], "a" * 64)
        self.assertEqual(r["checksum_source"]["url"], u)

    def test_sha256_reuse_requires_same_url_and_size(self):
        h = response()
        for old in [{"url": self.url, "size": 999, "sha256": "a" * 64}, {"url": self.url + "other", "size": 123, "sha256": "a" * 64}]:
            row = {"os": "windows", "branch": 580, "version": "580.1", "url": self.url, "discovery_source_urls": [g.WINDOWS_HISTORY]}
            self.assertNotEqual(g.collect_installer(row, FakeClient({("HEAD", self.url): h}), old)["status"], "PINNED")

    def test_redirect_must_stay_on_official_host(self):
        self.assertFalse(g.official_url("https://download.nvidia.com.evil.example/file"))

    def test_url_userinfo_is_refused(self):
        self.assertFalse(g.official_url("https://user:password@download.nvidia.com/file"))

    def test_deceptive_subdomain_is_refused(self):
        self.assertFalse(g.official_url('https://download.nvidia.com.evil.example/installer.exe'))

    def test_download_url_may_not_carry_userinfo_even_on_official_host(self):
        self.assertFalse(g.official_url('https://download.nvidia.com@evil.example/installer.exe'))

    def test_official_host_with_username_only_is_refused(self):
        self.assertFalse(g.official_url('https://user@download.nvidia.com/installer.exe'))

    def test_linux_installer_get_requires_explicit_hash_mode(self):
        with self.assertRaisesRegex(ValueError, 'explicit bounded hash'):
            g.Client().request('https://download.nvidia.com/driver.run')

    def test_windows_installer_get_requires_explicit_hash_mode(self):
        with self.assertRaisesRegex(ValueError, 'explicit bounded hash'):
            g.Client().request('https://download.nvidia.com/driver.exe')


class BoundedDownload(unittest.TestCase):
    def test_download_hashes_exact_bytes_and_deletes_file(self):
        with tempfile.TemporaryDirectory(prefix="DRV1-test-") as d:
            result = g.hash_stream(io.BytesIO(b"abc"), 3, Path(d))
            self.assertEqual(result, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
            self.assertEqual(list(Path(d).iterdir()), [])

    def test_truncated_download_is_not_a_pin_and_is_deleted(self):
        with tempfile.TemporaryDirectory(prefix="DRV1-test-") as d:
            with self.assertRaisesRegex(ValueError, "size"):
                g.hash_stream(io.BytesIO(b"abc"), 4, Path(d))
            self.assertEqual(list(Path(d).iterdir()), [])

    def test_oversize_download_is_not_a_pin_and_is_deleted(self):
        with tempfile.TemporaryDirectory(prefix="DRV1-test-") as d:
            with self.assertRaisesRegex(ValueError, "size"):
                g.hash_stream(io.BytesIO(b"abc"), 2, Path(d))
            self.assertEqual(list(Path(d).iterdir()), [])

    def test_large_size_refused_before_file_creation(self):
        with tempfile.TemporaryDirectory(prefix="DRV1-test-") as d:
            with self.assertRaisesRegex(ValueError, "15 GiB"):
                g.hash_stream(io.BytesIO(), 16 * 1024 ** 3, Path(d))
            self.assertEqual(list(Path(d).iterdir()), [])

    def test_nonpositive_size_refused_before_file_creation(self):
        with tempfile.TemporaryDirectory(prefix='DRV1-test-') as d:
            with self.assertRaisesRegex(ValueError, '15 GiB'):
                g.hash_stream(io.BytesIO(), 0, Path(d))
            self.assertEqual(list(Path(d).iterdir()), [])

    def test_disk_guard_fails_before_reading(self):
        with tempfile.TemporaryDirectory(prefix="DRV1-test-") as d, patch.object(g.shutil, "disk_usage", return_value=(1, 1, 5 * 1024 ** 3)):
            stream = io.BytesIO(b"abc")
            with self.assertRaisesRegex(ValueError, "6 GiB"):
                g.hash_stream(stream, 3, Path(d))
            self.assertEqual(stream.tell(), 0)
            self.assertEqual(list(Path(d).iterdir()), [])

    def test_disk_guard_reserves_space_for_the_expected_download(self):
        with tempfile.TemporaryDirectory(prefix='DRV1-test-') as d, patch.object(g.shutil, 'disk_usage', return_value=(1, 1, 6 * 1024 ** 3 + 1)):
            with self.assertRaisesRegex(ValueError, '6 GiB'):
                g.hash_stream(io.BytesIO(b'abc'), 3, Path(d))
            self.assertEqual(list(Path(d).iterdir()), [])


class Catalog(unittest.TestCase):
    @unittest.skipIf(__import__('os').environ.get('DRV1_FIXTURES_ONLY'), 'catalog checked in main suite')
    def test_offline_regeneration_is_byte_identical_without_client(self):
        with tempfile.TemporaryDirectory(prefix='DRV1-replay-') as d, patch.object(g, 'Client', side_effect=AssertionError('network must not be used')):
            out = Path(d) / 'catalog.json'
            g.main(['--offline', '--output', str(out)])
            self.assertEqual(out.read_bytes(), (HERE.parent / 'driver-availability.json').read_bytes())

    def test_curated_record_paths_exist_in_the_public_tree(self):
        meta = json.loads((HERE / 'sources.json').read_text())
        for row in meta['test_records']:
            with self.subTest(trace=row['trace']):
                self.assertTrue((HERE.parents[2] / row['trace']).is_file())

    def test_curated_known_bad_and_core_references_start_empty(self):
        meta = json.loads((HERE / 'sources.json').read_text())
        self.assertEqual(meta['known_bad'], [])
        self.assertEqual(meta['core_reference_versions'], [])


@unittest.skipIf(__import__('os').environ.get('DRV1_FIXTURES_ONLY'), 'runner contract tested outside product mutants')
class MutationRunner(unittest.TestCase):
    def load_runner(self):
        runner_spec = importlib.util.spec_from_file_location('drv1_runner_contract', HERE / 'mutants.py')
        runner = importlib.util.module_from_spec(runner_spec)
        runner_spec.loader.exec_module(runner)
        return runner

    def test_baseline_requests_tap_even_when_default_reporter_is_spec(self):
        runner = self.load_runner()
        from types import SimpleNamespace
        with patch.object(runner.subprocess, 'run', return_value=SimpleNamespace(returncode=0, stdout='TAP version 13\nok 1 - baseline\n', stderr='')) as run:
            result = runner.run('js', 'baseline', runner.JS)
        self.assertIn('--test-reporter=tap', run.call_args.args[0])
        self.assertEqual(result['exit_code'], 0)

    def test_mutant_requests_tap_and_counts_distinct_named_failures(self):
        runner = self.load_runner()
        from types import SimpleNamespace
        output = 'TAP version 13\nnot ok 1 - first killer\nnot ok 2 - second killer\n'
        with patch.object(runner.subprocess, 'run', return_value=SimpleNamespace(returncode=1, stdout=output, stderr='')) as run:
            result = runner.run('js', 'probe-mutant', runner.JS)
        self.assertIn('--test-reporter=tap', run.call_args.args[0])
        self.assertEqual(result['killer_count'], 2)


if __name__ == "__main__":
    unittest.main()

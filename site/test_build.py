"""Small local tests; temporary git tags stand in for published releases."""

import importlib.util
import io
import tarfile
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


def builder():
    spec = importlib.util.spec_from_file_location(
        "site_build", Path(__file__).with_name("build.py")
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Versions(unittest.TestCase):
    # FALSIFIER: unpublished tags move the landing, an older tag is skipped, or tag config runs.
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="site1-test-")
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Site fixture")
        self.git("config", "user.email", "fixture@example.com")
        (self.repo / "docs").mkdir()
        (self.repo / "docs/live.md").write_text("# Internal\nSTATUS: LIVE\n")
        self.commit("no manual")
        self.git("tag", "v0.8.0")
        (self.repo / "docs/howto").mkdir()
        (self.repo / "docs/howto/SUMMARY.md").write_text(
            "# Summary\n\n- [Start](index.md)\n- [Windows](windows.md)\n"
        )
        (self.repo / "docs/howto/index.md").write_text("# Start\n")
        (self.repo / "docs/howto/windows.md").write_text("# Windows\n")
        (self.repo / "docs/FAQ.md").write_text("# FAQ\n")
        self.commit("base")
        self.git("tag", "v0.9.0")
        (self.repo / "site/content").mkdir(parents=True)
        (self.repo / "site/content/_index.md").write_text("+++\ntitle='tag landing'\n+++\n")
        (self.repo / "site/config.toml").write_text("this must never configure a tagged build")
        (self.repo / "site/build.sh").write_text("exit 99\n")
        self.commit("landing")
        self.git("tag", "v0.10.0")
        self.git("tag", "v1.0.0")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.repo, text=True).strip()

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "-qm", message)

    def release(self, tag, date="2026-10-26T00:00:00Z", **patch):
        return dict(
            tag_name=tag, published_at=date, draft=False, assets=[{"name": "binary"}], **patch
        )

    def test_no_published_release_keeps_master_but_tag_docs_exist(self):
        plan = builder().version_plan(self.repo, [])
        self.assertEqual(plan["versions"], ["master", "v1.0.0", "v0.10.0", "v0.9.0"])
        self.assertEqual((plan["root"], plan["latest"]), ("master", "master"))

    def test_zero_tags_has_only_master(self):
        self.git("tag", "-d", "v0.8.0", "v0.9.0", "v0.10.0", "v1.0.0")
        self.assertEqual(builder().version_plan(self.repo, [])["versions"], ["master"])

    def test_published_artifacts_select_newest_publication(self):
        releases = [
            self.release("v0.10.0", "2026-10-26T00:00:00Z"),
            self.release("v1.0.0", "2026-10-25T00:00:00Z"),
        ]
        plan = builder().version_plan(self.repo, releases)
        self.assertEqual((plan["root"], plan["latest"]), ("v0.10.0", "v0.10.0"))

    def test_no_assets_and_drafts_do_not_move_landing(self):
        releases = [self.release("v1.0.0")]
        releases[0]["assets"] = []
        draft = self.release("v0.10.0")
        draft["draft"] = True
        self.assertEqual(builder().version_plan(self.repo, releases + [draft])["root"], "master")

    def test_assetless_newer_release_does_not_displace_published_manual(self):
        releases = [self.release("v0.10.0", "2026-10-25T00:00:00Z"), self.release("v1.0.0")]
        releases[1]["assets"] = []
        self.assertEqual(builder().version_plan(self.repo, releases)["latest"], "v0.10.0")

    def test_draft_newer_release_does_not_displace_published_landing(self):
        releases = [self.release("v0.10.0", "2026-10-25T00:00:00Z"), self.release("v1.0.0")]
        releases[1]["draft"] = True
        self.assertEqual(builder().version_plan(self.repo, releases)["root"], "v0.10.0")

    def test_publication_sort_works_with_api_order_reversed(self):
        releases = [self.release("v1.0.0", "2026-10-25T00:00:00Z"), self.release("v0.10.0")]
        self.assertEqual(builder().version_plan(self.repo, releases)["latest"], "v0.10.0")

    def test_master_override_keeps_published_manual_selected(self):
        plan = builder().version_plan(self.repo, [self.release("v0.10.0")], "master")
        self.assertEqual((plan["root"], plan["latest"]), ("master", "v0.10.0"))

    def test_old_tag_without_site_still_gets_docs(self):
        plan = builder().version_plan(self.repo, [self.release("v0.9.0")])
        self.assertEqual(plan["root"], "master")
        self.assertEqual(plan["latest"], "v0.9.0")
        self.assertIn("v0.9.0", plan["versions"])

    def test_master_override_affects_only_landing(self):
        plan = builder().version_plan(self.repo, [self.release("v1.0.0")], "master")
        self.assertEqual(plan["root"], "master")
        self.assertEqual(plan["latest"], "v1.0.0")

    def test_snapshot_contains_tag_data_not_configs_or_scripts(self):
        dest = self.repo / "snapshot"
        builder().snapshot(self.repo, "v0.10.0", dest)
        self.assertFalse((dest / "docs/live.md").exists())
        self.assertTrue((dest / "docs/howto/SUMMARY.md").is_file())
        self.assertTrue((dest / "site/content/_index.md").is_file())
        self.assertFalse((dest / "site/config.toml").exists())
        self.assertFalse((dest / "site/build.sh").exists())

    def test_tags_without_manual_are_absent_even_if_published(self):
        plan = builder().version_plan(self.repo, [self.release("v0.8.0")])
        self.assertNotIn("v0.8.0", plan["versions"])
        self.assertEqual(plan["latest"], "master")

    def test_manual_order_and_faq_only(self):
        book = self.repo / "book"
        selected = builder().manual_source(self.repo, book)
        self.assertEqual(selected, ["index.md", "windows.md", "FAQ.md"])
        self.assertFalse((book / "live.md").exists())
        self.assertTrue((book / "SUMMARY.md").read_text().endswith("- [FAQ](FAQ.md)\n"))

    def test_manual_does_not_copy_internal_docs(self):
        book = self.repo / "book"
        builder().manual_source(self.repo, book)
        self.assertEqual(
            {p.name for p in book.iterdir()}, {"SUMMARY.md", "index.md", "windows.md", "FAQ.md"}
        )

    def test_internal_links_fail_publication(self):
        for link in [
            "../design/secret.html",
            "https://github.com/reindertpelsma/kayfabe/blob/master/docs/reference/other.md",
            "/docs/OWNER_RULINGS.md",
            "https://github.com/reindertpelsma/kayfabe/blob/master/docs/STATUS_DETAIL.md",
        ]:
            with self.subTest(link=link), self.assertRaises(ValueError):
                builder().check_page(
                    '<title>Manual</title><a href="' + link + '">internal</a>',
                    "docs/master/index.html",
                )

    def test_more_internal_links_fail_publication(self):
        for link in [
            "../../archive/old.md",
            "/docs/WINDOWS_GUEST_SETUP.md",
            "../handoff-notes.md",
            "/docs/STATUS_AND_HANDOFF.md",
        ]:
            with self.subTest(link=link), self.assertRaises(ValueError):
                builder().check_page(
                    '<title>Manual</title><a href="' + link + '">internal</a>',
                    "docs/master/index.html",
                )

    def test_vendor_documentation_links_remain_allowed(self):
        builder().check_page(
            '<title>Manual</title><a href="https://vendor.example/docs/reference/setup">'
            "Vendor instructions</a>",
            "docs/master/index.html",
        )

    def test_other_repository_documentation_links_remain_allowed(self):
        builder().check_page(
            '<title>Manual</title><a href="https://github.com/other/project/docs/design/">'
            "Upstream docs</a>",
            "docs/master/index.html",
        )

    def test_missing_title_fails_publication(self):
        with self.assertRaises(ValueError):
            builder().check_page("<p>Missing title</p>", "docs/master/index.html")

    def test_empty_title_fails_publication(self):
        with self.assertRaises(ValueError):
            builder().check_page("<title> </title>", "docs/master/index.html")

    def test_release_canonicals_use_real_latest_page(self):
        module = builder()
        output = self.repo / "output"
        for version in ["master", "v0.9.0", "v1.0.0"]:
            folder = output / "docs" / version
            folder.mkdir(parents=True)
            (folder / "index.html").write_text("<head><title>Manual</title></head>")
        module.metadata(output, {"versions": ["master", "v0.9.0", "v1.0.0"], "latest": "v1.0.0"})
        text = (output / "docs/v0.9.0/index.html").read_text()
        self.assertIn('rel="canonical" href="https://kayfabe.dev/docs/v1.0.0/"', text)
        self.assertNotIn("/docs/v0.9.0/", (output / "sitemap.xml").read_text())
        self.assertIn("/docs/v1.0.0/", (output / "sitemap.xml").read_text())

    def test_release_canonicals_keep_chapter_path(self):
        module = builder()
        output = self.repo / "output"
        folder = output / "docs/v0.9.0"
        folder.mkdir(parents=True)
        (folder / "windows.html").write_text("<head><title>Windows</title></head>")
        module.metadata(output, {"versions": ["v0.9.0", "v1.0.0"], "latest": "v1.0.0"})
        text = (folder / "windows.html").read_text()
        self.assertIn('rel="canonical" href="https://kayfabe.dev/docs/v1.0.0/windows.html"', text)

    def fake_landing(self, source, destination, *args):
        destination.mkdir(parents=True, exist_ok=True)
        (destination / "index.html").write_text("<head><title>Landing</title></head><main></main>")

    def assert_master_book_config(self, source):
        module = builder()
        work = self.repo / "work"
        work.mkdir()
        output = self.repo / "out"

        def rendered(command, **kwargs):
            destination = Path(command[-1])
            destination.mkdir(parents=True)
            (destination / "toc.html").write_text("<head></head>")

        with patch.object(module.subprocess, "run", side_effect=rendered):
            module.docs_build(
                source,
                "v0.10.0",
                {"versions": ["v0.10.0"], "latest": "v0.10.0"},
                work,
                output,
                {"mdbook": "mdbook"},
            )
        self.assertEqual(
            (work / "book/book.toml").read_text(), (module.SITE / "book.toml").read_text()
        )

    def test_tag_missing_book_config_uses_master(self):
        self.assert_master_book_config(self.repo)

    def test_tag_broken_book_config_uses_master(self):
        (self.repo / "site/book.toml").write_text("this is not TOML")
        self.assert_master_book_config(self.repo)

    def test_oldest_manual_render_failure_fails_build(self):
        module = builder()

        def render(source, version, *args):
            if version == "v0.9.0":
                raise subprocess.CalledProcessError(1, "mdbook")

        with patch.object(module, "docs_build", side_effect=render), patch.object(
            module, "landing_build", side_effect=self.fake_landing
        ), self.assertRaises(subprocess.CalledProcessError):
            module.build(self.repo, {}, [])

    def test_old_landing_render_failure_fails_build(self):
        module = builder()

        def render(source, *args):
            if source != self.repo:
                raise subprocess.CalledProcessError(1, "zola")
            self.fake_landing(source, *args)

        with patch.object(module, "docs_build"), patch.object(
            module, "landing_build", side_effect=render
        ), self.assertRaises(subprocess.CalledProcessError):
            module.build(self.repo, {}, [])

    def assert_bad_tool_hash(self, name):
        module = builder()
        config = self.repo / "config"
        config.mkdir()
        cache = self.repo / "cache"
        cache.mkdir()
        (config / "tools.json").write_text(
            json.dumps(
                {
                    name: {
                        "version": "1",
                        "url": "https://example.invalid/tool.tar.gz",
                        "sha256": "0" * 64,
                    }
                }
            )
        )
        with tarfile.open(cache / (name + "-1.tar.gz"), "w:gz") as archive:
            entry = tarfile.TarInfo(name)
            entry.size = 4
            archive.addfile(entry, io.BytesIO(b"tool"))
        with patch.object(module, "SITE", config), self.assertRaisesRegex(ValueError, "SHA-256"):
            module.tools(cache)

    def test_zola_download_hash_is_required(self):
        self.assert_bad_tool_hash("zola")

    def test_mdbook_download_hash_is_required(self):
        self.assert_bad_tool_hash("mdbook")

    def test_broken_links_are_reported_without_failing(self):
        dest = self.repo / "public"
        dest.mkdir()
        (dest / "index.html").write_text(
            '<a href="ok.html#yes">yes</a><a href="missing.html">missing</a>'
            '<a href="ok.html#absent">absent</a><a href="https://example.com">external</a>'
        )
        (dest / "ok.html").write_text('<h1 id="yes">OK</h1>')
        result = builder().broken_links(dest)
        self.assertEqual(len(result), 2)
        self.assertEqual({item["reason"] for item in result}, {"file missing", "anchor missing"})

    def test_missing_file_link_is_reported(self):
        output = self.repo / "public"
        output.mkdir()
        (output / "index.html").write_text('<a href="missing.html">Broken</a>')
        self.assertEqual(builder().broken_links(output)[0]["target"], "missing.html")


if __name__ == "__main__":
    unittest.main()

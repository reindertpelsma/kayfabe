"""Small local tests; temporary git tags stand in for published releases."""

import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest


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
        (self.repo / "docs/live.md").write_text("# Live\n\n**STATUS: LIVE**, fixture\n")
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
        self.git("tag", "-d", "v0.9.0", "v0.10.0", "v1.0.0")
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
        self.assertTrue((dest / "docs/live.md").is_file())
        self.assertTrue((dest / "site/content/_index.md").is_file())
        self.assertFalse((dest / "site/config.toml").exists())
        self.assertFalse((dest / "site/build.sh").exists())

    def test_live_filter_uses_first_status_near_top(self):
        docs = self.repo / "docs"
        (docs / "old.md").write_text("STATUS: SUPERSEDED\nSTATUS: LIVE\n")
        (docs / "research.md").write_text("**STATUS: RESEARCH**\n")
        (docs / "late.md").write_text("\n" * 50 + "STATUS: LIVE\n")
        self.assertEqual([p.name for p in builder().live_documents(docs)], ["live.md"])

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


if __name__ == "__main__":
    unittest.main()

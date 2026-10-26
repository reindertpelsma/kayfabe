"""Validate the rendered site, after build.py; run in PRs and production builds."""

from html.parser import HTMLParser
import json
from pathlib import Path
import unittest
from urllib.parse import quote, urlsplit
import xml.etree.ElementTree as ET

SITE = Path(__file__).resolve().parent
OUTPUT = SITE / "public"


class Metadata(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.tags = {}
        self.forms = []
        self.selects = []
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        values = dict(attrs)
        if tag == "title":
            self.tags["title"] = True
        elif tag == "meta":
            self.tags[values.get("name", values.get("property", values.get("http-equiv")))] = (
                values.get("content")
            )
        elif tag == "link" and values.get("rel") == "canonical":
            self.tags["canonical"] = values.get("href")
        elif tag == "form":
            self.forms.append(values)
        elif tag == "select":
            self.selects.append(values)


class BuiltOutput(unittest.TestCase):
    # FALSIFIER: any rendered page lacks SEO, indexes master/staging, or aliases latest canonically.
    def setUp(self):
        self.report = json.loads((SITE / "build-report.json").read_text())

    def test_every_page_has_absolute_canonical_and_metadata(self):
        required = {"title", "description", "og:title", "og:description", "og:url", "canonical"}
        for path in OUTPUT.rglob("*.html"):
            meta = Metadata(path).tags
            self.assertTrue(required <= meta.keys(), str(path.relative_to(OUTPUT)))
            self.assertTrue(meta["canonical"].startswith("https://kayfabe.dev/"))
            self.assertNotIn("/docs/latest/", meta["canonical"])
            self.assertTrue(meta["description"])

    def test_master_and_staging_are_noindex(self):
        for folder in [OUTPUT / "staging", OUTPUT / "docs/master"]:
            for path in folder.rglob("*.html"):
                self.assertEqual(Metadata(path).tags.get("robots"), "noindex", str(path))

    def test_sitemap_contains_only_indexable_absolute_urls(self):
        urls = [e.text for e in ET.parse(OUTPUT / "sitemap.xml").iter() if e.tag.endswith("loc")]
        self.assertIn("https://kayfabe.dev/", urls)
        self.assertIn("https://kayfabe.dev/docs/", urls)
        for url in urls:
            self.assertTrue(url.startswith("https://kayfabe.dev/"))
            self.assertNotIn("/staging/", url)
            self.assertNotIn("/docs/master/", url)
            self.assertNotIn("/docs/latest/", url)
            path = OUTPUT / urlsplit(url).path.lstrip("/")
            if path.is_dir():
                path /= "index.html"
            self.assertNotEqual(Metadata(path).tags.get("robots"), "noindex")

    def test_robots_uses_sitemap_without_disallow(self):
        text = (OUTPUT / "robots.txt").read_text()
        self.assertIn("Sitemap: https://kayfabe.dev/sitemap.xml", text)
        self.assertNotIn("Disallow", text)
        self.assertTrue((OUTPUT / "404.html").is_file())

    def test_only_user_manual_is_published_with_builtin_search(self):
        for version in self.report["versions"]:
            book = OUTPUT / "docs" / version
            self.assertFalse((book / "design").exists())
            self.assertFalse((book / "OWNER_RULINGS.html").exists())
            self.assertTrue((book / "searchindex.js").is_file())
            self.assertIn("URL_SEARCH_PARAM = 'search'", (book / "searcher.js").read_text())
        self.assertEqual(
            (OUTPUT / "docs/index.html").read_text().count("Design and internals (GitHub)"), 1
        )

    def test_versions_json_search_default_and_plain_links(self):
        versions = json.loads((OUTPUT / "docs/versions.json").read_text())
        self.assertEqual([v["version"] for v in versions], self.report["versions"])
        master = next(v for v in versions if v["version"] == "master")
        self.assertEqual(master["label"], "master (unreleased)")
        self.assertFalse(master["latest"])
        latest = self.report["latest"]
        self.assertEqual(
            [v["version"] for v in versions if v["latest"]], [] if latest == "master" else [latest]
        )
        target = "/docs/" + quote(latest, safe="/") + "/"
        for path in [OUTPUT / "index.html", OUTPUT / "docs/index.html"]:
            self.assertIn({"action": target, "method": "get"}, Metadata(path).forms)
            self.assertIn('name="search"', path.read_text())
        for path in [OUTPUT / "docs/index.html", OUTPUT / "docs/latest/index.html"]:
            self.assertEqual(Metadata(path).tags["refresh"], "0;url=" + target)
        index = (OUTPUT / "docs/index.html").read_text()
        for version in versions:
            self.assertIn(version["label"], index)
        self.assertIn(target, index)
        self.assertEqual(
            Metadata(OUTPUT / "docs/master/index.html").selects[0]["data-current"], "master"
        )


if __name__ == "__main__":
    unittest.main()

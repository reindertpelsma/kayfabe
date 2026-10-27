"""Validate the rendered site, after build.py; run in PRs and production builds."""

from html.parser import HTMLParser
import json
from pathlib import Path
import re
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
        required = {
            "title",
            "description",
            "og:title",
            "og:description",
            "og:url",
            "og:type",
            "twitter:card",
            "twitter:title",
            "twitter:description",
            "canonical",
        }
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
        # /docs/ is a real index page; only /docs/latest/ redirects.
        self.assertNotIn("refresh", Metadata(OUTPUT / "docs/index.html").tags)
        self.assertEqual(Metadata(OUTPUT / "docs/latest/index.html").tags["refresh"], "0;url=" + target)
        index = (OUTPUT / "docs/index.html").read_text()
        for version in versions:
            self.assertIn(version["label"], index)
        self.assertIn(target, index)
        self.assertEqual(
            Metadata(OUTPUT / "docs/master/index.html").selects[0]["data-current"], "master"
        )


class Pages(HTMLParser):
    """Resource references: every tag attribute that makes the browser fetch something."""

    def __init__(self, text):
        super().__init__()
        self.resources, self.links, self.text_ids = [], [], set()
        self.feed(text)

    def handle_starttag(self, tag, attrs):
        values = dict(attrs)
        if "id" in values:
            self.text_ids.add(values["id"])
        if tag == "a" and values.get("href"):
            self.links.append(values["href"])
        elif tag == "link" and values.get("rel") != "canonical" and values.get("href"):
            self.resources.append((tag, values["href"]))
        for key in ["src", "srcset", "data", "poster"]:
            if values.get(key):
                self.resources.append((tag, values[key]))
        if tag == "form" and values.get("action"):
            self.resources.append((tag, values["action"]))


def is_foreign(url):
    parts = urlsplit(url)
    return bool(parts.netloc) and parts.netloc != "kayfabe.dev" or url.startswith("//")


class SelfContained(unittest.TestCase):
    # FALSIFIER: any built page, stylesheet or script makes the browser fetch from another host.
    def test_no_third_party_resources_in_html(self):
        for path in OUTPUT.rglob("*.html"):
            for tag, url in Pages(path.read_text()).resources:
                self.assertFalse(is_foreign(url), f"{path.relative_to(OUTPUT)}: <{tag}> {url}")

    def test_no_third_party_resources_in_css_or_js(self):
        reference = re.compile(r"""(?:@import\s+(?:url\()?|url\()\s*['"]?(https?:)?//""", re.I)
        for path in list(OUTPUT.rglob("*.css")):
            self.assertIsNone(reference.search(path.read_text()), str(path.relative_to(OUTPUT)))
        for name in ["site.js", "docs/master/theme/versions.js"]:
            path = OUTPUT / name
            if path.is_file():
                self.assertNotRegex(path.read_text(), r"https?://", name)

    def test_landing_weight_is_small(self):
        page = (OUTPUT / "index.html").read_text()
        self.assertLess(len(page.encode()), 60_000)
        self.assertLess((OUTPUT / "style.css").stat().st_size, 20_000)
        for image in re.findall(r'<img[^>]+src="([^"]+)"', page):
            self.assertLess((OUTPUT / image.lstrip("/")).stat().st_size, 300_000)


class LandingAndDocsIndex(unittest.TestCase):
    def setUp(self):
        self.report = json.loads((SITE / "build-report.json").read_text())
        self.landing = (OUTPUT / "index.html").read_text()

    def test_download_block_matches_the_release_list(self):
        release = self.report["release"]
        if release is None:
            self.assertIn("No release yet: build from source", self.landing)
            self.assertNotIn("/releases/download/", self.landing)
        else:
            self.assertNotIn("No release yet", self.landing)
            self.assertIn(release["url"], self.landing)
            for asset in release["assets"]:
                self.assertIn(asset["url"], self.landing)

    def test_landing_has_the_required_blocks(self):
        for text in [
            "Status: research",
            "No installer yet",
            "Windows is not stable",
            "What works today",
            "Try it, or help",
            "Contribute and community",
            "How fast, with the numbers",
            "Who it is for",
            "Works today",
            "Not yet",
            "VFIO passthrough",
            "NVIDIA vGPU",
            "API forwarding",
            "Apache-2.0 OR GPL-2.0-or-later",
        ]:
            self.assertIn(text, self.landing)
        for banned in ["loads nothing from", "other hosts", "code you have to trust"]:
            self.assertNotIn(banned, self.landing)
        self.assertEqual(self.landing.count("<h1"), 1)
        self.assertIn("<svg", self.landing)
        self.assertRegex(self.landing, r'<img[^>]+alt="[^"]+"')
        self.assertIn('class="skip-link"', self.landing)

    def test_landing_has_no_banned_claim_and_every_block_has_a_source(self):
        import claims_guard

        visible = re.sub(r"<!--.*?-->", "", self.landing, flags=re.S)
        self.assertEqual(claims_guard.find(visible), [])
        self.assertGreaterEqual(self.landing.count("<!-- src:"), 30)
        # Every section on the page keeps its source next to it.
        for section in re.findall(r"<section.*?</section>", self.landing, flags=re.S):
            if "hero-search" in section and "<h2" not in section.split("</h2>")[0]:
                continue
            self.assertIn("<!-- src:", section, section[:80])

    def test_landing_keeps_the_honest_parts(self):
        for text in [
            "27 to 31%",
            "experimental build (unmerged, Heaven only)",
            "not on master",
            "Your licences",
            "use nvkvm-pv",
            "one guest per GPU",
        ]:
            self.assertIn(text, re.sub(r"<[^>]+>", "", self.landing).replace("\n", " "))
        self.assertNotIn("paguro", self.landing.lower())

    def test_docs_index_follows_summary_order(self):
        summary = (SITE.parent / "docs/howto/SUMMARY.md").read_text()
        files = re.findall(r"\]\(([^)]+)\.md\)", summary)
        index = (OUTPUT / "docs/index.html").read_text()
        if self.report["latest"] != "master":
            return  # the released manual's SUMMARY is not the working tree's
        positions = [index.index(f'href="/docs/master/{name}.html"') for name in files]
        self.assertEqual(positions, sorted(positions))
        self.assertIn("/docs/master/FAQ.html", index)
        self.assertEqual(index.count("Design and internals (GitHub)"), 1)

    def test_manual_pages_have_breadcrumbs_edit_link_and_pager(self):
        book = OUTPUT / "docs/master"
        if not book.is_dir():
            return
        page = (book / "security.html").read_text()
        self.assertIn('aria-label="Breadcrumb"', page)
        self.assertIn(
            "https://github.com/reindertpelsma/kayfabe/edit/master/docs/howto/security.md", page
        )
        self.assertIn('rel="prev"', page)
        self.assertIn('class="skip-link"', page)
        self.assertIn(">View and edit source on GitHub</a>", page)
        faq = (book / "FAQ.html").read_text()
        self.assertIn("/edit/master/docs/FAQ.md", faq)
        goals = (book / "goals.html").read_text()
        self.assertIn("Goals and plans", goals)
        self.assertIn("/edit/master/docs/howto/goals.md", goals)
        self.assertIn("only QEMU is supported", goals)
        self.assertIn('class="kf-label kf-measured"', (book / "windows-guest.html").read_text())
        self.assertIn('class="kf-gap"', (book / "security.html").read_text())

    def test_about_and_security_pages_exist_and_are_indexable(self):
        for name in ["about", "security"]:
            tags = Metadata(OUTPUT / name / "index.html").tags
            self.assertNotIn("robots", tags)
            self.assertEqual(tags["canonical"], f"https://kayfabe.dev/{name}/")
        urls = [e.text for e in ET.parse(OUTPUT / "sitemap.xml").iter() if e.tag.endswith("loc")]
        self.assertIn("https://kayfabe.dev/about/", urls)


if __name__ == "__main__":
    unittest.main()

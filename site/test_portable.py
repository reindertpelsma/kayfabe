"""Portable output falsifiers: URL resolution, byte preservation, and offline navigation."""

import importlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class PortableURLs(unittest.TestCase):
    def setUp(self):
        self.mod = importlib.import_module("portable")
        self.temp = tempfile.TemporaryDirectory(prefix="site-portable-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in [
            "index.html",
            "about/index.html",
            "docs/master/index.html",
            "docs/master/chapter.html",
            "images/one.png",
            "images/two.png",
            "images/one two.png",
            "images/café.png",
        ]:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture")

    def rewrite(self, text, page="docs/master/chapter.html"):
        return self.mod.html_urls(text, self.root / page, self.root)

    def test_depth_index_queries_and_encoded_paths(self):
        self.assertEqual(
            self.rewrite('<a href="/about/?x=1&amp;y=2#part">'),
            '<a href="../../about/index.html?x=1&amp;y=2#part">',
        )
        self.assertEqual(
            self.rewrite("<a href='https://kayfabe.dev/docs/master/'>"),
            "<a href='index.html'>",
        )
        self.assertEqual(self.rewrite('<a href="/">'), '<a href="../../index.html">')

    def test_encoded_paths_and_unquoted_attributes(self):
        self.assertEqual(
            self.rewrite('<a href="/images/one%20two.png?q=a%20b">'),
            '<a href="../../images/one%20two.png?q=a%20b">',
        )
        self.assertEqual(
            self.rewrite('<img src="/images/caf%C3%A9.png">'),
            '<img src="../../images/caf%C3%A9.png">',
        )
        self.assertEqual(
            self.rewrite("<a href=/about/ >"), "<a href=../../about/index.html >"
        )

    def test_assets_srcset_forms_refresh_and_metadata(self):
        source = (
            '<img src="/images/one.png" srcset="/images/one.png 1x, /images/two.png 2x">'
            '<form action="/docs/master/"></form>'
            '<meta http-equiv="refresh" content="0; url=/about/">'
            '<meta property="og:url" content="https://kayfabe.dev/about/">'
        )
        result = self.rewrite(source)
        self.assertIn('src="../../images/one.png"', result)
        self.assertIn(
            'srcset="../../images/one.png 1x, ../../images/two.png 2x"', result
        )
        self.assertIn('action="index.html"', result)
        self.assertIn('content="0; url=../../about/index.html"', result)
        self.assertIn('content="../../about/index.html"', result)

    def test_only_attribute_value_bytes_change(self):
        source = (
            '<!DOCTYPE html>\r\n<!-- href="/about/" -->\n'
            "<script>const html = '<a href=\"/about/\">';</script>\n"
            '<pre>ordinary &lt;a href="/about/"&gt; text</pre>\n'
            '<A  HREF = "/about/" data-note="a > b">É &amp; &#169;</A>'
        )
        expected = source.replace('HREF = "/about/"', 'HREF = "../../about/index.html"')
        self.assertEqual(self.rewrite(source), expected)

    def test_external_fragments_data_and_non_url_content_unchanged(self):
        source = (
            '<a href="https://elsewhere.example/about/">x</a><a href="#part">x</a>'
            '<a href="mailto:a@example.com">x</a>'
            '<img srcset="data:image/png;base64,AAAA 1x, /images/two.png 2x">'
            '<meta name="description" content="A /about/ description">'
        )
        self.assertEqual(
            self.rewrite(source),
            source.replace("/images/two.png", "../../images/two.png"),
        )

    def test_css_urls_preserve_comments_and_strings(self):
        css = '/* url(/bad) */ .a{content:"url(/not-a-url)";background:url("/images/one.png")} '
        self.assertEqual(
            self.mod.css_urls(css, self.root / "style.css", self.root),
            css.replace('url("/images/one.png")', 'url("images/one.png")'),
        )
        self.assertEqual(
            self.mod.css_urls(
                "x{background:url(data:image/png;base64,AA)}",
                self.root / "style.css",
                self.root,
            ),
            "x{background:url(data:image/png;base64,AA)}",
        )

    def test_base_tag_cannot_rebase_404_assets_outside_book(self):
        # mdBook's generated 404 has <base href="/">, unlike normal chapters.
        self.assertEqual(
            self.rewrite('<base href="/"><link href="theme/book.css">'),
            '<base href="chapter.html"><link href="theme/book.css">',
        )
        self.assertEqual(
            self.rewrite('<BASE HREF="https://kayfabe.dev/">'),
            '<BASE HREF="chapter.html">',
        )


class PortableNavigation(unittest.TestCase):
    def run_js(self, protocol, missing=False, current="master", target="v1"):
        script = Path(__file__).with_name("book-theme") / "versions.js"
        harness = r"""
const vm=require('node:vm'),fs=require('node:fs');
(async()=>{
const protocol=process.argv[1],missing=process.argv[2]==='true';
const current=process.argv[4],target=process.argv[5];
const calls=[]; const prefix=protocol==='file:'?'file:///bundle/':'https://example.test/x/y/';
const select={dataset:{current},value:current,hidden:false,options:[{value:'master'},{value:'v1'}],
 replaceChildren(...options){this.options=options;}};
const location={protocol,pathname:new URL(prefix+'docs/'+current+'/deep/chapter.html').pathname,
 href:prefix+'docs/'+current+'/deep/chapter.html'};
const context={URL,document:{
 currentScript:{src:prefix+'docs/'+current+'/theme/versions.js'},getElementById:()=>select},
 location,Option:function(label,value){this.value=value;},fetch:async(url,opts)=>{
 calls.push([String(url),opts?.method||'GET']);
 if(protocol==='file:')throw Error('offline fetch forbidden');
 return opts?{ok:!missing}:{ok:true,json:async()=>[
  {label:'master',version:'master'},{label:'v1',version:'v1'}]};}};
vm.runInNewContext(fs.readFileSync(process.argv[3],'utf8'),context);
await new Promise(r=>setTimeout(r,10));
if(typeof select.onchange!=='function')throw Error('navigation disappeared');
select.value=target;await select.onchange();
console.log(JSON.stringify({calls,href:location.href,hidden:select.hidden}));
})().catch(e=>{console.error(e);process.exit(1)});
"""
        return json.loads(
            subprocess.check_output(
                [
                    "node",
                    "-e",
                    harness,
                    protocol,
                    str(missing).lower(),
                    str(script),
                    current,
                    target,
                ],
                text=True,
            )
        )

    def test_http_prefix_fetch_and_preserved_chapter(self):
        result = self.run_js("https:")
        self.assertEqual(
            result["calls"][0][0], "https://example.test/x/y/docs/versions.json"
        )
        self.assertEqual(
            result["href"], "https://example.test/x/y/docs/v1/deep/chapter.html"
        )
        self.assertEqual(result["calls"][1][1], "HEAD")

    def test_missing_http_chapter_uses_explicit_index(self):
        self.assertEqual(
            self.run_js("https:", True)["href"],
            "https://example.test/x/y/docs/v1/index.html",
        )

    def test_nested_tag_fetches_docs_json_from_actual_depth(self):
        result = self.run_js("https:", current="v1/nested", target="master")
        self.assertEqual(
            result["calls"][0][0], "https://example.test/x/y/docs/versions.json"
        )
        self.assertEqual(
            result["href"], "https://example.test/x/y/docs/master/deep/chapter.html"
        )

    def test_file_nested_target_has_real_path_components(self):
        result = self.run_js("file:", target="v1/nested")
        self.assertEqual(result["calls"], [])
        self.assertEqual(result["href"], "file:///bundle/docs/v1/nested/index.html")

    def test_file_navigation_never_fetches_or_disappears(self):
        result = self.run_js("file:")
        self.assertEqual(result["calls"], [])
        self.assertFalse(result["hidden"])
        self.assertEqual(result["href"], "file:///bundle/docs/v1/index.html")


class PortableOutput(unittest.TestCase):
    def broken_fixture(self, html):
        import check_portable

        with tempfile.TemporaryDirectory(prefix="portable-crawl-killer-") as temp:
            root = Path(temp)
            (root / "docs").mkdir()
            (root / "index.html").write_text(html)
            return check_portable.check(root)["broken"]

    def test_crawler_rejects_root_absolute_regression(self):
        errors = self.broken_fixture('<a href="/index.html">home</a>')
        self.assertTrue(any("root absolute" in row[2] for row in errors))
        self.assertTrue(any("escaped HTTP prefix" in row[2] for row in errors))

    def test_crawler_rejects_missing_asset_regression(self):
        errors = self.broken_fixture('<img src="missing.png">')
        self.assertTrue(
            any("missing or escaped file target" in row[2] for row in errors)
        )
        self.assertTrue(any("404" in row[2] for row in errors))

    @unittest.skipUnless(
        os.environ.get("SITE_PORTABLE_TOOLS"),
        "set SITE_PORTABLE_TOOLS for real pinned builds",
    )
    def test_real_two_version_build_http_prefix_and_file_targets(self):
        import shutil
        import check_portable
        import test_build

        build = test_build.builder()
        with tempfile.TemporaryDirectory(prefix="site-portable-build-") as temp:
            repo = Path(temp)
            source = Path(__file__).resolve().parent.parent
            shutil.copytree(
                source / "site",
                repo / "site",
                ignore=shutil.ignore_patterns(
                    "public", "__pycache__", "build-report.json"
                ),
            )
            shutil.copytree(source / "docs/howto", repo / "docs/howto")
            shutil.copy2(source / "docs/FAQ.md", repo / "docs/FAQ.md")

            def git(*args):
                return subprocess.run(
                    ["git", *args],
                    cwd=repo,
                    check=True,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.PIPE,
                )

            git("init", "-q")
            git(
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.com",
                "add",
                "site",
                "docs",
            )
            git(
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.com",
                "commit",
                "-qm",
                "Fixture",
            )
            git("tag", "v1/nested")
            report = build.build(
                repo,
                build.tools(Path(os.environ["SITE_PORTABLE_TOOLS"])),
                [],
                portable=True,
            )
            self.assertTrue(report["portable"])
            self.assertEqual(report["broken_links"], [])
            self.assertEqual(report["versions"], ["master", "v1/nested"])
            crawl = check_portable.check(repo / "site/public")
            self.assertGreater(crawl["local_references"], 500)
            self.assertEqual(crawl["broken"], [])
            for version in report["versions"]:
                text = (repo / "site/public/docs" / version / "index.html").read_text()
                self.assertIn('<option value="master">', text)
                self.assertIn('<option value="v1/nested">', text)

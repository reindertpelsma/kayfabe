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
        self.mod = importlib.import_module('portable')
        self.temp = tempfile.TemporaryDirectory(dir='/data', prefix='site-portable-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in ['index.html', 'about/index.html', 'docs/master/index.html',
                     'docs/master/chapter.html', 'images/one.png', 'images/two.png']:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('fixture')

    def rewrite(self, text, page='docs/master/chapter.html'):
        return self.mod.html_urls(text, self.root / page, self.root)

    def test_depth_index_queries_and_encoded_paths(self):
        self.assertEqual(self.rewrite('<a href="/about/?x=1&amp;y=2#part">'),
                         '<a href="../../about/index.html?x=1&amp;y=2#part">')
        self.assertEqual(self.rewrite("<a href='https://kayfabe.dev/docs/master/'>"),
                         "<a href='index.html'>")
        self.assertEqual(self.rewrite('<a href="/">'), '<a href="../../index.html">')

    def test_assets_srcset_forms_refresh_and_metadata(self):
        source = ('<img src="/images/one.png" srcset="/images/one.png 1x, /images/two.png 2x">'
                  '<form action="/docs/master/"></form>'
                  '<meta http-equiv="refresh" content="0; url=/about/">'
                  '<meta property="og:url" content="https://kayfabe.dev/about/">')
        result = self.rewrite(source)
        self.assertIn('src="../../images/one.png"', result)
        self.assertIn('srcset="../../images/one.png 1x, ../../images/two.png 2x"', result)
        self.assertIn('action="index.html"', result)
        self.assertIn('content="0; url=../../about/index.html"', result)
        self.assertIn('content="../../about/index.html"', result)

    def test_only_attribute_value_bytes_change(self):
        source = ('<!DOCTYPE html>\n<!-- href="/about/" -->\n'
                  '<script>const html = \'<a href="/about/">\';</script>\n'
                  '<pre>ordinary &lt;a href="/about/"&gt; text</pre>\n'
                  '<A  HREF = "/about/" data-note="a > b">É &amp; &#169;</A>')
        expected = source.replace('HREF = "/about/"', 'HREF = "../../about/index.html"')
        self.assertEqual(self.rewrite(source), expected)

    def test_external_fragments_data_and_non_url_content_unchanged(self):
        source = ('<a href="https://elsewhere.example/about/">x</a><a href="#part">x</a>'
                  '<a href="mailto:a@example.com">x</a>'
                  '<img srcset="data:image/png;base64,AAAA 1x, /images/two.png 2x">'
                  '<meta name="description" content="A /about/ description">')
        self.assertEqual(self.rewrite(source), source.replace('/images/two.png', '../../images/two.png'))

    def test_css_urls_preserve_comments_and_strings(self):
        css = '/* url(/bad) */ .a{content:"url(/not-a-url)";background:url("/images/one.png")} '
        self.assertEqual(self.mod.css_urls(css, self.root / 'style.css', self.root),
                         css.replace('url("/images/one.png")', 'url("images/one.png")'))
        self.assertEqual(self.mod.css_urls('x{background:url(data:image/png;base64,AA)}',
                                          self.root / 'style.css', self.root),
                         'x{background:url(data:image/png;base64,AA)}')


class PortableNavigation(unittest.TestCase):
    def run_js(self, protocol, missing=False):
        script = Path(__file__).with_name('book-theme') / 'versions.js'
        harness = r'''
const vm=require('node:vm'),fs=require('node:fs');
(async()=>{
const protocol=process.argv[1],missing=process.argv[2]==='true';
const calls=[]; const prefix=protocol==='file:'?'file:///bundle/':'https://example.test/x/y/';
const select={dataset:{current:'master'},value:'master',hidden:false,options:[{value:'master'},{value:'v1'}],
 replaceChildren(...options){this.options=options;}};
const location={protocol,pathname:new URL(prefix+'docs/master/deep/chapter.html').pathname,
 href:prefix+'docs/master/deep/chapter.html'};
const context={URL,document:{currentScript:{src:prefix+'docs/master/theme/versions.js'},getElementById:()=>select},
 location,Option:function(label,value){this.value=value;},fetch:async(url,opts)=>{
 calls.push([String(url),opts?.method||'GET']);
 if(protocol==='file:')throw Error('offline fetch forbidden');
 return opts?{ok:!missing}:{ok:true,json:async()=>[{label:'master',version:'master'},{label:'v1',version:'v1'}]};}};
vm.runInNewContext(fs.readFileSync(process.argv[3],'utf8'),context);
await new Promise(r=>setTimeout(r,10));
if(typeof select.onchange!=='function')throw Error('navigation disappeared');
select.value='v1';await select.onchange();
console.log(JSON.stringify({calls,href:location.href,hidden:select.hidden}));
})().catch(e=>{console.error(e);process.exit(1)});
'''
        return json.loads(subprocess.check_output(['node', '-e', harness, protocol,
                                                  str(missing).lower(), str(script)], text=True))

    def test_http_prefix_fetch_and_preserved_chapter(self):
        result = self.run_js('https:')
        self.assertEqual(result['calls'][0][0], 'https://example.test/x/y/docs/versions.json')
        self.assertEqual(result['href'], 'https://example.test/x/y/docs/v1/deep/chapter.html')
        self.assertEqual(result['calls'][1][1], 'HEAD')

    def test_missing_http_chapter_uses_explicit_index(self):
        self.assertEqual(self.run_js('https:', True)['href'],
                         'https://example.test/x/y/docs/v1/index.html')

    def test_file_navigation_never_fetches_or_disappears(self):
        result = self.run_js('file:')
        self.assertEqual(result['calls'], [])
        self.assertFalse(result['hidden'])
        self.assertEqual(result['href'], 'file:///bundle/docs/v1/index.html')

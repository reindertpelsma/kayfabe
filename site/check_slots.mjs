import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { JSDOM } from 'jsdom';

const slot = '<!-- kf-slot-a -->';
const dummy = '<script data-kf-dummy="1"></script>';
export function validate(html, candidate) {
  if (!candidate) {
    for (const word of ['kf-slot-', 'cloudflareinsights', 'Web Analytics', 'cross-site tracking']) {
      assert.equal(html.toLowerCase().includes(word.toLowerCase()), false, word);
    }
  }
  const before = new JSDOM(html);
  const document = before.window.document;
  assert.equal(document.querySelectorAll('footer#kf-footer').length, 1, 'stable footer');
  assert.equal(document.querySelectorAll('#kf-footer').length, 1, 'unique footer id');
  if (candidate) {
    const walker = document.createTreeWalker(document, before.window.NodeFilter.SHOW_COMMENT);
    const slots = [];
    while (walker.nextNode()) {
      if (walker.currentNode.data.trim() === 'kf-slot-a') slots.push(walker.currentNode);
    }
    assert.equal(slots.length, 1, 'one slot');
    assert.equal(slots[0].parentNode, document.head, 'slot belongs directly to head');
    assert.equal(html.split(slot).length, 2, 'one literal slot');
    slots[0].remove();
    const after = new JSDOM(html.replace(slot, dummy));
    assert.equal(after.window.document.querySelectorAll('script[data-kf-dummy]').length, 1);
    after.window.document.querySelector('script[data-kf-dummy]').remove();
    assert.equal(
      after.window.document.documentElement.outerHTML,
      document.documentElement.outerHTML,
    );
    after.window.close();
  }
  before.window.close();
}

export function privacyScript() {
  const workflow = readFileSync(new URL('../.github/workflows/site.yml', import.meta.url), 'utf8');
  const literal = workflow.match(/privacy = b"""([\s\S]*?)"""/)?.[1];
  assert.ok(literal?.includes('<script>') && literal.endsWith('</script>'));
  return literal.slice(literal.indexOf('<script>') + '<script>'.length, -'</script>'.length);
}

export function verifyPrivacy(beacons = [true, false]) {
  const script = privacyScript();
  for (const beacon of beacons) {
    const dom = new JSDOM('<html><head></head><body></body></html>', {
      runScripts: 'outside-only',
    });
    const { document } = dom.window;
    if (beacon) {
      const element = document.createElement('script');
      element.setAttribute('data-cf-beacon', '{"token":"synthetic"}');
      document.head.append(element);
    }
    dom.window.eval(script);
    const footer = document.createElement('footer');
    footer.id = 'kf-footer';
    document.body.append(footer);
    document.dispatchEvent(new dom.window.Event('DOMContentLoaded'));
    assert.equal(document.querySelectorAll('#kf-privacy').length, beacon ? 1 : 0);
    dom.window.eval(script);
    document.dispatchEvent(new dom.window.Event('DOMContentLoaded'));
    assert.equal(document.querySelectorAll('#kf-privacy').length, beacon ? 1 : 0);
    if (beacon) {
      assert.match(
        footer.textContent,
        /Cloudflare Web Analytics uses no cookies or cross-site tracking/,
      );
      assert.ok(footer.querySelector('a[href^="https://developers.cloudflare.com/"]'));
    }
    dom.window.close();
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = process.argv[2];
  const candidate = process.argv[3] === 'true';
  let pages = 0;
  const walk = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) walk(path);
      else {
        const text = readFileSync(path).toString('utf8').toLowerCase();
        for (const word of ['cloudflareinsights', 'web analytics', 'cross-site tracking']) {
          assert.equal(text.includes(word), false, path + ': ' + word);
        }
        if (!candidate) assert.equal(text.includes('kf-slot-'), false, path);
      }
      if (!entry.isDirectory() && entry.name.endsWith('.html')) {
        validate(readFileSync(path, 'utf8'), candidate);
        pages++;
      }
    }
  };
  walk(root);
  verifyPrivacy();
  console.log(`DOM checks: ${pages} pages; candidate=${candidate}; privacy checks passed`);
}

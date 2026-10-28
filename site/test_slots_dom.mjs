import test from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import { validate, verifyPrivacy } from './check_slots.mjs';
const page =
  '<html><head><!-- kf-slot-a --></head><body><footer id="kf-footer"></footer></body></html>';
test('candidate head slot and dummy replacement preserve the existing DOM', () => {
  validate(page, true);
});
test('body comment cannot masquerade as head slot', () => {
  assert.throws(() =>
    validate(
      page.replace('<head><!-- kf-slot-a --></head>', '<head></head><!-- kf-slot-a -->'),
      true,
    ),
  );
});
test('ordinary output rejects slots and branded privacy content', () => {
  assert.throws(() => validate(page, false));
  assert.throws(() => validate(page.replace('<!-- kf-slot-a -->', 'Web Analytics'), false));
  validate(page.replace('<!-- kf-slot-a -->', ''), false);
});
test('workflow privacy JS waits, requires the beacon, and stays single', () => {
  verifyPrivacy();
});

test('footer comment cannot masquerade as the head slot', () => {
  assert.throws(() =>
    validate(
      page
        .replace('<!-- kf-slot-a -->', '')
        .replace('<footer id="kf-footer">', '<footer id="kf-footer"><!-- kf-slot-a -->'),
      true,
    ),
  );
});
test('privacy still waits for a late footer when the beacon exists', () => {
  verifyPrivacy([true]);
});
test('privacy remains absent without a beacon across repeated execution', () => {
  verifyPrivacy([false]);
});

test('dummy replacement also preserves docs metadata and escaped code', () => {
  validate(
    page
      .replace('<head>', '<head><meta charset="utf-8">')
      .replace('</body>', '<pre>&lt;/head&gt;</pre></body>'),
    true,
  );
});

test('a duplicate non-footer id cannot divert the privacy note', () => {
  assert.throws(() => validate(page.replace('<body>', '<body><h1 id="kf-footer">x</h1>'), true));
});

const syntheticBeacon =
  '<script type="module" ' +
  'src="https://static.cloudflareinsights.com/beacon.min.js" ' +
  `data-cf-beacon='{"token":"fixture-token"}'></script>`;
for (const [name, prefix, selector] of [
  [
    'abrupt empty comment',
    '<!--><img src="https://wrong.invalid/x">-->',
    'img[src="https://wrong.invalid/x"]',
  ],
  [
    'abrupt dash comment',
    '<!---><script src="https://wrong.invalid/x"></script>-->',
    'script[src="https://wrong.invalid/x"]',
  ],
]) {
  test('HTML5 exposes the synthetic prohibited node after ' + name, () => {
    const dom = new JSDOM(
      '<html><head>' + prefix + syntheticBeacon + '</head><body></body></html>',
    );
    assert.equal(dom.window.document.querySelectorAll(selector).length, 1);
    assert.equal(dom.window.document.querySelectorAll('script[data-cf-beacon]').length, 1);
    dom.window.close();
  });
}

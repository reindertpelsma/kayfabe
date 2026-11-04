// Row-tag reference (design section 8, proposal). FALSIFIER: a tag verifies after any tagged
// field changes, under another key, with a malformed tag, or a field can smuggle a separator.
import test from 'node:test';
import assert from 'node:assert/strict';
import { TAG_FIELDS, canonicalRow, rowTag, verifyRowTag } from '../shared/row-tag.js';

const KEY = 'synthetic-row-tag-key-for-tests-only-0123456789';
const ROW = {
  id: 'syn-1-ad104-linux-gate',
  commit_sha: 'a'.repeat(40),
  boundary_hash: 'b'.repeat(64),
  requirement_key: 'linux-gate',
  platform_key: 'p-0123456789abcdef01234567',
  suite: 'linux-gate',
  verdict: 'PASS',
  source: 'reproduced',
  tier: 'middle',
  finished_at: '2026-11-04T00:00:00Z',
};

test('a tag verifies for its row and key only', async () => {
  const tag = await rowTag(KEY, ROW);
  assert.match(tag, /^[A-Za-z0-9_-]{43}$/);
  assert.equal(await verifyRowTag(KEY, ROW, tag), true);
  assert.equal(await verifyRowTag(KEY + 'x', ROW, tag), false);
  for (const field of TAG_FIELDS) {
    const changed = { ...ROW, [field]: ROW[field] + 'x' };
    assert.equal(await verifyRowTag(KEY, changed, tag), false, field);
  }
  const flipped = (tag[0] === 'A' ? 'B' : 'A') + tag.slice(1);
  assert.equal(await verifyRowTag(KEY, ROW, flipped), false);
  for (const bad of [null, '', tag + '=', tag.slice(1), 'synthetic-tag-0123456789abcdef'])
    assert.equal(await verifyRowTag(KEY, ROW, bad), false);
});

test('fields are complete, non-empty and cannot carry a separator', async () => {
  assert.equal(canonicalRow(ROW).split('\n').length, TAG_FIELDS.length + 1);
  for (const field of TAG_FIELDS) {
    assert.throws(() => canonicalRow({ ...ROW, [field]: '' }));
    assert.throws(() => canonicalRow({ ...ROW, [field]: 'a\nb' }));
    const { [field]: _, ...missing } = ROW;
    assert.throws(() => canonicalRow(missing));
  }
  await assert.rejects(rowTag('short', ROW));
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// Review of pass 3 (2026-10-24), F1 and T5.
// F1: an ambiguous /rent reply (5xx, client timeout, connection error, a garbled gateway page)
// followed by BUSY is not a definite BUSY: the Worker may have created a box for this run, and
// its in-isolate busy flag answers the runner's own retry while that create is running. The
// runner keeps asking, and on the give-up path sends a best-effort POST /stop/<run>. A clean
// BUSY (nothing was ever admitted) still stops waiting before the budget is gone, no stop.
// T5: the BUSY flag is reset before every /rent send.
// FALSIFIER: an ambiguous reply followed by BUSY ends the run early, or ends it without a
// /stop; a clean BUSY waits out the budget or sends a /stop.
const fixture = new URL('edits4_runner_fixture.py', import.meta.url).pathname;
const py = (args) =>
  spawnSync('python3', ['-I', fixture, ...args], { encoding: 'utf8', timeout: 20000 });
for (const name of [
  'clean-busy',
  'ambiguous-503-then-busy',
  'ambiguous-timeout-then-busy',
  'ambiguous-html-then-busy',
])
  test('F1: runner ' + name, () => {
    const p = py([name]);
    assert.equal(p.status, 0, p.stderr + p.stdout);
    assert.match(p.stdout, new RegExp('^ok ' + name));
  });

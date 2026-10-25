import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// 2026-10-25 (MEASURED live, twice): a real Worker answered POST /rent with HTTP 200, the CI
// runner sent /stop at once and ended with no explanation, because validate_status() raised a
// bare Invalid (about 40 raise sites, none carrying information). PR #117 already prints
// "Worker reply not accepted: HTTP <code> ..." for a bare refusal; a 200 reply that fails the
// runner's own check now prints ONE line naming the line number of the raise in runner.py.
// FALSIFIER: a 200 reply failing validate_status ends without exactly one
// "runner: Worker reply rejected by runner.py check at line <N>" line, N is not the line of a
// `raise Invalid()` in runner.py, two different bad fields give the same N, the line appears
// after the /stop (or on a good reply), carries any Worker-provided text, or the run's state,
// verdict or /stop behaviour differs from before.
const MARKER = 'SeNtInEl9f3a7c1d';
const LINE = /^runner: Worker reply rejected by runner\.py check at line (\d+)$/;
const run = (args) => {
  const p = spawnSync(
    'python3',
    ['-I', new URL('runner_reject_fixture.py', import.meta.url).pathname, JSON.stringify(args)],
    { encoding: 'utf8', timeout: 30000 },
  );
  assert.equal(p.status, 0, p.stderr);
  return JSON.parse(p.stdout);
};
const lines = (r) =>
  r.shown.split('\n').filter((l) => l.startsWith('runner: Worker reply rejected'));
const lineNumber = (r) => {
  const found = lines(r);
  assert.equal(found.length, 1, r.shown);
  const match = LINE.exec(found[0]);
  assert.ok(match, found[0]);
  return Number(match[1]);
};
const bad = {
  driver: { driver_version: 'garbage' },
  country: { country: 'xx' },
  deadlines: { soft_deadline_at: 10801000 },
};

for (const [name, mutate] of Object.entries(bad)) {
  test('200 /rent reply with a bad ' + name + ': exactly one rejected-line, integer N', () => {
    const r = run({ mutate });
    assert.equal(typeof lineNumber(r), 'number');
  });
  test('200 /rent reply with a bad ' + name + ': N is a raise Invalid() in runner.py', () => {
    const r = run({ mutate });
    const n = lineNumber(r);
    assert.ok(n > 1 && n <= r.source.length, String(n));
    assert.match(r.source[n - 1], /^\s*raise Invalid\(\)\s*$/, r.source[n - 1]);
  });
  test('200 /rent reply with a bad ' + name + ': same end state, one /stop, line first', () => {
    const r = run({ mutate });
    assert.deepEqual(r.result, { state: 'UNAVAILABLE', verdict: 'UNAVAILABLE', verified: 0 });
    assert.deepEqual(r.calls.filter((c) => c.startsWith('/stop/')).length, 1);
    assert.deepEqual(r.calls.filter((c) => c === '/rent').length, 1);
    // The line is already in the job log when /stop is sent, so it survives a run that ends there.
    assert.equal(r.stop_marks.length, 1);
    assert.ok(r.shown.slice(0, r.stop_marks[0]).includes('Worker reply rejected'), r.shown);
  });
  test('200 /rent reply with a bad ' + name + ': no Worker text in the output', () => {
    const value = Object.fromEntries(
      Object.entries(mutate).map(([k, v]) => [k, typeof v === 'string' ? v + MARKER : v]),
    );
    const r = run({
      mutate: { ...value, marker_field: MARKER, driver_version: 'garbage' + MARKER },
    });
    assert.ok(!r.shown.includes(MARKER), r.shown);
    assert.equal(lines(r).length, 1);
  });
}

test('two different bad fields give two different line numbers', () => {
  const numbers = Object.values(bad).map((mutate) => lineNumber(run({ mutate })));
  assert.equal(new Set(numbers).size, 3, numbers.join());
});

test('the same bad field gives the same line number every time', () => {
  assert.equal(lineNumber(run({ mutate: bad.country })), lineNumber(run({ mutate: bad.country })));
});

test('a bad later /status reply prints one line too, with a raise line', () => {
  const r = run({ where: 'status', mutate: bad.driver });
  const n = lineNumber(r);
  assert.match(r.source[n - 1], /^\s*raise Invalid\(\)\s*$/);
  assert.equal(r.calls.filter((c) => c.startsWith('/stop/')).length, 1);
});

test('a good reply prints no such line', () => {
  const r = run({});
  assert.equal(lines(r).length, 0, r.shown);
  assert.deepEqual(r.result, { state: 'GRACE', verdict: 'PASS', verified: 1 });
});

test('an unrelated extra field with a marker is rejected without echoing it', () => {
  const r = run({ mutate: { [MARKER]: MARKER } });
  assert.equal(lines(r).length, 1);
  assert.ok(!r.shown.includes(MARKER), r.shown);
});

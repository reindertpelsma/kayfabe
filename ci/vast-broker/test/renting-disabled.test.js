import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// 2026-10-25 (MEASURED live, 16:10 UTC): with RENTING_ENABLED=false the Worker answers
// POST /rent with HTTP 503 {state: RENTING_DISABLED, verdict: UNAVAILABLE}. The runner treated
// every 503 as a transient, possibly-ambiguous infrastructure error and re-POSTed /rent every
// 20 s for the whole 200-minute budget (127 POSTs in 5 minutes at the Worker; a gpu job stayed
// in_progress for 35+ minutes). RENTING_DISABLED is a deliberate, configured refusal: the Worker
// did not touch Vast, so it is terminal on /rent, ambiguous for nothing, and owes no cleanup.
// FALSIFIER: after a 503 RENTING_DISABLED the runner sends a second /rent, sleeps, sends a
// /stop, marks the run ambiguous, reports a state other than RENTING_DISABLED, exits 0, or
// prints no one-line explanation; OR a real outage (503 UNAVAILABLE, 500, 502, 504) stops
// being retried; OR a 503 on /status or /logs of an admitted run stops being retried.
const fixture = new URL('renting_disabled_fixture.py', import.meta.url).pathname;
const py = (name) =>
  spawnSync('python3', ['-I', fixture, name], { encoding: 'utf8', timeout: 30000 });
const ok = (name, title) =>
  test(title, () => {
    const p = py(name);
    assert.equal(p.status, 0, p.stderr + p.stdout);
    assert.match(p.stdout, new RegExp('^ok ' + name));
  });
ok('one-post', 'RENTING_DISABLED on /rent: exactly one POST, no sleep, not ambiguous, no stop');
ok('one-line', 'RENTING_DISABLED on /rent: one static diagnostic line');
ok('retry-read', 'RENTING_DISABLED on /rent: retry_read returns at once with clean flags');
ok('main', 'RENTING_DISABLED on /rent: main() exits 1 with state RENTING_DISABLED, one line');
ok('unavailable-503-retries', '503 UNAVAILABLE on /rent is still retried until admission');
ok('unavailable-503-gives-up', '503 UNAVAILABLE on /rent still ends ambiguous with a /stop');
for (const status of ['500', '502', '504'])
  ok('http' + status, status + ' on /rent is still retried');
ok('status-503', '503 RENTING_DISABLED on /status is still transient');
ok('logs-503', '503 RENTING_DISABLED on /logs is still transient');
ok('admitted-status-503', 'an admitted run rides out a 503 RENTING_DISABLED on /status');

import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, env, nonce } from './helpers.js';
// STATUS: LIVE, 2026-11-06 (GPU-free). Vanish hunt 2026-11-06 (traces/vanish_hunt_20261106): Vast's
// per-instance actual_status flaps (`offline`, `created`, `unknown`) while the instance and the box
// stay up, and the Worker closed (destroyed) a live rental on ONE such reading.
// FALSIFIER: a live box (one that keeps POSTing, or whose status returns to running) is closed on
// a single non-running reading, on two, on three readings that span less than 120 s, or on
// readings that a box POST interrupted; or a really gone instance is NOT closed within the window;
// or Vast's verified absence is delayed; or a close leaves no `close reason=` Worker line.
const log = (seq = 1, text = 'hello') => ({ seq, kind: 'log', text });
const rent = async (overrides) => {
  const h = await setup(overrides);
  assert.equal((await h.call()).status, 200);
  return h;
};
const record = (h) => h.storage.get('record:123-1');
const instance = (h) => h.cloud.instances.get('900');
const poll = async (h) => (await h.call('/status/123-1')).json();
const at = async (h, seconds, fn = poll) => {
  h.time.value += seconds * 1000;
  return fn(h);
};
const lines = async (h) =>
  ((await record(h)).log ?? '').split('\n').filter((l) => l.startsWith('[Worker] '));
const closeLines = async (h) => (await lines(h)).filter((l) => l.startsWith('[Worker] close '));
// A box that has reported once is in TESTING, as in the lost runs.
const testing = async (overrides) => {
  const h = await rent(overrides);
  assert.equal((await h.report(log())).status, 200);
  assert.equal((await poll(h)).state, 'TESTING');
  return h;
};

for (const status of ['offline', 'created', 'exited', 'stopped', 'loading', 'weird-STATE!', null]) {
  test(
    'TESTING: one non-running reading (' + status + ') does not close, a running one clears it',
    async () => {
      const h = await testing();
      instance(h).actual_status = status;
      await at(h, 60);
      assert.equal((await record(h)).closed, null);
      assert.deepEqual(h.cloud.deletes, []);
      assert.equal((await record(h)).providerBad.count, 1);
      assert.equal((await record(h)).providerFlaps, 1);
      instance(h).actual_status = 'running';
      await at(h, 60);
      assert.equal((await record(h)).providerBad, undefined);
      assert.equal((await record(h)).closed, null);
      assert.deepEqual(h.cloud.deletes, []);
    },
  );
}
test('TESTING: two non-running readings (cron cadence, 60 s apart) do not close', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  await at(h, 60, (x) => x.service.sweep());
  await at(h, 60, (x) => x.service.sweep());
  assert.equal((await record(h)).providerBad.count, 2);
  assert.equal((await record(h)).closed, null);
  assert.deepEqual(h.cloud.deletes, []);
  instance(h).actual_status = 'running';
  await at(h, 60, (x) => x.service.sweep());
  assert.deepEqual(h.cloud.deletes, []);
  assert.equal((await record(h)).providerBad, undefined);
});
test('TESTING: three readings 60 s apart (120 s) with no box post close the rental as ENDED', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  instance(h).status_msg = 'Required resources are currently unavailable';
  await at(h, 60, (x) => x.service.sweep());
  await at(h, 60, (x) => x.service.sweep());
  assert.deepEqual(h.cloud.deletes, []);
  await at(h, 60, (x) => x.service.sweep());
  const r = await record(h);
  assert.equal(r.closed, 'ENDED');
  assert.deepEqual(h.cloud.deletes, ['900']);
  const close = await closeLines(h);
  assert.equal(close.length, 1);
  assert.match(
    close[0],
    /^\[Worker\] close reason=ENDED provider=offline msg=Required resources are currently unavailable$/,
  );
  assert.ok(
    (await lines(h)).some((l) => /provider gone: 3 readings over 120 s, no box post/.test(l)),
  );
});
test('TESTING: readings every second count once per 20 s and never close before 120 s', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  // First reading at +1 s, counted readings at 1, 21, ..., 101 s: 120 s are not yet spanned.
  for (let s = 0; s < 120; s++) await at(h, 1);
  assert.equal((await record(h)).closed, null);
  assert.deepEqual(h.cloud.deletes, []);
  await at(h, 1); // +121 s: the 7th counted reading, 120 s after the first
  assert.equal((await record(h)).closed, 'ENDED');
  assert.deepEqual(h.cloud.deletes, ['900']);
});
test('TESTING: readings 25 s apart (four readings over 75 s) do not close; the 120 s mark does', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  for (let i = 0; i < 4; i++) await at(h, 25);
  assert.equal((await record(h)).providerBad.count, 4);
  assert.equal((await record(h)).closed, null);
  assert.deepEqual(h.cloud.deletes, []);
  await at(h, 25);
  await at(h, 25);
  assert.equal((await record(h)).closed, 'ENDED'); // 150 s after the first reading
});
test('TESTING: two readings are never enough, however far apart', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  await at(h, 5);
  await at(h, 300);
  assert.equal((await record(h)).providerBad.count, 2);
  assert.equal((await record(h)).closed, null);
  assert.deepEqual(h.cloud.deletes, []);
});
test('TESTING: three readings 5 s apart, then one 130 s later, count as two', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  for (let i = 0; i < 3; i++) await at(h, 5);
  await at(h, 130);
  assert.equal((await record(h)).providerBad.count, 2);
  assert.equal((await record(h)).closed, null);
});
test('TESTING: a burst of readings inside 20 s counts as one reading', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  for (let s = 0; s < 10; s++) await at(h, 1);
  assert.equal((await record(h)).providerBad.count, 1);
  // Two more readings far later: only 2 counted so far (first burst + this one), then the third.
  await at(h, 200);
  assert.equal((await record(h)).providerBad.count, 2);
  assert.equal((await record(h)).closed, null);
  await at(h, 60);
  assert.equal((await record(h)).closed, 'ENDED');
});
test('TESTING: a box POST resets the window; the box is then closed only 3 readings and 120 s after it', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  await at(h, 60, (x) => x.service.sweep());
  await at(h, 60, (x) => x.service.sweep());
  assert.equal((await record(h)).providerBad.count, 2);
  // The box posts while the provider still says offline: accepted, anomaly counted, window cleared.
  const posted = await at(h, 30, (x) => x.report(log(2, 'still alive')));
  assert.equal(posted.status, 200);
  const afterPost = await record(h);
  assert.equal(afterPost.providerBad, undefined);
  assert.equal(afterPost.providerAnomalies, 1);
  assert.equal(afterPost.closed, null);
  await at(h, 30, (x) => x.service.sweep()); // first reading of the new window
  await at(h, 60, (x) => x.service.sweep());
  assert.equal((await record(h)).closed, null);
  assert.deepEqual(h.cloud.deletes, []);
  await at(h, 60, (x) => x.service.sweep());
  assert.equal((await record(h)).closed, 'ENDED');
  assert.deepEqual(h.cloud.deletes, ['900']);
});
test('TESTING: a box that keeps posting is never closed by the provider status', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  for (let i = 0; i < 12; i++) {
    await at(h, 60, (x) => x.service.sweep());
    assert.equal((await at(h, 30, (x) => x.report(log(2 + i, 'beat')))).status, 200);
  }
  assert.equal((await record(h)).closed, null);
  assert.deepEqual(h.cloud.deletes, []);
});
test('a POST newer than the first bad reading restarts the window even if update() sees it first', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  await at(h, 60, (x) => x.service.sweep());
  const r = await record(h);
  r.lastPostAt = h.time.value + 1000; // a post after the window opened that did not clear it
  await h.storage.put('record:123-1', r);
  await at(h, 61, (x) => x.service.sweep());
  assert.equal((await record(h)).providerBad.count, 1);
  assert.equal((await record(h)).closed, null);
});
test('a POST clears a pending window even when the provider reads running at the POST', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  await at(h, 60, (x) => x.service.sweep());
  assert.equal((await record(h)).providerBad.count, 1);
  instance(h).actual_status = 'running';
  assert.equal((await at(h, 10, (x) => x.report(log(2, 'x')))).status, 200);
  assert.equal((await record(h)).providerBad, undefined);
});
test('update(): a POST recorded after the second reading stops the third from closing', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  await at(h, 60, (x) => x.service.sweep());
  await at(h, 60, (x) => x.service.sweep());
  const r = await record(h);
  r.lastPostAt = h.time.value + 30000;
  await h.storage.put('record:123-1', r);
  await at(h, 60, (x) => x.service.sweep());
  assert.equal((await record(h)).closed, null);
  assert.equal((await record(h)).providerBad.count, 1);
  assert.deepEqual(h.cloud.deletes, []);
});
test('report path: a POST while the provider reads non-running is accepted (was 401 + destroy)', async () => {
  const h = await testing();
  instance(h).actual_status = 'created';
  const r = await at(h, 10, (x) => x.report(log(2, 'x')));
  assert.equal(r.status, 200);
  assert.deepEqual(h.cloud.deletes, []);
  assert.equal((await record(h)).closed, null);
  assert.equal((await record(h)).providerAnomalies, 1);
  assert.equal((await record(h)).lastSeq, 2);
  assert.ok(
    (await lines(h)).some((l) => /provider=created phase=TESTING .*\(box post accepted\)/.test(l)),
  );
});
test('report path: Vast absence still closes at once, with a close line', async () => {
  const h = await testing();
  h.cloud.instances.delete('900');
  const r = await at(h, 10, (x) => x.report(log(2, 'x')));
  assert.equal(r.status, 401);
  assert.equal((await record(h)).closed, 'UNAVAILABLE');
  assert.match(
    (await closeLines(h))[0],
    /^\[Worker\] close reason=UNAVAILABLE provider=absent msg=-$/,
  );
});
test('provider absent (Vast 404) closes at once on one reading and logs it', async () => {
  const h = await testing();
  h.cloud.instances.delete('900');
  await at(h, 5);
  const r = await record(h);
  assert.equal(r.closed, 'GONE');
  assert.equal(r.verdict, 'UNAVAILABLE');
  assert.ok((await lines(h)).includes('[Worker] provider_absent'));
  assert.match((await closeLines(h))[0], /^\[Worker\] close reason=GONE provider=absent msg=-$/);
});
test('provider absent during BOOTING closes at once too, with a close line', async () => {
  const h = await rent();
  h.cloud.instances.delete('900');
  await at(h, 5);
  assert.equal((await record(h)).closed, 'GONE');
  assert.match((await closeLines(h))[0], /^\[Worker\] close reason=GONE provider=absent msg=-$/);
});
test('a first POST while the instance is absent (BOOTING) closes at once', async () => {
  const h = await rent();
  h.cloud.instances.delete('900');
  assert.equal((await at(h, 5, (x) => x.report(log(1)))).status, 401);
  assert.equal((await record(h)).closed, 'UNAVAILABLE');
});

// BOOTING
test('BOOTING: offline is treated like created (no close, no counting) until the boot timeout', async () => {
  const h = await rent();
  instance(h).actual_status = 'offline';
  for (let s = 0; s < 9; s++) await at(h, 60, (x) => x.service.sweep());
  const r = await record(h);
  assert.equal(r.closed, null);
  assert.equal(r.providerBad, undefined);
  assert.deepEqual(h.cloud.deletes, []);
  // The existing boot timeout (BOOT_LIMIT_MINUTES=10) is untouched.
  await at(h, 120, (x) => x.service.sweep());
  assert.equal((await record(h)).closed, 'BOOT_TIMEOUT');
  assert.deepEqual(h.cloud.deletes, ['900']);
  const close = await closeLines(h);
  assert.equal(close.length, 1);
  assert.match(close[0], /^\[Worker\] close reason=BOOT_TIMEOUT provider=offline msg=-$/);
});
test('BOOTING: offline then running boots normally; a first POST while offline is NOT_READY, not a close', async () => {
  const h = await rent();
  instance(h).actual_status = 'offline';
  const early = await at(h, 30, (x) => x.report(log(1)));
  assert.equal(early.status, 409);
  assert.deepEqual(h.cloud.deletes, []);
  assert.equal((await record(h)).closed, null);
  instance(h).actual_status = 'running';
  assert.equal((await at(h, 30, (x) => x.report(log(1)))).status, 200);
});
for (const word of ['exited', 'stopped'])
  test(
    'BOOTING: ' +
      word +
      ' (a non-pending status) is tolerated for one reading, closed after the window',
    async () => {
      const h = await rent();
      instance(h).actual_status = word;
      await at(h, 5);
      assert.equal((await record(h)).closed, null);
      await at(h, 60, (x) => x.service.sweep());
      assert.equal((await record(h)).closed, null);
      await at(h, 60, (x) => x.service.sweep());
      assert.equal((await record(h)).closed, 'ENDED');
      assert.deepEqual(h.cloud.deletes, ['900']);
    },
  );

// LOGGING
const nasty =
  'host\u001b[31m lost\u0000 \n##[error] ' +
  'x'.repeat(400) +
  ' Bearer abcdef 10.1.2.3 ' +
  'a'.repeat(40) +
  ' 900 ' +
  env.VAST_API_KEY;
const offlineWith = async (msg) => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  instance(h).status_msg = msg;
  await at(h, 60);
  const line = (await lines(h)).find((l) => l.startsWith('[Worker] provider=offline'));
  assert.ok(line, 'a raw status line');
  return { h, line };
};
test('status line: prints the raw actual_status and the message', async () => {
  const { line } = await offlineWith('Required resources are currently unavailable');
  assert.equal(
    line,
    '[Worker] provider=offline phase=TESTING msg=Required resources are currently unavailable',
  );
});
test('status line: the message is one line without control or injection sequences', async () => {
  const { line } = await offlineWith(nasty);
  assert.match(line, /^\[Worker\] provider=offline phase=TESTING msg=host\S* lost /);
  assert.ok(!/\n|\u001b|\u0000|##\[/.test(line));
});
test('status line: addresses, bearer tokens, long token-like runs and secrets are redacted', async () => {
  const { line } = await offlineWith(
    'Bearer abcdef 10.1.2.3 ' + 'a'.repeat(40) + ' 900 ' + env.VAST_API_KEY + ' end',
  );
  assert.match(line, / end$/);
  assert.ok(!/Bearer abcdef|10\.1\.2\.3|mock-vast-binding/.test(line));
  assert.ok(!line.includes('a'.repeat(32)), 'long token-like runs are redacted');
});
test('status line: the message is bounded to 120 characters', async () => {
  const { line } = await offlineWith('word '.repeat(500));
  assert.ok(
    line.length <= '[Worker] provider=offline phase=TESTING msg='.length + 120,
    String(line.length),
  );
});
test('status line: the stored message is bounded to 120 characters', async () => {
  const { h } = await offlineWith('word '.repeat(500));
  assert.ok(Array.from((await record(h)).providerMsg).length <= 120);
});
test('close line: the message is bounded to 120 characters', async () => {
  const { h } = await offlineWith('word '.repeat(500));
  for (let i = 0; i < 2; i++) await at(h, 60, (x) => x.service.sweep());
  const close = (await closeLines(h))[0];
  assert.ok(close.length <= '[Worker] close reason=ENDED provider=offline msg='.length + 120);
});
test('a status word outside [a-z_]{1,16} is printed as unknown, a missing one as null', async () => {
  const h = await testing();
  instance(h).actual_status = 'ALL-CAPS WORDS';
  await at(h, 60);
  assert.ok(
    (await lines(h)).some((l) => /^\[Worker\] provider=unknown phase=TESTING msg=-/.test(l)),
  );
  instance(h).actual_status = null;
  await at(h, 60);
  assert.ok((await lines(h)).some((l) => /^\[Worker\] provider=null phase=TESTING/.test(l)));
});
for (const word of ['x'.repeat(40), 'a\nb', 'Running', 'a b', '']) {
  test('a status word that is not [a-z_]{1,16} is unknown: ' + JSON.stringify(word), async () => {
    const h = await testing();
    instance(h).actual_status = word;
    await at(h, 60);
    const line = (await lines(h)).find((l) => l.includes(' phase=TESTING'));
    assert.ok(
      (await lines(h)).some((l) => l.startsWith('[Worker] provider=unknown phase=TESTING')),
      JSON.stringify(await lines(h)),
    );
    assert.ok(line);
  });
}
test('flaps are counted per run and shown on the status line', async () => {
  const h = await testing();
  for (let i = 0; i < 2; i++) {
    instance(h).actual_status = 'offline';
    await at(h, 60);
    instance(h).actual_status = 'running';
    await at(h, 60);
  }
  assert.equal((await record(h)).providerFlaps, 2);
  assert.ok(
    (await lines(h)).some((l) =>
      /^\[Worker\] provider=running phase=TESTING msg=- flaps=2$/.test(l),
    ),
  );
  assert.equal((await closeLines(h)).length, 0);
});

// Every close reason writes exactly one line.
const only = async (h, reason, provider = 'running') => {
  const close = await closeLines(h);
  assert.equal(close.length, 1, JSON.stringify(close));
  assert.match(
    close[0],
    new RegExp('^\\[Worker\\] close reason=' + reason + ' provider=' + provider + ' msg=-$'),
  );
};
test('close line: RUN_GONE (the GitHub run completed)', async () => {
  const h = await testing();
  h.cloud.run.status = 'completed';
  await at(h, 120); // past the lifecycle read cache
  assert.equal((await record(h)).closed, 'RUN_GONE');
  await only(h, 'RUN_GONE');
});
test('close line: CONTRACT_PRICE', async () => {
  const h = await rent();
  instance(h).inet_down_cost = 0.0051;
  assert.equal((await h.call('/status/123-1')).status, 503);
  assert.equal((await record(h)).closed, 'CONTRACT_PRICE');
  assert.equal((await closeLines(h)).length, 1);
  assert.match(
    (await closeLines(h))[0],
    /^\[Worker\] close reason=CONTRACT_PRICE provider=none msg=-$/,
  );
});
// The TTL close of report() is not reachable with a valid token: jose treats exp <= now as expired,
// and the token's exp must equal reportExpiry, so verification refuses first. It gets its line from
// the shared close(); the DEADLINE close (heartbeat passed, token still valid) is the live one.
test('close line: DEADLINE (a report after the heartbeat deadline, token still valid)', async () => {
  const h = await testing();
  h.time.value += 16 * 60000;
  assert.equal((await h.report(log(2))).status, 401);
  assert.equal((await record(h)).closed, 'DEADLINE');
  await only(h, 'DEADLINE');
});
test('close line: LIMIT (report count)', async () => {
  const h = await testing({ MAX_REPORTS: '3' });
  await h.report(log(2));
  await h.report(log(3));
  await h.report(log(4));
  assert.equal((await record(h)).closed, 'LIMIT');
  await only(h, 'LIMIT');
});
test('close line: STOPPED (/stop) and STOPPED (/stop-all)', async () => {
  const a = await testing();
  await a.call('/stop/123-1');
  assert.equal((await record(a)).closed, 'STOPPED');
  await only(a, 'STOPPED');
  const b = await testing();
  assert.equal((await b.call('/stop-all', { token: env.OWNER_STOP_TOKEN })).status, 200);
  assert.equal((await record(b)).closed, 'STOPPED');
  await only(b, 'STOPPED');
  assert.deepEqual(b.cloud.deletes, ['900']);
});
test('close line: stop-all writes one line per open rental', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2' });
  const hex = (n) => n.toString(16).padStart(2, '0').repeat(20);
  h.cloud.contracts = [901, 902];
  h.cloud.offer.host_id = 3001;
  h.cloud.extraOffers = [
    {
      ...h.cloud.offer,
      id: 11,
      gpu_name: 'RTX_3060',
      compute_cap: 860,
      machine_id: 200011,
      host_id: 3011,
    },
  ];
  for (const n of [201, 202]) {
    h.cloud.runs.set(String(n), {
      ...h.cloud.run,
      id: n,
      head_sha: hex(n),
      head_branch: 'b' + n,
      pull_requests: [],
    });
    const r = await h.call('/rent', {
      claims: { run_id: String(n) },
      body: {
        runner_nonce: 'f'.repeat(64),
        tested_sha: hex(n),
        profile: 'check',
        remaining_seconds: 12000,
      },
    });
    assert.equal(r.status, 200);
  }
  assert.equal((await h.call('/stop-all', { token: env.OWNER_STOP_TOKEN })).status, 200);
  for (const n of [201, 202]) {
    const r = await h.storage.get('record:' + n + '-1');
    assert.equal(r.closed, 'STOPPED');
    assert.equal(
      r.log.split('\n').filter((l) => l.startsWith('[Worker] close reason=STOPPED')).length,
      1,
    );
  }
});
test('close line: one line even when a second close follows a failed destroy', async () => {
  const h = await testing();
  h.cloud.vastError = 500;
  await h.call('/stop/123-1');
  await h.call('/stop-all', { token: env.OWNER_STOP_TOKEN }).catch(() => {});
  h.cloud.vastError = 0;
  await h.service.sweep();
  assert.equal((await closeLines(h)).length, 1);
});
for (const owner of [true, false])
  test(
    'close line: an ended record from before this change gets no late line (owner stop ' +
      owner +
      ')',
    async () => {
      const h = await testing();
      await h.call('/stop/123-1');
      const r = await record(h);
      delete r.closeLogged;
      r.log = '';
      await h.storage.put('record:123-1', r);
      await h.call('/stop/123-1', owner ? { token: env.OWNER_STOP_TOKEN } : {});
      assert.equal((await closeLines(h)).length, 0);
    },
  );
test('close line: HEARTBEAT_TIMEOUT keeps its destroy line and adds the close line', async () => {
  const h = await testing();
  await at(h, 16 * 60);
  assert.equal((await record(h)).closed, 'HEARTBEAT_TIMEOUT');
  assert.ok((await lines(h)).includes('[Worker] destroy reason=HEARTBEAT_TIMEOUT'));
  await only(h, 'HEARTBEAT_TIMEOUT');
});
test('close line: UNAVAILABLE on a provider API error in update()', async () => {
  const h = await testing();
  h.cloud.vastError = 500;
  assert.equal((await h.call('/status/123-1')).status >= 400, true);
  h.cloud.vastError = 0;
  const close = await closeLines(h);
  assert.equal(close.length, 1);
  assert.match(close[0], /^\[Worker\] close reason=UNAVAILABLE provider=running msg=-$/);
});
test('close line: UNAVAILABLE on a provider API error during BOOTING', async () => {
  const h = await rent();
  h.cloud.vastError = 500;
  await h.call('/status/123-1');
  h.cloud.vastError = 0;
  const close = await closeLines(h);
  assert.equal(close.length, 1);
  assert.match(close[0], /^\[Worker\] close reason=UNAVAILABLE provider=\S+ msg=-$/);
});
test('close line: SUPERSEDED (a newer run of the same branch)', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2' });
  const hex = (n) => n.toString(16).padStart(2, '0').repeat(20);
  const rentRun = async (n) => {
    h.cloud.runs.set(String(n), {
      ...h.cloud.run,
      id: n,
      head_sha: hex(n),
      head_branch: 'x',
      pull_requests: [],
    });
    return h.call('/rent', {
      claims: { run_id: String(n) },
      body: {
        runner_nonce: 'f'.repeat(64),
        tested_sha: hex(n),
        profile: 'check',
        remaining_seconds: 12000,
      },
    });
  };
  assert.equal((await rentRun(201)).status, 200);
  assert.equal((await rentRun(202)).status, 409);
  const old = await h.storage.get('record:201-1');
  assert.equal(old.closed, 'SUPERSEDED');
  const text = old.log.split('\n').filter((l) => l.startsWith('[Worker] close '));
  assert.equal(text.length, 1);
  assert.match(text[0], /^\[Worker\] close reason=SUPERSEDED provider=\S+ msg=-$/);
});
test('close line is written once when the destroy has to be retried', async () => {
  const h = await testing();
  h.cloud.vastError = 500;
  await h.call('/stop/123-1');
  await h.call('/stop/123-1');
  h.cloud.vastError = 0;
  await h.call('/stop/123-1');
  assert.deepEqual(h.cloud.deletes, ['900']);
  assert.equal((await closeLines(h)).length, 1);
});
test('close line is never added to an ended record (stop twice, owner stop after the end)', async () => {
  const h = await testing();
  await h.call('/stop/123-1');
  await h.call('/stop/123-1');
  await h.call('/stop/123-1', { token: env.OWNER_STOP_TOKEN });
  await h.service.sweep();
  assert.equal((await closeLines(h)).length, 1);
  assert.notEqual((await record(h)).ended, null);
});

// Cost and teardown are unchanged.
test('after the tolerance close the rental is destroyed by exact id, settled, and the slot released', async () => {
  const h = await testing();
  instance(h).actual_status = 'offline';
  for (let i = 0; i < 3; i++) await at(h, 60, (x) => x.service.sweep());
  const r = await record(h);
  assert.equal(r.closed, 'ENDED');
  assert.notEqual(r.ended, null);
  assert.deepEqual(h.cloud.deletes, ['900']);
  assert.equal(h.cloud.instances.has('900'), false);
  assert.ok(h.cloud.instances.has('777') && h.cloud.instances.has('888'));
  // The slot is free again (MAX_CONCURRENT_RENTALS=1): a new run is not BUSY.
  h.cloud.runs.set('124', { ...h.cloud.run, id: 124, head_sha: 'b'.repeat(40), pull_requests: [] });
  h.cloud.contracts = [901];
  const next = await h.call('/rent', {
    claims: { run_id: '124' },
    body: {
      runner_nonce: 'f'.repeat(64),
      tested_sha: 'b'.repeat(40),
      profile: 'check',
      remaining_seconds: 19800,
    },
  });
  assert.notEqual((await next.json()).state, 'BUSY');
});
test('tolerance never extends the heartbeat timeout: a silent box with a flapping status is closed at 15 min', async () => {
  const h = await testing();
  for (let i = 0; i < 16; i++) {
    if ((await record(h)).closed) break;
    instance(h).actual_status = i % 2 ? 'running' : 'offline';
    await at(h, 60, (x) => x.service.sweep());
  }
  assert.equal((await record(h)).closed, 'HEARTBEAT_TIMEOUT');
  assert.deepEqual(h.cloud.deletes, ['900']);
});
test('tolerance never extends the hard deadline: a box that posts through an offline status closes at the limit', async () => {
  const h = await testing(); // HARD_RUNTIME_MINUTES=20
  instance(h).actual_status = 'offline';
  for (let i = 0; i < 3; i++)
    assert.equal((await at(h, 5 * 60, (x) => x.report(log(2 + i, 'beat')))).status, 200);
  assert.deepEqual(h.cloud.deletes, []);
  await at(h, 5 * 60, (x) => x.service.sweep());
  assert.equal((await record(h)).closed, 'HARD_TIMEOUT');
  assert.deepEqual(h.cloud.deletes, ['900']);
});
test('result grace: a non-running status after the result is tolerated only for the window', async () => {
  const h = await testing();
  assert.equal(
    (await h.report({ seq: 2, kind: 'result', result: { nonce, verdict: 'PASS' } })).status,
    200,
  );
  instance(h).actual_status = 'offline';
  await at(h, 60, (x) => x.service.sweep());
  assert.equal((await record(h)).closed, null);
  await at(h, 60, (x) => x.service.sweep());
  await at(h, 60, (x) => x.service.sweep());
  assert.equal((await record(h)).closed, 'ENDED');
  assert.equal((await record(h)).verdict, 'PASS');
});

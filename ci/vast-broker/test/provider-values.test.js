import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { setup } from './helpers.js';
import { Ledger } from '../src/ledger.js';
import { redactDoc } from '../src/runreport.js';
import { fileURLToPath } from 'node:url';
import { mergeEndpointValues, redactEndpointValues } from '../src/endpoint-values.js';
import { redactPublic, buildPublicLog } from '../src/publiclog.js';
import { publicLogRenderer } from '../public-log.js';
import { currentBootFailure, observeFatal } from '../src/provider-policy.js';
import { providerPolicy } from '../src/policy.js';
const runnerPath = fileURLToPath(new URL('../runner.py', import.meta.url));
// FALSIFIERS: every known endpoint spelling stays private through the actual relay;
// pending state changes cannot extend a continuous fatal witness beyond its window.
for (const option of ['-vp23456', '-voPort=00023456'])
  test('R17 A clustered SSH port private ' + option, async () => {
    const native = spawnSync('ssh', ['-G', '-F', '/dev/null', option, 'u@203.0.113.91'], {
      encoding: 'utf8',
    });
    assert.equal(native.status, 0);
    assert.match(native.stdout, /^port 23456$/m);
    const puts = [];
    const h = await setup({
      PROVIDER_CONFIRM_SECONDS: '1',
      LOGS_BUCKET: { put: async (k, t) => puts.push(String(t)) },
    });
    assert.equal((await h.call()).status, 200);
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'GPU error, unable to start instance; ssh ' + option + ' u@203.0.113.91',
    });
    for (let i = 0; i < 2; i++) {
      h.time.value += 1000;
      await h.service.sweep();
    }
    const r = await h.storage.get('record:123-1');
    assert.equal(r.closed, 'PROVIDER_FAULT');
    assert.equal(h.cloud.instances.has('900'), false);
    await h.service.publishRuns();
    assert.equal(puts.length, 1);
    assert.equal(
      puts[0].includes('23456'),
      false,
      'final privacy assertion: published error log exposes clustered port',
    );
  });
for (const raw of ['203.0.113.91:23456', '[2001:db8:1234:5678:90ab:cdef:1234:5678]:23456'])
  test('R17 B actual box relay port private ' + raw, async () => {
    const h = await setup();
    assert.equal((await h.call()).status, 200);
    assert.equal(
      (await h.report({ seq: 1, kind: 'log', text: 'PORTCASE ' + raw + '\n' })).status,
      200,
    );
    const r = await h.storage.get('record:123-1'),
      pages = [];
    let cursor = 0;
    while (cursor < r.logChunks) {
      const resp = await h.call('/logs/123-1?cursor=' + cursor, { method: 'GET' });
      assert.equal(resp.status, 200);
      const p = await resp.json();
      pages.push(p);
      cursor = p.next_cursor;
    }
    const py = spawnSync(
      'python3',
      [
        '-I',
        '-c',
        `import runpy,json,sys,io
x=json.load(sys.stdin);m=runpy.run_path(sys.argv[1],run_name='r17_regression');out=[]
r=m['Runner']({'redactions':[],'run':'123-1'},{},None,emit=out.append,summary=io.StringIO());pages=x['pages'];r.retry_read=lambda route:(200,pages.pop(0));r.pages(x['target']);print('\\n'.join(out))`,
        runnerPath,
      ],
      { input: JSON.stringify({ pages, target: r.logChunks }), encoding: 'utf8' },
    );
    assert.equal(py.status, 0, py.stderr);
    assert.match(py.stdout, /\[untrusted\]/);
    assert.equal((await h.call('/stop/123-1')).status, 200);
    assert.equal(h.cloud.instances.has('900'), false);
    assert.equal(
      py.stdout.includes('23456'),
      false,
      'final privacy assertion: actual Runner.pages output exposes bare endpoint port',
    );
  });
for (const restart of [false, true])
  test('R17 C continuous fatal survives pending-state flaps ' + restart, async () => {
    const h = await setup({ HARD_RUNTIME_MINUTES: '180', BOOT_LIMIT_MINUTES: '75' });
    assert.equal((await h.call()).status, 200);
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'GPU error, unable to start instance',
    });
    for (let i = 0; i < 5; i++) {
      h.cloud.instances.get('900').actual_status = ['created', 'loading'][i % 2];
      if (restart) h.restart();
      h.time.value += i ? 30000 : 1000;
      await h.service.sweep();
      if ((await h.storage.get('record:123-1')).ended !== null) break;
    }
    const r = await h.storage.get('record:123-1');
    await h.call('/stop/123-1');
    assert.equal(h.cloud.instances.has('900'), false);
    assert.equal(
      r.closed,
      'PROVIDER_FAULT',
      'final confirmation assertion: identical current fatal persists for 120 seconds without confirmation',
    );
  });

function rng(seed) {
  return () => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed;
  };
}
function valueCases(seed) {
  const next = rng(seed),
    rows = [];
  for (let i = 0; i < 512; i++) {
    const octets = [
        101 + (next() % 100),
        1 + (next() % 253),
        1 + (next() % 253),
        1 + (next() % 253),
      ],
      ip = octets.join('.'),
      n = octets.reduce((a, b) => a * 256 + b, 0),
      port = 10000 + (next() % 50000);
    const forms = [
      ip,
      octets.map((x) => String(x).padStart(4, '0')).join('.'),
      String(n),
      '0x' + n.toString(16),
      octets.map((x) => '0x' + x.toString(16)).join('.'),
      '::ffff:' + ip,
      '0:0:0:0:0:ffff:' + Math.floor(n / 65536).toString(16) + ':' + (n % 65536).toString(16),
    ];
    const ipv6 = '2606:' + ((next() % 65535) + 1).toString(16) + ':0:0:abcd:0:2345:6789';
    const context = mergeEndpointValues(
      mergeEndpointValues({}, { public_ipaddr: ip, ssh_port: port }),
      { public_ipaddr: ipv6 },
    );
    const alias = forms[next() % forms.length],
      zero = '0'.repeat(next() % 12),
      prefix = [
        '-vvvp',
        '-4oPort=',
        'mystery=',
        'pair:[address]:',
        'ssh_port=\n +',
        'digits gluedABC',
        'scp -vP',
      ][next() % 7];
    const raw =
      'event ' +
      prefix +
      zero +
      port +
      ' via [' +
      alias +
      '] alternate ' +
      (next() % 2
        ? ipv6
            .split(':')
            .map((x) => x.padStart(4, '0'))
            .join(':')
        : ipv6.replace(':0:0:', '::')
      ).toUpperCase() +
      '; split=' +
      String(port).slice(0, 2) +
      '\x01' +
      String(port).slice(2) +
      ' count=7 driver=580.88';
    rows.push({ raw, context, port, ip, alias, ipv6 });
  }
  return rows;
}
for (const seed of [731, 2197]) {
  test('values JS semantic property seed ' + seed, async () => {
    for (const row of valueCases(seed)) {
      for (const out of [
        redactEndpointValues(row.raw, row.context),
        redactPublic(row.raw, [], row.context),
        redactDoc({ nested: [row.raw] }, [], row.context).nested[0],
        (
          await buildPublicLog([row.raw.slice(0, 13), row.raw.slice(13)], [], {
            endpoints: row.context,
          })
        ).text,
      ]) {
        assert.equal(out.includes(row.ip), false, row.raw);
        assert.equal(out.toLowerCase().includes(row.alias.toLowerCase()), false, row.raw);
        assert.equal(out.toLowerCase().includes(row.ipv6.toLowerCase()), false, row.raw);
        assert.equal(
          [...out.matchAll(/[0-9]+/g)].some((m) => Number(m[0]) === row.port),
          false,
          row.raw,
        );
        assert.match(out, /count=7 driver=580.88/);
      }
    }
  });
  test('values Python semantic property seed ' + seed, () => {
    const py = spawnSync(
      'python3',
      ['-I', fileURLToPath(new URL('./provider_value_fixture.py', import.meta.url)), runnerPath],
      {
        input: JSON.stringify({ rows: valueCases(seed), renderer: publicLogRenderer }),
        encoding: 'utf8',
        timeout: 30000,
      },
    );
    assert.equal(py.status, 0, py.stderr || py.stdout);
  });
}
for (const variant of [1, 2]) {
  test('values private context is bounded and retained ' + variant, () => {
    let context = mergeEndpointValues({}, { public_ipaddr: '203.0.113.91', ssh_port: 23456 });
    for (let i = 0; i < 200; i++)
      context = mergeEndpointValues(context, {
        public_ipaddr: '198.51.100.' + (i + 1),
        ssh_port: 10000 + i,
      });
    assert.equal(context.ips.length, 128);
    assert.equal(context.ports.length, 128);
    assert.equal(context.overflow, true);
    assert.equal(redactEndpointValues('hex=deadbeef', context), 'hex=[redacted]');
    assert.equal(redactEndpointValues('bare 23456 and 00010199', context).match(/[0-9]/), null);
    assert.deepEqual(mergeEndpointValues({}, { public_ipaddr: 'not-an-address', ssh_port: true }), {
      ips: [],
      ports: [],
      overflow: false,
    });
  });
  test('values masks before and after sanitisation ' + variant, async () => {
    const h = await setup();
    assert.equal((await h.call()).status, 200);
    Object.assign(h.cloud.instances.get('900'), { public_ipaddr: '203.0.113.91', ssh_port: 23456 });
    await h.service.sweep();
    const raw =
      variant === 1
        ? 'unlabelled 00023456 and 3405803867 split=23\x01456'
        : '23\x01456 through 203.0.113.91';
    assert.equal((await h.report({ seq: 1, kind: 'log', text: raw + '\n' })).status, 200);
    const status = await (await h.call('/status/123-1', { method: 'GET' })).json();
    assert.deepEqual(JSON.parse(Buffer.from(status.endpoint_redactions, 'base64').toString()), {
      ips: ['203.0.113.91'],
      ports: [23456],
      overflow: false,
    });
    assert.equal(status.log_tail_sanitised.includes('23456'), false);
    assert.equal(status.log_tail_sanitised.includes('3405803867'), false);
    assert.equal(status.log_tail_sanitised.includes('203.0.113.91'), false);
    const page = await (await h.call('/logs/123-1?cursor=0', { method: 'GET' })).json();
    assert.equal(page.log_increment_sanitised.includes('23456'), false);
    await h.call('/stop/123-1');
    assert.equal(h.cloud.instances.has('900'), false);
  });
  test('values captures context across replacements ' + variant, async () => {
    const h = await setup({
      PROVIDER_CONFIRM_SECONDS: '1',
      DAILY_RENTALS: '20',
      DAILY_SPEND_USD: '100',
    });
    h.cloud.offer.host_id = 100;
    h.cloud.contracts = [900, 901];
    h.cloud.extraOffers = [{ ...h.cloud.offer, id: 22, machine_id: 200002, host_id: 200002 }];
    assert.equal((await h.call()).status, 200);
    Object.assign(h.cloud.instances.get('900'), {
      public_ipaddr: '203.0.113.91',
      ssh_port: 23456,
      actual_status: 'created',
      status_msg: 'GPU error, unable to start instance',
    });
    for (let i = 0; i < 2; i++) {
      h.time.value += 1000;
      await h.service.sweep();
    }
    assert.equal((await h.call()).status, 200);
    Object.assign(h.cloud.instances.get('901'), {
      public_ipaddr: '198.51.100.73',
      ssh_port: 34567,
    });
    await h.service.sweep();
    const status = await (await h.call('/status/123-1', { method: 'GET' })).json();
    const context = JSON.parse(Buffer.from(status.endpoint_redactions, 'base64').toString());
    assert.deepEqual(context.ports, [23456, 34567]);
    assert.equal(context.ips.length, 2);
    assert.equal(
      (
        await h.report({
          seq: 1,
          kind: 'log',
          text: 'old=00023456 new=+0034567 previous 203.0.113.91\n',
        })
      ).status,
      200,
    );
    const r = await h.storage.get('record:123-1');
    assert.equal(/23456|34567|203\.0\.113\.91/.test(r.log), false);
    await h.call('/stop/123-1');
    assert.equal(h.cloud.instances.has('900'), false);
  });
}
const pending = [
  'created',
  'loading',
  'scheduling',
  'starting',
  'offline',
  'running',
  null,
  undefined,
];
function permutations(xs) {
  return xs.length
    ? xs.flatMap((x, i) => permutations(xs.filter((_, j) => j !== i)).map((tail) => [x, ...tail]))
    : [[]];
}
const pendingOrders = [
  ...pending.flatMap((a) => pending.map((b) => [a, b])),
  ...permutations(['created', 'loading', 'scheduling', 'running']),
];
for (const restart of [false, true]) {
  for (const source of ['status', 'boot_log'])
    test(
      'values fatal pending permutations retain confirmation ' + source + ' ' + restart,
      async () => {
        const policy = providerPolicy();
        for (const order of pendingOrders) {
          let failure, cursor;
          for (let i = 0; i < 8; i++) {
            const state = order[i % order.length];
            const observed = await currentBootFailure(
              failure,
              cursor,
              {
                actual_status: state,
                status_msg:
                  source === 'status' ? 'GPU error, unable to start instance' : 'Preparing GPUs',
              },
              source === 'status'
                ? ''
                : "libvirt: QEMU Driver error : Domain not found: no domain with matching name 'C.900'\n",
              '900',
              policy,
            );
            cursor = observed.cursor;
            failure = {
              ...observeFatal(failure, observed.reason, i * 15000, policy, observed.witness),
              source: observed.source,
              state: observed.state,
              statusHash: observed.statusHash,
            };
            if (restart) {
              failure = structuredClone(failure);
              cursor = structuredClone(cursor);
            }
            assert.equal(failure.confirmed, i >= 6, 'pending ' + state + ' sample ' + i);
          }
        }
      },
    );
  test('values changed fatal message starts a new window ' + restart, async () => {
    const policy = providerPolicy();
    let failure, cursor;
    for (const [at, msg] of [
      [0, 'first'],
      [60000, 'second'],
      [90000, 'second'],
      [150000, 'second'],
    ]) {
      const observed = await currentBootFailure(
        failure,
        cursor,
        { actual_status: 'loading', status_msg: 'GPU error, unable to start instance ' + msg },
        '',
        '900',
        policy,
      );
      cursor = observed.cursor;
      failure = {
        ...observeFatal(failure, observed.reason, at, policy, observed.witness),
        source: observed.source,
        state: observed.state,
        statusHash: observed.statusHash,
      };
      if (restart) {
        failure = structuredClone(failure);
        cursor = structuredClone(cursor);
      }
      assert.equal(failure.confirmed, at === 150000);
    }
  });
}

for (const variant of [1, 2]) {
  test('values privacy takes precedence over coincident numeric labels ' + variant, () => {
    const context = mergeEndpointValues({}, { public_ipaddr: '203.0.113.91', ssh_port: 22 });
    assert.equal(
      redactEndpointValues('count=22 digitsABC+00022 tail=1223', context),
      'count=[redacted] digitsABC+[redacted] tail=1223',
    );
  });
  test('values recovery and changed message cancel pending failure ' + variant, async () => {
    const policy = providerPolicy();
    const first = await currentBootFailure(
      null,
      null,
      { actual_status: 'created', status_msg: 'GPU error, unable to start instance' },
      '',
      '900',
      policy,
    );
    const failure = {
      ...observeFatal(null, first.reason, 0, policy, first.witness),
      source: first.source,
      state: first.state,
      statusHash: first.statusHash,
    };
    const recovered = await currentBootFailure(
      failure,
      first.cursor,
      { actual_status: 'running', status_msg: 'GPU error, unable to start instance' },
      'Domain successfully started; GPUs ready\n',
      '900',
      policy,
    );
    assert.equal(recovered.reason, null);
    const stale = await currentBootFailure(
      null,
      recovered.cursor,
      { actual_status: 'loading', status_msg: 'GPU error, unable to start instance' },
      'Domain successfully started; GPUs ready\n',
      '900',
      policy,
    );
    assert.equal(stale.reason, null);
    const changed = await currentBootFailure(
      null,
      recovered.cursor,
      { actual_status: 'loading', status_msg: 'GPU error, unable to start instance again' },
      'Domain successfully started; GPUs ready\n',
      '900',
      policy,
    );
    assert.equal(changed.reason, 'GPU_START_ERROR');
  });
}

for (const variant of [1, 2])
  test('values publication masks legacy chunks using private context ' + variant, async () => {
    const puts = [],
      h = await setup({ LOGS_BUCKET: { put: async (k, t) => puts.push(String(t)) } });
    assert.equal((await h.call()).status, 200);
    Object.assign(h.cloud.instances.get('900'), { public_ipaddr: '203.0.113.91', ssh_port: 23456 });
    await h.service.sweep();
    const ledger = new Ledger(h.storage),
      r = await h.storage.get('record:123-1');
    await ledger.appendPublicLog(
      r,
      'legacy unknown operand ABC+00023456 numeric address 3405803867\n',
    );
    await ledger.save(r);
    await h.call('/stop/123-1');
    await h.service.publishRuns();
    assert.equal(puts.length, 1);
    assert.equal(/23456|3405803867/.test(puts[0]), false);
    assert.match(puts[0], /legacy unknown operand/);
  });

for (const variant of [1, 2])
  test('values recovery status uses the data table ' + variant, async () => {
    const policy = { ...providerPolicy(), recoverySignatures: ['CUSTOM READY ' + variant] };
    const log =
      "libvirt: QEMU Driver error : Domain not found: no domain with matching name 'C.900'\n";
    const unknown = await currentBootFailure(
      null,
      null,
      { actual_status: 'running', status_msg: 'GPUs ready' },
      log,
      '900',
      policy,
    );
    assert.equal(unknown.reason, 'KVM_DOMAIN_NOT_FOUND');
    const healthy = await currentBootFailure(
      null,
      unknown.cursor,
      { actual_status: 'loading', status_msg: 'CUSTOM READY ' + variant },
      log,
      '900',
      policy,
    );
    assert.equal(healthy.reason, null);
    assert.ok(healthy.cursor.healthyStatusHash);
  });

for (const variant of [1, 2])
  test('values endpoint masks preserve existing credential redaction ' + variant, async () => {
    const port = 23455 + variant,
      token = 'leftprivate' + port + 'rightprivate',
      context = { ips: [], ports: [port], overflow: false };
    assert.equal(
      redactPublic('credential=' + token, [token], context).includes('rightprivate'),
      false,
    );
    const h = await setup({ BLOB_GITHUB_TOKEN: token });
    assert.equal((await h.call()).status, 200);
    Object.assign(h.cloud.instances.get('900'), { public_ipaddr: '203.0.113.91', ssh_port: port });
    await h.service.sweep();
    const before = await h.storage.get('record:123-1');
    let seq = 0,
      removed = 0;
    for (const text of [token, token.slice(0, variant + 2) + '\x01' + token.slice(variant + 2)]) {
      assert.equal(
        (await h.report({ seq: ++seq, kind: 'log', text: 'credential=' + text + '\n' })).status,
        200,
      );
      removed += text.length - '[redacted]'.length;
      const after = await h.storage.get('record:123-1');
      assert.equal(after.fullBytesDropped - (before.fullBytesDropped ?? 0), removed);
      assert.equal(after.bytesDropped - (before.bytesDropped ?? 0), removed);
    }
    const r = await h.storage.get('record:123-1');
    await h.call('/stop/123-1');
    assert.equal(r.log.includes('rightprivate'), false);
    assert.equal(r.log.includes('leftprivate'), false);
  });

for (const port of [22, 23])
  test('values redaction expansion stays inside stored log bounds ' + port, async () => {
    const h = await setup({ MAX_REPORT_BYTES: '1024' });
    assert.equal((await h.call()).status, 200);
    Object.assign(h.cloud.instances.get('900'), { public_ipaddr: '203.0.113.91', ssh_port: port });
    await h.service.sweep();
    assert.equal(
      (
        await h.report({
          seq: 1,
          kind: 'log',
          text: (String(port)[0] + '\x01' + String(port)[1] + '\n').repeat(60),
        })
      ).status,
      200,
    );
    const r = await h.storage.get('record:123-1');
    await h.call('/stop/123-1');
    assert.ok(r.fullBytes <= 1024, 'log cap=' + r.fullBytes);
  });

for (const port of [23456, 23457])
  test('values first report learns live provider endpoint ' + port, async () => {
    const puts = [],
      h = await setup({ LOGS_BUCKET: { put: async (k, t) => puts.push(String(t)) } });
    assert.equal((await h.call()).status, 200);
    Object.assign(h.cloud.instances.get('900'), { public_ipaddr: '203.0.113.91', ssh_port: port });
    assert.equal(
      (await h.report({ seq: 1, kind: 'log', text: 'unlabelled=' + port + '\n' })).status,
      200,
    );
    await h.call('/stop/123-1');
    await h.service.publishRuns();
    assert.equal(puts.length, 1);
    assert.equal(puts[0].includes(String(port)), false);
  });

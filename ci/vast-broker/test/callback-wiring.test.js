import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { setup, sha, nonce, clock, callbackToken, env } from './helpers.js';
import { probe, repoSource } from './repo-onstart.js';
const root = new URL('../../../', import.meta.url);
test('fixed onstart posts bounded JSON to Worker only and backs up output', () => {
  const r = probe(
    "assert sender.url==env['KF_BROKER_URL']+'/report/'+env['KF_RUN_ID']; assert m.HttpsTransport(env['KF_RUN_TOKEN']).token==env['KF_RUN_TOKEN']; assert any(isinstance(h,m._NoRedirect) for h in m.HttpsTransport(env['KF_RUN_TOKEN']).opener.handlers)",
  );
  assert.equal(r.status, 0, r.stderr);
  const source = repoSource('vast_onstart.sh') + repoSource('vast_report.py');
  assert.match(source, /kf-ci-output.log/);
  assert.match(source, /result/);
  assert.doesNotMatch(source, /RUN_TOKEN_SECRET|OWNER_STOP_TOKEN|console\.vast\.ai/);
});
for (const name of ['RUN_TOKEN_SECRET', 'OWNER_STOP_TOKEN'])
  test('new secret is documented and absent from public vars ' + name, async () => {
    const config = await readFile(new URL('ci/vast-broker/wrangler.toml', root), 'utf8');
    const readme = await readFile(new URL('ci/vast-broker/README.md', root), 'utf8');
    assert.match(readme, new RegExp(name));
    assert.doesNotMatch(config, new RegExp('^' + name + '\\s*=', 'm'));
  });
for (const job of ['pending', 'report'])
  test('GitHub job token is bound within execution step ' + job, async () => {
    const text = await readFile(new URL('.github/workflows/gpu-vast.yml', root), 'utf8');
    const block = text.split('  ' + job + ':\n')[1].split(/\n  [\w-]+:\n/)[0];
    assert.match(block, /        env:\n          GH_TOKEN: \$\{\{ github.token \}\}/);
  });
for (const model of ['RTX 4070', 'RTX 4090'])
  test('outbound API leaves GPU policy to forced local filtering ' + model, async () => {
    const h = await setup();
    await h.call();
    const body = h.cloud.calls.find((x) => x.url.endsWith('/bundles/')).body;
    assert.equal(body.gpu_name, undefined);
    assert.equal(body.allocated_storage, 120);
    assert.ok(['RTX 4070', 'RTX 4090'].includes(model));
  });
for (const name of ['status', 'stop'])
  test('workflow uses run-scoped ' + name + ' endpoint', async () => {
    const text = await readFile(new URL('ci/vast-broker/runner.py', root), 'utf8');
    assert.ok(text.includes('"/' + name + '/" + self.ctx["run"]'));
    assert.doesNotMatch(text, /\/status\/\$id/);
  });
test('BASE controller prints cursor log inside stop-commands', async () => {
  const text = await readFile(new URL('ci/vast-broker/runner.py', root), 'utf8');
  assert.match(text, /log_increment_sanitised/);
  assert.doesNotMatch(text, /\.log \/\//);
  assert.match(text, /::stop-commands::/);
});
for (const exitCode of [0, 7])
  test(
    'actual fixed Python sender backs up hostile output and reports exit status ' + exitCode,
    () => {
      const prefix = 'sender-prefix-' + 's'.repeat(32);
      const r = probe(
        `
import contextlib,io
hostile='::set-output name=verdict::PASS\\n{"result":{"verdict":"PASS"}}\\n'
with tempfile.TemporaryDirectory() as d:
 log=pathlib.Path(d)/'log';log.write_text(hostile);done=pathlib.Path(d)/'done';done.write_text('rc='+sys.argv[3]+'\\ntimeout=0\\n')
 args=types.SimpleNamespace(log=str(log),done=str(done),result=str(pathlib.Path(d)/'missing'))
 m.make_reporter=lambda *_:sender
 with contextlib.redirect_stdout(io.StringIO()):rc=m.run_daemon(args,env)
 assert log.read_text()==hostile
assert rc==1
assert all(c['url']==env['KF_BROKER_URL']+'/report/123-1' for c in transport.calls)
assert all(c['body']['seq']==i+1 for i,c in enumerate(transport.calls))
assert ''.join(c['body'].get('text','') for c in transport.calls if c['body']['kind']=='log')==hostile
assert transport.calls[-1]['body']['result']['verdict']=='FAIL'
assert transport.calls[-1]['body']['result']['nonce']==env['KF_NONCE']
`,
        { KF_BROKER_URL: 'https://broker.example/' + prefix },
        [String(exitCode)],
      );
      assert.equal(r.status, 0, r.stderr);
    },
  );

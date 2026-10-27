import test from 'node:test';
import assert from 'node:assert/strict';
import { probe, repoSource } from './repo-onstart.js';
for (const exitCode of [0, 7])
  test('V4 sender preserves fast full bounded output once ' + exitCode, () => {
    const r = probe(
      `
import contextlib,io
output=''.join('message %d of bounded output\\n'%i for i in range(3000))
with tempfile.TemporaryDirectory() as d:
 log=pathlib.Path(d)/'log';log.write_text(output);done=pathlib.Path(d)/'done';done.write_text('rc='+sys.argv[3]+'\\ntimeout=0\\n')
 args=types.SimpleNamespace(log=str(log),done=str(done),result=str(pathlib.Path(d)/'missing'))
 m.make_reporter=lambda *_:sender
 printed=io.StringIO()
 with contextlib.redirect_stdout(printed):rc=m.run_daemon(args,env)
 assert log.read_text()==output
assert 'Worker reports 1 minutes remaining' in printed.getvalue()
ack=sender.ack
assert ack['grace_remaining_seconds']==60
assert ''.join(c['body'].get('text','') for c in transport.calls if c['body']['kind']=='log')==output
assert 'message 2999' not in transport.calls[-1]['body'].get('text','')
assert transport.calls[-1]['body']['result']['verdict']=='FAIL'
`,
      {},
      [String(exitCode)],
    );
    assert.equal(r.status, 0, r.stderr);
    assert.match(repoSource('vast_onstart.sh'), />>"\$log" 2>&1 <\/dev\/null/);
  });

import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// FALSIFIER: an API error URL or an unregistered tunnel admits live creation.
for (const [name, log] of [
  [
    'API timeout',
    'failed to request quick Tunnel: Post "https://api.trycloudflare.com/tunnel": timeout',
  ],
  [
    'unregistered URL',
    'Your quick Tunnel has been created!\nhttps://test-only-fake-tunnel.trycloudflare.com\nconnection refused',
  ],
])
  test('live tunnel parser refuses ' + name, () => {
    const script = `import importlib.util,sys,json
sys.path.insert(0,sys.argv[1])
spec=importlib.util.spec_from_file_location('live',sys.argv[1]+'/smoke-live.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
assert m.ready_tunnel(sys.argv[2]) is None
good='Your quick Tunnel has been created!\\nhttps://test-only-fake-tunnel.trycloudflare.com\\nRegistered tunnel connection'
assert m.ready_tunnel(good)=='https://test-only-fake-tunnel.trycloudflare.com'
`;
    const p = spawnSync('python3', ['-c', script, new URL('../', import.meta.url).pathname, log], {
      encoding: 'utf8',
    });
    assert.equal(p.status, 0, p.stderr);
  });

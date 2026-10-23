// Tests execute the actual PR104 modules/helpers; only clocks/transports are fake.
import { readFileSync } from 'node:fs';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { DEFAULTS } from '../src/defaults.js';
export const repoRoot = new URL('../../../scripts/ci/', import.meta.url);
export const repoSource = (name) => readFileSync(new URL(name, repoRoot), 'utf8');
export function boxEnv(patch = {}) {
  return {
    KF_PROTOCOL: '5',
    KF_BROKER_URL: 'https://broker.example',
    KF_RUN_ID: '123-1',
    KF_RUN_TOKEN:
      'v2.123.1.1800030000.' + 'b'.repeat(32) + '.' + 'a'.repeat(40) + '.' + 'A'.repeat(43),
    KF_NONCE: 'b'.repeat(32),
    KF_SHA: 'a'.repeat(40),
    KF_REPO: 'reindertpelsma/kayfabe',
    KF_RUNNER_NONCE: 'f'.repeat(64),
    KF_BOOT_DEADLINE: '1800000200',
    KF_HARD_DEADLINE: '1800000400',
    KF_SOFT_DEADLINE: '1800000300',
    KF_HEARTBEAT_SECONDS: String(DEFAULTS.heartbeatMinutes * 60),
    KF_GRACE_PASS: '300',
    KF_GRACE_FAIL: '600',
    ...patch,
  };
}
export function probe(body, patch = {}, args = []) {
  const prelude = `import importlib.util,json,sys,tempfile,pathlib,os,types
s=importlib.util.spec_from_file_location('report',sys.argv[1]);m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
env=json.loads(sys.argv[2])
class Clock:
 def __init__(self):self.value=1.0
 def __call__(self):return self.value
 def sleep(self,seconds):self.value+=max(seconds,0.001)
class Capture:
 def __init__(self):self.calls=[];self.responses=[]
 def post(self,url,body):
  self.calls.append({'url':url,'body':json.loads(body),'raw':body})
  return self.responses.pop(0) if self.responses else (200,{'grace_remaining_seconds':60})
clock=Clock();transport=Capture();limits=m.Limits(min_gap=0.001,poll=0.001,flush=0.001,final_budget=60)
sender=m.Reporter(env['KF_BROKER_URL'],env['KF_RUN_ID'],env['KF_RUN_TOKEN'],int(env['KF_HEARTBEAT_SECONDS']),limits,transport,clock)
sender.sleep=clock.sleep
`;
  return spawnSync(
    'python3',
    [
      '-I',
      '-c',
      prelude + body,
      new URL('vast_report.py', repoRoot).pathname,
      JSON.stringify(boxEnv(patch)),
      ...args,
    ],
    { encoding: 'utf8', timeout: 10000 },
  );
}
export async function helperProbe(name, args = [], patch = {}) {
  const dir = await mkdtemp(tmpdir() + '/v5-helper-');
  const env = boxEnv(patch);
  try {
    await writeFile(
      dir + '/env',
      Object.entries(env)
        .map(([k, v]) => `export ${k}='${v}'`)
        .join('\n') + '\n',
      { mode: 0o600 },
    );
    await writeFile(
      dir + '/curl',
      `#!/usr/bin/env python3\nimport json,sys,os\nwith open(os.environ['CAPTURE'],'w') as f:json.dump({'args':sys.argv[1:],'stdin':sys.stdin.read()},f)\nprint('200')\n`,
      { mode: 0o700 },
    );
    const run = spawnSync('sh', [new URL(name, repoRoot).pathname, ...args], {
      encoding: 'utf8',
      env: {
        ...process.env,
        PATH: dir + ':' + process.env.PATH,
        KF_CI_ENV_FILE: dir + '/env',
        CAPTURE: dir + '/capture',
      },
    });
    let capture = {};
    try {
      capture = JSON.parse(await readFile(dir + '/capture', 'utf8'));
    } catch {}
    return { ...run, ...capture };
  } finally {
    await rm(dir, { recursive: true });
  }
}

import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cp, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
// FALSIFIER V4: a routine deploy silently drops or invents public vars, deploys an unreviewed
// bundle, accepts retired Access or secret-named vars, or prints a configured value.
const broker = fileURLToPath(new URL('../', import.meta.url));
const OWNER = '229738317';
async function fixture({ bundle = 'export default {};\n', tomlOwner = OWNER, version } = {}) {
  const root = await mkdtemp(join(tmpdir(), 'v4-deploy-'));
  const site = join(root, 'broker');
  await mkdir(join(site, 'dist'), { recursive: true });
  await cp(broker + 'wrangler.toml', join(site, 'wrangler.toml'));
  const digest = createHash('sha256').update(bundle).digest('hex');
  await writeFile(join(site, 'dist', 'worker.js'), bundle);
  await writeFile(join(site, 'deployment.json'), JSON.stringify({ bundle_sha256: digest }));
  const toml = (await readFile(join(site, 'wrangler.toml'), 'utf8'))
    .replace(/^DEPLOYED_BUNDLE_SHA256 = ".*"$/m, 'DEPLOYED_BUNDLE_SHA256 = "' + digest + '"')
    .replace(/^OWNER_GITHUB_ID = ".*"$/m, 'OWNER_GITHUB_ID = "' + tomlOwner + '"');
  await writeFile(join(site, 'wrangler.toml'), toml);
  const log = join(root, 'wrangler.log');
  await writeFile(
    join(root, 'wrangler'),
    '#!/usr/bin/env node\n' +
      "const fs=require('fs');const a=process.argv.slice(2);\n" +
      "if(a[0]==='--version'){console.log('" +
      (version ?? '4.152.0') +
      "');process.exit(0);}\n" +
      'fs.appendFileSync(' +
      JSON.stringify(log) +
      ',JSON.stringify({argv:a,cwd:process.cwd()})+"\\n");\n' +
      "console.log('echo '+a.join(' '));console.error('err '+a.join(' '));\n",
    { mode: 0o700 },
  );
  return { root, site, log };
}
async function deploy(f, vars, extra = []) {
  const file = join(f.root, 'vars.env');
  await writeFile(file, vars);
  const run = spawnSync(process.execPath, [broker + 'deploy.mjs', file, ...extra], {
    encoding: 'utf8',
    timeout: 20000,
    env: { ...process.env, KF_DEPLOY_ROOT: f.site, WRANGLER: join(f.root, 'wrangler') },
  });
  let calls = [];
  try {
    calls = (await readFile(f.log, 'utf8')).trim().split('\n').map(JSON.parse);
  } catch {}
  return { ...run, calls, text: run.stdout + run.stderr };
}
for (const dry of [false, true])
  test('round deploy helper passes exactly the vars file ' + dry, async () => {
    const f = await fixture();
    try {
      const r = await deploy(
        f,
        '# public owner values\nOWNER_GITHUB_ID=' + OWNER + '\nDENIED_COUNTRIES=["CU","IR"]\n',
        dry ? ['--dry-run'] : [],
      );
      assert.equal(r.status, 0, r.text);
      assert.deepEqual(
        r.calls.map((x) => x.argv),
        [
          [
            'deploy',
            ...(dry ? ['--dry-run'] : []),
            '--var',
            'OWNER_GITHUB_ID:' + OWNER,
            '--var',
            'DENIED_COUNTRIES:["CU","IR"]',
          ],
        ],
      );
      assert.equal(r.calls[0].cwd, f.site);
      assert.ok(!r.text.includes(OWNER) && !r.text.includes('"CU"'), r.text);
      assert.match(r.text, /<OWNER_GITHUB_ID>/);
    } finally {
      await rm(f.root, { recursive: true });
    }
  });
for (const [name, vars, options] of [
  ['partial-access', 'OWNER_GITHUB_ID=229738317\nACCESS_AUD=aud-value-123\n'],
  [
    'complete-access',
    'OWNER_GITHUB_ID=229738317\nACCESS_TEAM_DOMAIN=x.cloudflareaccess.com\n' +
      'ACCESS_AUD=aud-value-123\nACCESS_RUNNER_CLIENT_ID=a.access\nACCESS_BOX_CLIENT_ID=b.access\n',
  ],
  ['box-access', 'OWNER_GITHUB_ID=229738317\nBOX_ACCESS_ID=box-value-123\n'],
  ['secret-name', 'OWNER_GITHUB_ID=229738317\nVAST_API_KEY=secret-value-123\n'],
  ['test-issuer', 'OWNER_GITHUB_ID=229738317\nALLOW_TEST_ISSUER=true\n'],
  ['missing-owner', 'DENIED_COUNTRIES=["CU"]\n'],
  ['non-numeric-owner', 'OWNER_GITHUB_ID=REPLACE_ME\n'],
  ['other-owner', 'OWNER_GITHUB_ID=42\n'],
  ['other-long-owner', 'OWNER_GITHUB_ID=2297383170\n'],
  ['duplicate', 'OWNER_GITHUB_ID=229738317\nOWNER_GITHUB_ID=229738317\n'],
  ['spaces', 'OWNER_GITHUB_ID = 229738317\n'],
  ['renting-word', 'OWNER_GITHUB_ID=229738317\nRENTING_ENABLED=yes\n'],
  ['bundle-mismatch', 'OWNER_GITHUB_ID=229738317\n', { tamper: true }],
  ['manifest-mismatch', 'OWNER_GITHUB_ID=229738317\n', { manifest: true }],
  ['toml-hash-mismatch', 'OWNER_GITHUB_ID=229738317\n', { tomlHash: true }],
  ['wrangler-version', 'OWNER_GITHUB_ID=229738317\n', { version: '4.151.0' }],
  ['wrangler-patch', 'OWNER_GITHUB_ID=229738317\n', { version: 'wrangler 4.152.01' }],
])
  test('round deploy helper refuses before wrangler deploy ' + name, async () => {
    const f = await fixture(options);
    try {
      if (options?.tamper) await writeFile(join(f.site, 'dist', 'worker.js'), 'tampered\n');
      if (options?.manifest)
        await writeFile(join(f.site, 'deployment.json'), JSON.stringify({ bundle_sha256: '0' }));
      if (options?.tomlHash) {
        const toml = await readFile(join(f.site, 'wrangler.toml'), 'utf8');
        await writeFile(
          join(f.site, 'wrangler.toml'),
          toml.replace(/^DEPLOYED_BUNDLE_SHA256 = ".*"$/m, 'DEPLOYED_BUNDLE_SHA256 = "0"'),
        );
      }
      const r = await deploy(f, vars);
      assert.notEqual(r.status, 0);
      assert.deepEqual(r.calls, []);
      for (const secret of ['aud-value-123', 'box-value-123', 'secret-value-123'])
        assert.ok(!r.text.includes(secret), r.text);
    } finally {
      await rm(f.root, { recursive: true });
    }
  });
for (const via of ['path', 'symlink'])
  test('round deploy helper refuses a vars file inside the checkout ' + via, async () => {
    const f = await fixture();
    const inside = join(broker, 'test', 'fixtures', 'deploy-vars-inside-' + via + '.env');
    try {
      await writeFile(inside, 'OWNER_GITHUB_ID=' + OWNER + '\n');
      const link = join(f.root, 'link.env');
      if (via === 'symlink') await symlink(inside, link);
      const run = spawnSync(
        process.execPath,
        [broker + 'deploy.mjs', via === 'path' ? inside : link],
        {
          encoding: 'utf8',
          env: { ...process.env, KF_DEPLOY_ROOT: f.site, WRANGLER: join(f.root, 'wrangler') },
        },
      );
      assert.notEqual(run.status, 0);
      await assert.rejects(readFile(f.log, 'utf8'));
    } finally {
      await rm(inside, { force: true });
      await rm(f.root, { recursive: true });
    }
  });

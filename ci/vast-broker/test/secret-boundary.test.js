import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { checkWorkflows } from '../workflow-guard.js';
const actual = await readFile(
  new URL('../../../.github/workflows/gpu-vast.yml', import.meta.url),
  'utf8',
);
// FALSIFIER WK1 domain: only the public URL changes context; path and Access stay secrets.
const safe = actual.replace('secrets.VAST_BROKER_URL', 'vars.VAST_BROKER_URL');
const errors = (text, extra = {}) => checkWorkflows({ 'gpu-vast.yml': text, ...extra });
const rejected = (text, extra) =>
  assert.ok(
    errors(text, extra).some((x) => x.startsWith('Broker endpoint')),
    JSON.stringify(errors(text, extra)),
  );
const injectBroker = (text, line) => {
  const start = text.indexOf('  gpu:\n'),
    end = text.indexOf('  report:\n', start),
    broker = text.indexOf('        id: broker', start),
    at = text.indexOf('          set -euo pipefail', broker);
  assert.ok(at > start && at < end, 'broker fixture scope');
  return text.slice(0, at) + line + '\n' + text.slice(at);
};
test('actual broker binds public URL and secret path only inside its execution step', () => {
  assert.doesNotMatch(actual, /secrets\.VAST_BROKER_URL/);
  assert.match(actual, /          WORKER_URL: \$\{\{ vars\.VAST_BROKER_URL \}\}/);
  assert.match(actual, /          WORKER_PATH: \$\{\{ secrets\.VAST_BROKER_PATH \}\}/);
  const job = actual.split('  gpu:\n')[1].split('  report:\n')[0];
  assert.doesNotMatch(job.split('    steps:')[0], /VAST_BROKER_URL|VAST_BROKER_PATH/);
});
test('canonical broker public URL and secret bindings pass the guard', () =>
  assert.deepEqual(errors(safe), []));
for (const name of ['URL', 'PATH']) {
  test('broker binding context is exact for ' + name, () =>
    rejected(
      safe.replace(
        (name === 'URL' ? 'vars.' : 'secrets.') + 'VAST_BROKER_' + name,
        (name === 'URL' ? 'secrets.' : 'vars.') + 'VAST_BROKER_' + name,
      ),
    ),
  );
  for (const id of ['pending', 'report', 'run'])
    test('another job cannot receive ' + name + ' secret ' + id, () =>
      rejected(
        safe +
          '  ' +
          (id === 'run' ? 'run' : 'leak-' + id) +
          ':\n    env:\n      ENDPOINT: ${{ secrets.VAST_BROKER_' +
          name +
          ' }}\n    steps: []\n',
      ),
    );
  for (const sink of ['name', 'run', 'summary'])
    test('secret context cannot enter ' + sink + ' ' + name, () => {
      const line =
        sink === 'name'
          ? '        name: ${{ secrets.VAST_BROKER_' + name + ' }}'
          : sink === 'run'
            ? '          echo "${{ secrets.VAST_BROKER_' + name + ' }}"'
            : '          printf "%s" "${{ secrets.VAST_BROKER_' +
              name +
              ' }}" >> "$GITHUB_STEP_SUMMARY"';
      rejected(injectBroker(safe, line));
    });
  test('other workflow cannot receive ' + name + ' secret', () =>
    rejected(safe, {
      'other.yml':
        'jobs:\n  work:\n    env:\n      HIDDEN: ${{ secrets.VAST_BROKER_' +
        name +
        ' }}\n    steps: []\n',
    }),
  );
  test('secret binding in a second broker step fails ' + name, () =>
    rejected(
      safe.replace(
        '  report:\n',
        '      - name: Other\n        env:\n          WORKER_' +
          (name === 'URL' ? 'URL' : 'PATH') +
          ': ${{ secrets.VAST_BROKER_' +
          name +
          ' }}\n        run: true\n  report:\n',
      ),
    ),
  );
}
for (const expression of ['toJSON(secrets)', 'secrets[inputs.name]', "secrets['VAST_BROKER_URL']"])
  test('opaque secret object access fails ' + expression, () =>
    rejected(safe + '  extra:\n    env:\n      ALL: ${{ ' + expression + ' }}\n    steps: []\n'),
  );
for (const alias of ['WORKER_URL', 'WORKER_PATH']) {
  test('endpoint alias cannot enter another job ' + alias, () =>
    rejected(safe + '  extra:\n    env:\n      ' + alias + ': hidden\n    steps: []\n'),
  );
  test('broker cannot echo endpoint alias ' + alias, () =>
    assert.ok(
      errors(injectBroker(safe, '          echo "$' + alias + '"')).includes(
        'Broker endpoint cannot be printed',
      ),
    ),
  );
  test('broker cannot put endpoint alias in a summary ' + alias, () =>
    assert.ok(
      errors(
        injectBroker(safe, '          printf "%s" "$' + alias + '" >> "$GITHUB_STEP_SUMMARY"'),
      ).includes('Broker endpoint cannot be printed'),
    ),
  );
  test('broker cannot export endpoint alias ' + alias, () =>
    rejected(injectBroker(safe, '          printf "url=%s" "$' + alias + '" >> "$GITHUB_OUTPUT"')),
  );
}
for (const output of ['url', 'path'])
  test('broker job output cannot carry endpoint ' + output, () =>
    rejected(
      safe.replace(
        '    outputs:\n      state:',
        '    outputs:\n      ' +
          output +
          ': ${{ steps.broker.outputs.' +
          output +
          ' }}\n      state:',
      ),
    ),
  );
for (const name of ['BROKER_PATH_PREFIX'])
  test('Worker endpoint binding is a secret ' + name, async () => {
    const config = await readFile(new URL('../wrangler.toml', import.meta.url), 'utf8');
    assert.doesNotMatch(config, new RegExp('^' + name + '\\s*=', 'm'));
    assert.match(config, new RegExp(name));
  });

for (const style of ['inline', 'folded'])
  for (const alias of ['WORKER_URL', 'WORKER_PATH'])
    test('unsupported broker run style fails closed ' + style + ' ' + alias, () => {
      const a = safe.indexOf('        run: |', safe.indexOf('  gpu:\n')),
        b = safe.indexOf('  report:\n', a);
      const replacement =
        style === 'inline'
          ? '        run: echo "$' + alias + '"\n'
          : '        run: >\n          echo "$' + alias + '"\n';
      rejected(safe.slice(0, a) + replacement + safe.slice(b));
    });

for (const variable of ['box_text', 'raw_response'])
  test('V4 only trusted renderer may write box data summary ' + variable, () =>
    rejected(
      injectBroker(
        safe,
        '          printf \"%s\" \"$' + variable + '\" >> \"$GITHUB_STEP_SUMMARY\"',
      ),
    ),
  );

for (const filename of ['gpu-vast.yml', 'gpu-vast-sweep.yml'])
  test('domain public URL guard and environment ' + filename, async () => {
    const workflow = await readFile(
      new URL('../../../.github/workflows/' + filename, import.meta.url),
      'utf8',
    );
    assert.match(workflow, /    environment: gpu-vast/);
    assert.match(workflow, /WORKER_URL: \$\{\{ vars\.VAST_BROKER_URL \}\}/);
    assert.deepEqual(checkWorkflows({ 'gpu-vast.yml': safe, [filename]: workflow }), []);
    const old = workflow.replace('vars.VAST_BROKER_URL', 'secrets.VAST_BROKER_URL');
    assert.ok(checkWorkflows({ 'gpu-vast.yml': safe, [filename]: old }).length);
  });
test('domain WORKER_URL is public config, while prefix remains secret', async () => {
  const config = await readFile(new URL('../wrangler.toml', import.meta.url), 'utf8');
  assert.match(config, /^WORKER_URL = "https:\/\/ci\.kayfabe\.dev"$/m);
  assert.doesNotMatch(config, /^BROKER_PATH_PREFIX\s*=/m);
});

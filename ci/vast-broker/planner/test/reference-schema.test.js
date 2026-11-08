import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { plan, checkSchema, contractSchema } from '../index.js';
import { fixture, clone } from './helpers.js';
// Reference implementation is a CI test dependency only, never a Worker import.
test('versioned JSON Schemas conform to independent Draft 2020-12 engine', () => {
  const f = fixture(),
    p = plan(f),
    examples = [
      ['input', f],
      ['catalog', f.catalog],
      ['requirements', f.requirements],
      ['limits', f.limits],
      ['box', f.boxes[0]],
      ['plan', p],
      ['assignment', p.assignments[0]],
    ];
  const proposed = JSON.parse(
    readFileSync(new URL('../fixtures/release-proposed.json', import.meta.url)),
  );
  examples.push(['input', proposed], ['plan', plan(proposed)]);
  const vectors = [];
  for (const [name, value] of examples) {
    vectors.push({ name, value });
    for (const key of Object.keys(value)) {
      const bad = clone(value);
      delete bad[key];
      vectors.push({ name, value: bad });
    }
    for (const version of [0, 2, '1']) vectors.push({ name, value: { ...clone(value), version } });
    vectors.push({ name, value: { ...clone(value), unknown: 'no' } });
  }
  for (const v of vectors) {
    try {
      checkSchema(v.name, v.value);
      v.valid = true;
    } catch {
      v.valid = false;
    }
  }
  const script = `import json,sys\nfrom jsonschema import Draft202012Validator\ndata=json.load(sys.stdin)\nschema=data['schema']\nDraft202012Validator.check_schema(schema)\nerrors=[]\nfor i,v in enumerate(data['vectors']):\n wrapped={**schema,'$ref':'#/$defs/'+v['name']}\n valid=Draft202012Validator(wrapped).is_valid(v['value'])\n if valid!=v['valid']:errors.append({'index':i,'name':v['name'],'js':v['valid'],'reference':valid})\nprint(json.dumps({'count':len(data['vectors']),'mismatches':errors}))\n`;
  const result = spawnSync('python3', ['-I', '-c', script], {
    input: JSON.stringify({ schema: contractSchema, vectors }),
    encoding: 'utf8',
    maxBuffer: 4 * 1024 * 1024,
  });
  assert.equal(result.status, 0, result.stderr);
  const out = JSON.parse(result.stdout);
  assert.ok(out.count > 100);
  assert.deepEqual(out.mismatches, []);
  for (const name of ['catalog', 'requirements', 'limits', 'box', 'input', 'plan', 'assignment']) {
    const standalone = JSON.parse(
      readFileSync(new URL('../schemas/' + name + '.schema.json', import.meta.url)),
    );
    assert.equal(standalone.$ref, 'contract.schema.json#/$defs/' + name);
  }
});

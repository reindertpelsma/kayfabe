// Each migrated patch is the formatted form of its original mutation.
import { readFileSync } from 'node:fs';
const migrations = JSON.parse(readFileSync(new URL('./mutation-anchors.json', import.meta.url)));
export function applyMutation(campaign, name, source) {
  const patches = migrations[campaign + ':' + name];
  if (!patches) throw Error('Missing migrated mutation: ' + name);
  for (const [before, after] of patches) {
    if (source.split(before).length !== 2)
      throw Error('Missing/ambiguous formatted anchor: ' + name);
    source = source.replace(before, after);
  }
  return source;
}

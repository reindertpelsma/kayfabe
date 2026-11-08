// Restricted, non-generating interpreter for the keywords used by our v1 schemas.
// No eval, I/O, Workers APIs, remote refs or user-supplied schema evaluation.
import contract from './schemas/contract.schema.json' with { type: 'json' };
function freeze(value) {
  if (value && typeof value === 'object') {
    for (const child of Object.values(value)) freeze(child);
    Object.freeze(value);
  }
}
freeze(contract);
export class PlanError extends Error {
  constructor(code, message = code) {
    super(message);
    this.name = 'PlanError';
    this.code = code;
  }
}
export const fail = (code, message) => {
  throw new PlanError(code, message);
};
export const assert = (condition, code, message) => {
  if (!condition) fail(code, message);
};
export const canonical = (v) =>
  JSON.stringify(v, function (k, x) {
    if (x && typeof x === 'object' && !Array.isArray(x))
      return Object.fromEntries(
        Object.keys(x)
          .sort()
          .map((k) => [k, x[k]]),
      );
    return x;
  });
export const compare = (a, b) => (String(a) < String(b) ? -1 : String(a) > String(b) ? 1 : 0);
export const same = (a, b) => canonical(a) === canonical(b);
export function jsonValue(value) {
  const seen = new Set(),
    pending = [[value, 0, false]];
  let count = 0;
  while (pending.length) {
    const [v, depth, leaving] = pending.pop();
    if (leaving) {
      seen.delete(v);
      continue;
    }
    assert(++count <= 150000 && depth <= 64, 'BOUNDS');
    if (v === null || typeof v === 'boolean') continue;
    if (typeof v === 'number') {
      assert(Number.isFinite(v), 'SCHEMA');
      continue;
    }
    if (typeof v === 'string') {
      assert(v.length <= 8192, 'BOUNDS');
      continue;
    }
    assert(
      typeof v === 'object' && !seen.has(v),
      'SCHEMA',
      'JSON value must be finite and acyclic',
    );
    seen.add(v);
    pending.push([v, depth, true]);
    assert(
      Array.isArray(v) || [null, Object.prototype].includes(Object.getPrototypeOf(v)),
      'SCHEMA',
    );
    for (const x of Object.values(v)) pending.push([x, depth + 1, false]);
  }
}
function matches(s, v, path) {
  if (s.$ref) {
    assert(s.$ref.startsWith('#/$defs/'), 'SCHEMA_REF');
    return matches(contract.$defs[s.$ref.slice(8)], v, path);
  }
  if (s.const !== undefined && !same(s.const, v)) return path + ' const';
  if (s.enum && !s.enum.some((x) => same(x, v))) return path + ' enum';
  if (s.anyOf && !s.anyOf.some((x) => !matches(x, v, path))) return path + ' anyOf';
  if (s.oneOf && s.oneOf.filter((x) => !matches(x, v, path)).length !== 1) return path + ' oneOf';
  const t = s.type;
  if (
    (t === 'null' && v !== null) ||
    (t === 'string' && typeof v !== 'string') ||
    (t === 'boolean' && typeof v !== 'boolean') ||
    (t === 'number' && (typeof v !== 'number' || !Number.isFinite(v))) ||
    (t === 'integer' && !Number.isSafeInteger(v)) ||
    (t === 'array' && !Array.isArray(v)) ||
    (t === 'object' && (!v || typeof v !== 'object' || Array.isArray(v)))
  )
    return path + ' type';
  if (
    typeof v === 'number' &&
    ((s.minimum !== undefined && v < s.minimum) || (s.maximum !== undefined && v > s.maximum))
  )
    return path + ' range';
  if (
    typeof v === 'string' &&
    ((s.minLength !== undefined && v.length < s.minLength) ||
      (s.maxLength !== undefined && v.length > s.maxLength) ||
      (s.pattern && !new RegExp(s.pattern).test(v)))
  )
    return path + ' string';
  if (Array.isArray(v)) {
    if (
      (s.minItems !== undefined && v.length < s.minItems) ||
      (s.maxItems !== undefined && v.length > s.maxItems)
    )
      return path + ' length';
    if (s.items)
      for (let i = 0; i < v.length; i++) {
        const e = matches(s.items, v[i], path + '[' + i + ']');
        if (e) return e;
      }
  }
  if (v && typeof v === 'object' && !Array.isArray(v)) {
    if (s.maxProperties !== undefined && Object.keys(v).length > s.maxProperties)
      return path + ' properties';
    for (const k of s.required ?? []) if (!Object.hasOwn(v, k)) return path + '.' + k + ' required';
    for (const [k, x] of Object.entries(v)) {
      const child = s.properties && Object.hasOwn(s.properties, k) ? s.properties[k] : undefined;
      if (!child && s.additionalProperties === false) return path + '.' + k + ' unknown';
      if (child || typeof s.additionalProperties === 'object') {
        const e = matches(child ?? s.additionalProperties, x, path + '.' + k);
        if (e) return e;
      }
    }
  }
  return null;
}
export function checkSchema(name, value) {
  jsonValue(value);
  const error = matches(contract.$defs[name], value, name);
  assert(!error, 'SCHEMA', error);
  return true;
}
export { contract as contractSchema };

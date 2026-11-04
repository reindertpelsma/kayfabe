import { QUERIES } from './queries.js';

// The only path from the Worker to D1: a statement name from QUERIES plus primitive bindings.
// There is no method that takes SQL text.
export class Db {
  constructor(d1) {
    if (!d1 || typeof d1.prepare !== 'function') throw new Error('D1 binding DB missing');
    this.d1 = d1;
  }

  async all(name, params = []) {
    if (!Object.hasOwn(QUERIES, name)) throw new Error('unknown statement');
    for (const value of params) {
      const kind = typeof value;
      if (!(value === null || kind === 'string' || (kind === 'number' && Number.isFinite(value))))
        throw new Error('binding type');
    }
    const result = await this.d1
      .prepare(QUERIES[name])
      .bind(...params)
      .all();
    return Array.isArray(result?.results) ? result.results : [];
  }

  async first(name, params = []) {
    const rows = await this.all(name, params);
    return rows.length ? rows[0] : null;
  }
}

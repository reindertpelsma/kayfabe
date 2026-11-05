// A D1-shaped fake over node:sqlite (Node >= 22.13) with the real migrations of
// ci/platform-api/migrations applied in order. Like D1: foreign keys on, batch() is one
// transaction, first() returns null for no row. Tests skip when node:sqlite is missing.
import { readdirSync, readFileSync } from 'node:fs';

let sqlite = null;
try {
  sqlite = await import('node:sqlite');
} catch {}
export const d1Skip = sqlite ? false : 'node:sqlite unavailable (Node >= 22.13 needed)';

const MIGRATIONS = new URL('../../platform-api/migrations/', import.meta.url);
export const migrations = () =>
  readdirSync(MIGRATIONS)
    .filter((name) => /^[0-9]{4}_[a-z0-9_]+\.sql$/.test(name))
    .sort()
    .map((name) => readFileSync(new URL(name, MIGRATIONS), 'utf8'));

export class FakeD1 {
  constructor({ upTo = Infinity, failOn = null } = {}) {
    this.database = new sqlite.DatabaseSync(':memory:');
    migrations()
      .slice(0, upTo)
      .forEach((sql) => this.database.exec(sql));
    this.log = [];
    this.failOn = failOn;
  }
  prepare(sql) {
    const self = this;
    const statement = (params) => ({
      sql,
      params,
      bind: (...more) => {
        for (const v of more)
          if (v === undefined || typeof v === 'boolean' || (typeof v === 'object' && v !== null))
            throw TypeError('D1 binding type');
        return statement([...params, ...more]);
      },
      async all() {
        self.log.push({ sql, params });
        return {
          success: true,
          results: self.database
            .prepare(sql)
            .all(...params)
            .map((r) => ({ ...r })),
        };
      },
      async first() {
        const { results } = await this.all();
        return results[0] ?? null;
      },
      async run() {
        self.log.push({ sql, params });
        if (self.failOn && self.failOn.test(sql)) throw Error('injected D1 fault');
        const info = self.database.prepare(sql).run(...params);
        return { success: true, meta: { changes: Number(info.changes) } };
      },
    });
    return statement([]);
  }
  async batch(statements) {
    this.database.exec('BEGIN');
    try {
      const out = [];
      for (const s of statements) out.push(await s.run());
      this.database.exec('COMMIT');
      return out;
    } catch (error) {
      this.database.exec('ROLLBACK');
      throw error;
    }
  }
  rows(sql, ...params) {
    return this.database
      .prepare(sql)
      .all(...params)
      .map((r) => ({ ...r }));
  }
}

// Test helpers: a D1-shaped fake over node:sqlite and a seeded database file that the Worker
// side opens READ-ONLY, so any write statement fails in the engine itself.
import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync, readFileSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { syntheticRows, applyRows } from '../scripts/seed-data.mjs';
import { handle } from '../src/api.js';

// Every migration in order, as `wrangler d1 migrations apply` would apply them.
const MIGRATIONS = new URL('../migrations/', import.meta.url);
export const MIGRATION = readdirSync(MIGRATIONS)
  .filter((name) => /^[0-9]{4}_[a-z0-9_]+\.sql$/.test(name))
  .sort()
  .map((name) => readFileSync(new URL(name, MIGRATIONS), 'utf8'))
  .join('\n');

const plain = (row) => (row ? { ...row } : row);

// The subset of the D1 client API the Worker uses: prepare(sql).bind(...).all().
export class FakeD1 {
  constructor(database, log = null) {
    this.database = database;
    this.log = log;
  }
  prepare(sql) {
    const database = this.database;
    const log = this.log;
    const statement = (params) => ({
      bind: (...more) => statement([...params, ...more]),
      async all() {
        if (log) log.push({ sql, params });
        const prepared = database.prepare(sql);
        return { success: true, results: prepared.all(...params).map(plain), meta: {} };
      },
      async first() {
        const { results } = await this.all();
        return results[0] ?? null;
      },
      async run() {
        if (log) log.push({ sql, params });
        database.prepare(sql).run(...params);
        return { success: true, meta: {} };
      },
    });
    return statement([]);
  }
}

// Build the schema and synthetic rows in a temp file, then reopen it read-only.
export function seededDatabase({ rows = syntheticRows(), schemaOnly = false } = {}) {
  const directory = mkdtempSync(join(tmpdir(), 'kf-platform-test-'));
  const path = join(directory, 'platform.sqlite');
  const writer = new DatabaseSync(path);
  writer.exec(MIGRATION);
  if (!schemaOnly) applyRows(writer, rows);
  writer.close();
  return { path, reader: new DatabaseSync(path, { readOnly: true }) };
}

export function api({ database, env = {}, log = null } = {}) {
  const db = database ?? seededDatabase().reader;
  const bound = { DB: new FakeD1(db, log), ...env };
  const call = async (path, init = {}) => {
    const response = await handle(new Request('https://api.test.invalid' + path, init), bound);
    const text = response.status === 302 || response.status === 204 ? '' : await response.text();
    let body = null;
    try {
      body = text ? JSON.parse(text) : null;
    } catch {
      body = text;
    }
    return { response, status: response.status, headers: response.headers, body, text };
  };
  return { call, db };
}

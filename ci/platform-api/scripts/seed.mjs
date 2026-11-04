// Print SYNTHETIC seed SQL for local development:
//   node scripts/seed.mjs > /tmp/seed.sql
//   npx wrangler d1 migrations apply kayfabe-platform --local
//   npx wrangler d1 execute kayfabe-platform --local --file /tmp/seed.sql
// Never run it against the remote (production) database.
import { syntheticRows, toSql } from './seed-data.mjs';

process.stdout.write(toSql(syntheticRows()));

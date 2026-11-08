// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import { readFileSync } from 'node:fs';
import { validateTable, selectDrivers } from './validate.js';
try {
  const table = JSON.parse(readFileSync(process.argv[2] ?? new URL('../driver-availability.json', import.meta.url)));
  validateTable(table);
  if (process.argv[3]) {
    console.log(JSON.stringify(selectDrivers(process.argv[4] ?? '0', JSON.parse(readFileSync(process.argv[3])), table), null, 2));
  } else console.log(`driver availability v${table.schema_version}: ${table.branches.length} branches, ${table.drivers.length} installers; DESIGN-ONLY`);
} catch (e) {
  console.error(e.message); process.exitCode = 1;
}

#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { DEFAULTS } from './src/defaults.js';
import { RELEASE_MATRIX, validateReleaseMatrix } from './src/release-matrix.js';
const value = process.argv[2] ? JSON.parse(readFileSync(process.argv[2], 'utf8')) : RELEASE_MATRIX;
const errors = validateReleaseMatrix(value, DEFAULTS);
for (const error of errors) console.error(error);
console.log(
  JSON.stringify({
    status: errors.length ? 'INVALID' : value.status,
    dies: value.dies?.length,
    errors: errors.length,
  }),
);
process.exitCode = errors.length ? 1 : 0;

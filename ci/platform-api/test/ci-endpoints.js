// Shared by ci.test.js and api.test.js: CI store endpoints over the synthetic seed.
import { syntheticRows } from '../scripts/seed-data.mjs';

export const rows = syntheticRows();
export const finalRun = rows.ci_runs.find((r) => r.verdict === 'PASS');
export const running = rows.ci_runs.find((r) => r.verdict === null);
export const shared = (() => {
  const byTree = new Map();
  for (const r of rows.ci_runs) {
    if (!byTree.has(r.tree_hash)) byTree.set(r.tree_hash, new Set());
    byTree.get(r.tree_hash).add(r.commit_sha);
  }
  return [...byTree].find(([, commits]) => commits.size > 1);
})();

export const CI_ENDPOINTS = () => [
  '/v1/ci/matrix',
  '/v1/ci/matrix?tier=middle&family=ada',
  '/v1/ci/matrix?tree=' + finalRun.tree_hash,
  '/v1/ci/setups',
  '/v1/ci/setups?os=linux&gpu=GA106',
  '/v1/ci/setups?os=windows&gpu=GA106,GA104&cpu=amd&driver=580.95.05&tier=middle&limit=5',
  '/v1/ci/setups?source=reproduced,community&report_tier=unverified',
  '/v1/ci/facets',
  '/v1/ci/trees',
  '/v1/ci/trees?tier=largest&limit=2',
  '/v1/ci/trees/' + finalRun.tree_hash,
  '/v1/ci/runs/' + finalRun.run_id,
  '/v1/ci/runs/' + running.run_id,
  '/v1/ci/commits/' + finalRun.commit_sha,
];

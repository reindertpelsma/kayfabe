import { DEFAULTS } from './defaults.js';
import { hexSha } from './policy.js';
// What the box does not need. Measured 2026-10-26 on a real Vast box: a blobless clone followed
// by a full checkout downloaded every blob of the tree and spent ~15 minutes in the checkout, most
// of it traces/ (~0.5 GB) and archive/. The list starts from everything and removes those two
// directories except for the paths the payload reads: the dependency-cache and payload scripts,
// cargo test inputs of the kf-* crates (traces/ read by include_str! and by tests, the evidence
// paths of crates/kf-rm/data) and the one archived C source scripts/bench/cuda_ladder.sh compiles.
// docs/ stays: the per-die constant tests, the oracle-citation floor and the census read it.
// test/onstart-sparse.test.js fails if a path a box script names is excluded.
const SPARSE = [
  '/*',
  '!/archive/',
  '/archive/nvkvm/tests/mode2/cup2.c',
  '!/traces/',
  '/traces/blackwell_consttab_20261023/generate_rows.py',
  '/traces/blackwell_host_caps_review_20261022/tier_audit/precomp_fields.tsv',
  '/traces/blackwell_provider_validation_20261023/lease_metadata/precomp_tier3.txt',
  '/traces/blackwell_windows_resume_20261023/lease_metadata/precomp_tier3.txt',
  '/traces/c_oracle_census/',
  '/traces/cap1_coldboot_hermetic.rec',
  '/traces/cap1b_coldboot_hermetic_d6.rec',
  '/traces/constants_measure_20261014b/',
  '/traces/constants_night_20261014/',
  '/traces/die_driver_sweep_20261015/',
  '/traces/driver_matrix/',
  '/traces/rawclient_ce_interrupt_20261008/',
  '/traces/real_ad104/',
  '/traces/real_ga104/',
  '/traces/real_ga106/',
  '/traces/real_hw_20261013/',
  '/traces/real_tu116/',
  '/traces/user_ring_scan_20261014/',
];
// The provider's onstart: clone, write the sparse list, check out the exact tested commit, exec the
// repo-side script. Only the two interpolated values are inputs; per-run data travels in the KF_*
// environment. The list is written to .git/info/sparse-checkout rather than through
// `git sparse-checkout set`, because the box image's git (2.34) has no `--no-cone` and `set` would
// check out the clone's default branch first. It must exist before the fetch and the checkout.
export function onstart(sha, repo) {
  if (!hexSha(sha) || !/^[-\w.]+\/[-\w.]+$/.test(repo)) throw Error('Invalid fixed onstart inputs');
  return `#!/bin/bash
set -eu
git clone --filter=blob:none --no-checkout https://github.com/${repo}.git /workspace/kf-ci-src
cd /workspace/kf-ci-src
git config core.sparseCheckout true
git config core.sparseCheckoutCone false
mkdir -p .git/info
cat >.git/info/sparse-checkout <<'KF_SPARSE'
${SPARSE.join('\n')}
KF_SPARSE
git fetch --depth=1 origin ${sha}
git checkout --detach ${sha}
[ "$(git rev-parse HEAD)" = '${sha}' ]
exec bash ${DEFAULTS.onstartPath}
`;
}

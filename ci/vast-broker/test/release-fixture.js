// Synthetic executable policy for admission/verdict tests. Production stays
// proposed, with unknown minima and no executor capability. Never mutate it at
// module-import time: test/index.js imports all probes into a shared process.
import nodeTest from 'node:test';
import { readFileSync } from 'node:fs';
import { RELEASE_MATRIX } from '../src/release-matrix.js';
export const proposedMatrix = JSON.parse(
  readFileSync(new URL('../release-matrix.json', import.meta.url)),
);
export function executableMatrix() {
  const copy = structuredClone(proposedMatrix);
  copy.status = 'executable';
  // Synthetic pins permit mock execution; these never replace proposal data on disk.
  for (const pin of copy.driver_installers)
    if (pin.status === 'UNPINNED') {
      pin.status = 'PINNED';
      pin.sha256 = 'a'.repeat(64);
      pin.size = 42;
      pin.source = 'SYNTHETIC test installer, never fetched or used for hardware';
    }

  for (const row of copy.dies) {
    row.status = 'executable';
    row.blockers = [];
    row.windows_guest_abi = '580.65.06';
    if (row.cross_subrun) row.cross_subrun.windows_guest_abi = '580.65.06';
    row.minimum_drivers = {
      host_driver: '580.65.06',
      linux_guest_driver: '580.65.06',
      windows_guest_driver: '580.88',
    };
  }
  return copy;
}
const replace = (target, value) => {
  for (const key of Object.keys(target)) delete target[key];
  Object.assign(target, structuredClone(value));
};
export function matrixTest(name, options, body) {
  if (typeof options === 'function') {
    body = options;
    options = {};
  }
  return nodeTest(name, options, async (t) => {
    const saved = structuredClone(RELEASE_MATRIX);
    replace(RELEASE_MATRIX, executableMatrix());
    try {
      await body(t);
    } finally {
      replace(RELEASE_MATRIX, saved);
    }
  });
}
export function capablePayload(h) {
  h.cloud.reporterText = 'BOX_PROTOCOL = "7"\nBOX_RELEASE_MATRIX_RECEIPTS = 1\n';
  h.cloud.payloadText = 'PAYLOAD_RELEASE_MATRIX_EXECUTOR = 1\n';
  return h;
}
// Build host-reported fixtures from policy data, independently of the production
// receipt-definition helper. Negative tests remove or change individual facts.
export function receipts(row) {
  const rows = [
    {
      id: 'full',
      drivers: { ...row.drivers },
      windows_guest_abi: row.windows_guest_abi,
      verdict: 'PASS',
    },
  ];
  for (const run of row.subruns)
    for (const driver of run.drivers) {
      const axis = run.os === 'linux' ? 'linux_guest_driver' : 'windows_guest_driver';
      rows.push({
        id: run.os + '_subrun_' + driver.split('.').join('_'),
        drivers: { ...row.drivers, [axis]: driver },
        windows_guest_abi: row.windows_guest_abi,
        verdict: 'PASS',
      });
    }
  if (row.cross_subrun)
    for (const os of ['linux', 'windows'])
      rows.push({
        id: os + '_cross',
        drivers: { ...row.cross_subrun.drivers },
        windows_guest_abi: row.cross_subrun.windows_guest_abi,
        verdict: 'PASS',
      });
  return { schema_version: 1, die: row.die, rows };
}
export function receiptParts(row) {
  return Object.fromEntries(receipts(row).rows.map((r) => [r.id, 'PASS']));
}
export function subrunSuites(row) {
  const names = new Set(row.subruns.map((run) => run.os + '_subrun_' + run.driver_family));
  if (row.cross_subrun) {
    names.add('linux_cross');
    names.add('windows_cross');
  }
  return [...names].map((name) => ({ name, pass: 1, fail: 0, skip: 0 }));
}

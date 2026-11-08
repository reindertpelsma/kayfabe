// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// Offline planning only. No Worker integration, I/O, clock, or random source.
const FAMILIES = ['turing', 'ampere', 'ada', 'blackwell'];
const OSES = ['linux', 'windows'];
const STATUSES = ['PINNED', 'UNPINNED:needs-download', 'UNPINNED:no-sha256', 'UNAVAILABLE', 'UNKNOWN:probe-error'];
const insist = (condition, message) => { if (!condition) throw new Error(message); };
const nonempty = v => typeof v === 'string' && v.trim().length > 0;
const array = (v, label) => insist(Array.isArray(v), `${label}: expected array`);
const object = (v, label) => insist(v && typeof v === 'object' && !Array.isArray(v), `${label}: expected object`);
const compare = (a, b) => a < b ? -1 : a > b ? 1 : 0;

export function exactVersionKey(version) {
  insist(typeof version === 'string' && /^\d{3}\.\d{1,3}(?:\.\d{1,3})?$/.test(version), 'invalid exact version');
  const parts = version.split('.').map(Number);
  return [parts[0], parts[1], parts[2] ?? 0].join('.');
}
function date(value) {
  return typeof value === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(value)
    && Number.isFinite(Date.parse(value)) && new Date(value).toISOString().slice(0, 10) === value;
}
function instant(value) {
  return typeof value === 'string' && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(value)
    && date(value.slice(0, 10)) && Number.isFinite(Date.parse(value));
}
function official(raw, download = false) {
  try {
    const u = new URL(raw);
    const hosts = download
      ? u.hostname === 'download.nvidia.com' || u.hostname.endsWith('.download.nvidia.com')
      : u.hostname === 'nvidia.com' || u.hostname.endsWith('.nvidia.com') || u.hostname === 'gfwsl.geforce.com';
    return u.protocol === 'https:' && hosts && !u.username && !u.password && !u.port && !u.hash;
  } catch { return false; }
}
function record(r) {
  object(r, 'test record');
  insist(/^[0-9a-f]{8,40}$/.test(r.revision) && nonempty(r.trace) && date(r.date)
    && nonempty(r.scope) && ['pass', 'referenced-only'].includes(r.result), 'invalid test record');
}

export function validateTable(t) {
  object(t, 'table');
  insist(t.schema_version === 1, 'unsupported schema_version');
  insist(t.status === 'DESIGN-ONLY' && instant(t.generated_at), 'invalid catalog status/date');
  object(t.families, 'families');
  for (const f of FAMILIES) {
    const row = t.families[f]; object(row, `family ${f}`);
    insist(row.minimum_supported_branch === 'UNKNOWN' || Number.isSafeInteger(row.minimum_supported_branch), 'invalid family minimum');
    insist(Number.isSafeInteger(row.sweep_floor) && row.sweep_floor >= 535
      && (f !== 'blackwell' || row.sweep_floor >= 570), 'invalid scoped family floor');
    insist(official(row.source_url) && instant(row.checked_at) && nonempty(row.evidence), 'missing family evidence');
  }
  array(t.branches, 'branches'); insist(t.branches.length > 0, 'no branches');
  const branches = new Set();
  for (const b of t.branches) {
    insist(Number.isSafeInteger(b.id) && b.id >= 535 && b.name === `R${b.id}` && !branches.has(b.id), 'invalid/duplicate branch');
    branches.add(b.id);
    insist(['production', 'new-feature', 'LTS', 'UNKNOWN'].includes(b.branch_type) && official(b.source_url), 'invalid branch type/source');
    for (const range of [b.first_version, b.last_version]) {
      object(range, 'branch version range');
      for (const os of OSES) if (range[os] !== null) exactVersionKey(range[os]);
    }
    insist(['known-good', 'referenced-only', 'never-tested'].includes(b.repo_status), 'invalid repo_status');
    array(b.test_records, 'branch test_records'); b.test_records.forEach(record);
    if (b.repo_status === 'known-good') insist(b.test_records.some(r => r.result === 'pass'), 'known-good needs revision + trace');
  }
  object(t.abi_boundary_hypothesis, 'ABI hypothesis');
  insist(t.abi_boundary_hypothesis.status === 'HYPOTHESIS' && nonempty(t.abi_boundary_hypothesis.falsifier)
    && t.abi_boundary_hypothesis.used_for_selection === false, 'ABI hypothesis cannot prune the sweep');
  array(t.drivers, 'drivers');
  const keys = new Set();
  for (const d of t.drivers) {
    object(d, 'driver');
    insist(OSES.includes(d.os) && branches.has(d.branch), 'invalid OS/branch');
    const key = `${d.os}:${exactVersionKey(d.version)}`;
    const major = Number(d.version.split('.')[0]);
    if (d.os === 'linux') insist(major === d.branch, 'version in wrong branch');
    else {
      object(d.branch_evidence, 'Windows branch evidence');
      insist(d.branch_evidence.branch === d.branch && d.branch_evidence.version === d.version
        && official(d.branch_evidence.source_url), 'Windows branch needs explicit official evidence');
    }
    insist(!keys.has(key), 'duplicate normalized OS/version'); keys.add(key);
    insist(official(d.url, true), 'installer must have official HTTPS URL');
    const path = new URL(d.url).pathname;
    insist(d.os === 'linux'
      ? path.endsWith(`/NVIDIA-Linux-x86_64-${d.version}.run`)
      : path.endsWith('whql.exe') && path.includes(`/${d.version}/`) && path.includes('-desktop-'), 'wrong installer identity');
    insist(STATUSES.includes(d.status), 'invalid pin status');
    insist(d.size === null || Number.isSafeInteger(d.size) && d.size > 0, 'invalid size');
    insist(d.sha256 === null || /^[0-9a-f]{64}$/.test(d.sha256), 'invalid sha256');
    insist(d.release_date === null || date(d.release_date), 'invalid release date');
    insist(d.last_tested === null || date(d.last_tested) && d.last_tested <= t.generated_at.slice(0, 10), 'invalid last_tested');
    array(d.test_records, 'driver test_records'); d.test_records.forEach(record);
    array(d.discovery_source_urls, 'discovery sources');
    insist(d.discovery_source_urls.length > 0 && d.discovery_source_urls.every(u => official(u)), 'invalid discovery source');
    object(d.probe, 'probe');
    insist(d.probe.method === 'HEAD' && instant(d.probe.checked_at)
      && d.probe.checked_at <= t.generated_at && (d.probe.status === null
        || Number.isSafeInteger(d.probe.status) && d.probe.status >= 100 && d.probe.status <= 599), 'invalid HEAD probe');
    insist(d.probe.final_url === null || official(d.probe.final_url, true), 'unofficial redirect');
    if (d.status === 'PINNED') {
      insist(d.probe.status === 200 && d.probe.final_url !== null && d.size !== null
        && d.sha256 !== null, 'PINNED needs HEAD 200 + sha256 + size');
      object(d.checksum_source, 'checksum provenance');
      insist(['nvidia-sha256', 'download-sha256'].includes(d.checksum_source.kind)
        && official(d.checksum_source.url, true) && instant(d.checksum_source.checked_at)
        && d.checksum_source.checked_at <= t.generated_at, 'invalid checksum provenance');
    } else insist(d.sha256 === null, 'an unpinned record cannot carry an active pin');
    if (d.status === 'UNAVAILABLE') insist(d.probe.status === 404, 'only 404 proves absent');
    if (d.status.startsWith('UNPINNED:')) insist(d.probe.status === 200, 'unpinned must be available');
  }
  array(t.known_bad, 'known_bad');
  for (const r of t.known_bad) {
    insist((r.os === '*' || OSES.includes(r.os)) && nonempty(r.reason), 'invalid known_bad'); exactVersionKey(r.version);
  }
  array(t.core_reference_versions, 'core_reference_versions');
  for (const r of t.core_reference_versions) {
    insist(FAMILIES.includes(r.family) && OSES.includes(r.os)
      && keys.has(`${r.os}:${exactVersionKey(r.version)}`), 'invalid core reference');
  }
  return true;
}

// FNV-1a over explicit UTF-16 code units: stable across platforms and processes.
function rank(seed, slot, d) {
  let h = 2166136261;
  const s = JSON.stringify([seed, slot.box.id, slot.box.family, slot.box.role, slot.branch, slot.axis, d.os, exactVersionKey(d.version)]);
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619) >>> 0;
  return h;
}
function banned(d, t) {
  return t.known_bad.some(r => (r.os === '*' || r.os === d.os) && exactVersionKey(r.version) === exactVersionKey(d.version));
}
function share(a, b, d, t) {
  return a.box.id !== b.box.id && a.box.family === b.box.family && a.os === b.os
    && a.box.role === 'core-reference' && b.box.role === 'core-reference'
    && t.core_reference_versions.some(r => r.family === a.box.family && r.os === d.os
      && exactVersionKey(r.version) === exactVersionKey(d.version));
}

export function selectDrivers(seed, boxes, t) {
  validateTable(t);
  insist(typeof seed === 'string' || Number.isSafeInteger(seed), 'seed must be a string or safe integer');
  seed = String(seed);
  array(boxes, 'boxes'); insist(boxes.length > 0, 'empty boxes');
  const seen = new Set();
  for (const b of boxes) {
    insist(nonempty(b.id) && FAMILIES.includes(b.family) && nonempty(b.role) && !seen.has(b.id), 'invalid/duplicate box');
    insist(b.product_line === undefined || ['geforce', 'rtx-enterprise'].includes(b.product_line), 'invalid product_line');
    seen.add(b.id);
  }
  const sortedBoxes = [...boxes].sort((a, b) => compare(a.id, b.id));
  const result = sortedBoxes.map(b => ({ ...b, runs: [], excluded_branches: [] }));
  const branches = [...t.branches].sort((a, b) => a.id - b.id);
  for (const b of result) {
    for (const branch of branches) {
      if (branch.id < t.families[b.family].sweep_floor) b.excluded_branches.push({ branch: branch.id, reason: 'BELOW_FAMILY_FLOOR' });
      else b.runs.push({ branch: branch.id, drivers: {} });
    }
    insist(b.runs.length > 0, `NO_SUPPORTED_BRANCHES family=${b.family}`);
  }
  const assigned = new Map(); // One ownership ledger for the entire release.
  for (const branch of branches) {
    const slots = [];
    for (const b of result) {
      const run = b.runs.find(r => r.branch === branch.id);
      if (!run) continue;
      for (const [axis, os] of [['host_driver', 'linux'], ['linux_guest_driver', 'linux'], ['windows_guest_driver', 'windows']]) {
        const slot = { box: b, branch: branch.id, axis, os, run };
        slot.candidates = t.drivers.filter(d => d.branch === branch.id && d.os === os
          && (os !== 'windows' || (d.product_line ?? 'geforce') === (b.product_line ?? 'geforce'))
          && d.status === 'PINNED' && d.probe.status === 200 && !banned(d, t));
        slot.candidates.sort((a, b) => compare(a.last_tested ?? '', b.last_tested ?? '')
          || rank(seed, slot, a) - rank(seed, slot, b) || compare(exactVersionKey(a.version), exactVersionKey(b.version)));
        insist(slot.candidates.length > 0, `INSUFFICIENT_PINNED_VERSIONS branch=${branch.id} os=${os} box=${b.id}`);
        slots.push(slot);
      }
    }
    // Augmenting assignment, with rollback for the explicitly shareable core pins.
    // A later restricted OS slot can displace an earlier choice instead of causing
    // a false exhaustion. Owners are global across OS axes (no OS-prefixed key).
    function place(slot, visited) {
      if (visited.has(slot)) return false;
      const nextVisited = new Set([...visited, slot]);
      for (const d of slot.candidates) {
        const snapshot = new Map(assigned);
        const key = exactVersionKey(d.version);
        const conflicts = [...assigned].filter(([owner, chosen]) => owner !== slot
          && exactVersionKey(chosen.version) === key && !share(slot, owner, d, t)).map(([owner]) => owner);
        assigned.set(slot, d);
        let ok = true;
        for (const owner of conflicts) {
          assigned.delete(owner);
          if (!place(owner, nextVisited)) { ok = false; break; }
        }
        if (ok) return true;
        assigned.clear(); snapshot.forEach((chosen, owner) => assigned.set(owner, chosen));
      }
      return false;
    }
    for (const slot of [...slots].sort((a, b) => a.candidates.length - b.candidates.length)) {
      insist(place(slot, new Set()), `INSUFFICIENT_PINNED_VERSIONS branch=${branch.id} os=${slot.os} box=${slot.box.id}`);
    }
    for (const [slot, d] of assigned) slot.run.drivers[slot.axis] = {
      os: d.os, version: d.version, url: d.url, sha256: d.sha256, size: d.size,
    };
  }
  // Canonical axis order also keeps JSON output independent of matching history.
  for (const b of result) for (const r of b.runs) r.drivers = Object.fromEntries(
    ['host_driver', 'linux_guest_driver', 'windows_guest_driver'].map(axis => [axis, r.drivers[axis]]));
  return result;
}

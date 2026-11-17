// The expected download and upload of ONE leg, computed from its instruction (docs/design/V3_WORKER_PROPIMAGES.md, "Bandwidth").
// STATUS: DESIGN-ONLY, 2026-11-16. Pure and tested; nothing in src/index.js calls it, so no rental price changes.
//
// Every number lives in ci/vast-broker/bandwidth.json (per-stage download table, margin, repo size, upload estimates); there is no
// data number in this file. Pinned object sizes are read from propimages.json (the private images) and release-matrix.json (the
// NVIDIA installers), never copied into the table. A box pays per GB whatever the source, so a private R2 object, an official URL, a
// package mirror and the repo clone are counted the same way.
//
//   estimateBandwidth(instruction, tier, options) -> { down_bytes, up_bytes, down_gb, up_gb, margin_percent, stages, items, lines }
//
// options: { table, propimages, matrix }   the data files (default: the committed ones)
//          { r2Sizes }                      { objectName: bytes } filled by the Worker from an R2 list, for `r2-listed` entries
//          { repoBytes }                    the repo size at the sha (for example the GitHub API value), replaces the repo entry
//          { marginPercent }                replaces the table's margin
//          { only, skip }                   the payload's --only / --skip, so a manual leg is counted as it runs
import bandwidthTable from '../bandwidth.json' with { type: 'json' };
import { PROPIMAGES } from './propimages.js';
import { RELEASE_MATRIX } from './release-matrix.js';

export const BANDWIDTH = bandwidthTable;
const GB = 1e9; // decimal gigabytes, the unit of the profile table in defaults.json
const PERCENT = 100;
const isObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
const count = (n) => Number.isSafeInteger(n) && n >= 0;

// The stages the payload plans for an instruction, in run order: the same rule as plan_stage_names in scripts/ci/vast_payload.py
// (test/bandwidth.test.js compares the two). `only` is the payload's --only list: it is what puts the manual-only stage
// `install_flow` (and `propimage_apps` without an image list, and `matlab` without the instruction's object) into a large plan.
export function plannedStages(instruction, { only = [] } = {}) {
  const i = instruction;
  const names = ['provision', 'cuda_host', 'merge_bar', 'cuda_guest'];
  if (i.linux_apps !== null && i.linux_apps !== undefined) names.push('apps');
  if (i.tier === 'large' && only.includes('install_flow')) names.push('install_flow');
  if (i.windows) {
    names.push('windows_gate');
    if (i.winapps) names.push('windows_apps');
    if (i.windows_runs === 2) names.push('windows_gate_2');
  }
  if ((i.propimages ?? []).length > 0 || (i.tier === 'large' && only.includes('propimage_apps')))
    names.push('propimage_apps');
  if (i.tier === 'large' && ((i.matlab ?? null) !== null || only.includes('matlab')))
    names.push('matlab');
  if ((i.stages ?? []).includes('specviewperf')) names.push('specviewperf');
  return names;
}

const matches = (when, ctx) =>
  when === undefined ||
  Object.entries(when).every(([key, list]) => {
    if (key === 'images') return ctx.images.some((label) => list.includes(label));
    return list.includes(ctx[key]);
  });

// The bytes of a `size_from` reference. `propimage:image` (with per_image) is the image object of one selected image,
// `propimage:<id>` another object of propimages.json, `driver:<os>/<version>` an installer of release-matrix.json, `r2:<name>` a size
// from the Worker's R2 list (the entry's own down_bytes is the fallback).
function sizeOf(entry, label, { propimages, matrix, r2Sizes }) {
  const ref = entry.size_from;
  if (ref === undefined) return { bytes: entry.down_bytes, from: 'table' };
  const [kind, name] = [ref.slice(0, ref.indexOf(':')), ref.slice(ref.indexOf(':') + 1)];
  if (kind === 'propimage') {
    const objects = new Map(propimages.objects.map((o) => [o.id, o]));
    const id =
      name === 'image' ? propimages.images.find((img) => img.label === label)?.object : name;
    const object = objects.get(id);
    if (object === undefined) throw Error('bandwidth size_from:' + entry.id);
    return { bytes: object.size, from: 'propimages.json:' + id };
  }
  if (kind === 'driver') {
    const pin = (matrix.driver_installers ?? []).find((p) => p.os + '/' + p.version === name);
    if (!pin || !count(pin.size)) throw Error('bandwidth size_from:' + entry.id);
    return { bytes: pin.size, from: 'release-matrix.json:' + name };
  }
  if (kind === 'r2') {
    if (count(r2Sizes[name])) return { bytes: r2Sizes[name], from: 'r2 list' };
    if (!count(entry.down_bytes)) throw Error('bandwidth size_from:' + entry.id);
    return { bytes: entry.down_bytes, from: 'table (not in the r2 list)' };
  }
  throw Error('bandwidth size_from:' + entry.id);
}

export function estimateBandwidth(instruction, tier, options = {}) {
  const {
    table = BANDWIDTH,
    propimages = PROPIMAGES,
    matrix = RELEASE_MATRIX,
    r2Sizes = {},
    repoBytes,
    marginPercent,
    only = [],
    skip = [],
  } = options;
  if (!isObject(instruction)) throw Error('bandwidth instruction');
  const plan = plannedStages(instruction, { only });
  for (const name of [...only, ...skip])
    if (!plan.includes(name)) throw Error('bandwidth stage:' + name);
  const chosen = plan.filter((n) => (!only.length || only.includes(n)) && !skip.includes(n));
  for (const name of chosen)
    if (!isObject(table.stages[name])) throw Error('bandwidth table stage:' + name);
  const sent = (instruction.propimages ?? []).map((img) => img.label);
  // the manual stage with no image list is the one pinned image the old stage ran
  const images = sent.length ? sent : chosen.includes('propimage_apps') ? table.default_images : [];
  const ctx = {
    tier: tier ?? instruction.tier,
    winapps: instruction.winapps,
    spec: instruction.spec,
    linux_apps: instruction.linux_apps,
    images,
  };
  const env = { propimages, matrix, r2Sizes };
  const items = [];
  const add = (stage, entry) => {
    if (!matches(entry.when, ctx)) return;
    for (const label of entry.per_image ? images : [undefined]) {
      let { bytes, from } = sizeOf(entry, label, env);
      if (entry.id === 'fixed.repo_clone' && count(repoBytes))
        [bytes, from] = [repoBytes, 'passed in'];
      items.push({
        id: entry.id,
        stage,
        label,
        what: entry.what,
        class: entry.class,
        basis: entry.basis,
        down: bytes,
        up: 0,
        from,
      });
    }
  };
  for (const entry of table.fixed) add('fixed', entry);
  for (const name of chosen) {
    for (const entry of table.stages[name].entries) add(name, entry);
    const upload = table.stages[name].upload;
    items.push({
      id: name + '.upload',
      stage: name,
      what: 'evidence pull-back',
      class: 'upload',
      basis: upload.basis,
      down: 0,
      up: upload.up_bytes,
      from: 'table',
    });
  }
  const margin = marginPercent ?? table.margin.percent;
  if (typeof margin !== 'number' || !Number.isFinite(margin) || margin < 0)
    throw Error('bandwidth margin');
  const sum = (key) => items.reduce((n, item) => n + item[key], 0);
  const scaled = (n) => Math.ceil((n * (PERCENT + margin)) / PERCENT);
  const gb = (n) => Math.ceil((n * PERCENT) / GB) / PERCENT;
  const down_bytes = scaled(sum('down'));
  const up_bytes = scaled(sum('up'));
  const out = {
    down_bytes,
    up_bytes,
    down_gb: gb(down_bytes),
    up_gb: gb(up_bytes),
    margin_percent: margin,
    stages: chosen,
    images,
    items,
  };
  out.lines = breakdown(out, ctx.tier);
  return out;
}

// The text the Worker puts in the leg's log: what the number is made of, one data-file entry per line (the id is the id in
// bandwidth.json, so a surprise in a bill maps to the one entry to edit).
export function breakdown(estimate, tier) {
  const fmt = (n) => (n / GB).toFixed(3).padStart(8) + ' GB';
  const lines = [
    'bandwidth estimate (ci/vast-broker/bandwidth.json) tier=' +
      tier +
      ' stages=' +
      estimate.stages.join(':') +
      (estimate.images.length ? ' images=' + estimate.images.join(',') : ''),
  ];
  for (const it of estimate.items)
    lines.push(
      (it.down ? fmt(it.down) + ' down ' : fmt(it.up) + ' up   ') +
        '[' +
        it.basis +
        ' ' +
        it.class +
        '] ' +
        it.id +
        (it.label ? ' (' + it.label + ')' : '') +
        ': ' +
        it.what +
        (it.from === 'table' ? '' : ' <' + it.from + '>'),
    );
  const raw = (key) => estimate.items.reduce((n, item) => n + item[key], 0);
  lines.push(
    fmt(raw('down')) + ' down  sum, ' + fmt(raw('up')) + ' up  sum',
    '+' +
      estimate.margin_percent +
      ' percent margin: ' +
      estimate.down_gb.toFixed(2) +
      ' GB down, ' +
      estimate.up_gb.toFixed(2) +
      ' GB up',
  );
  return lines;
}

// What to hand the Worker's pricing today: policy.js resourceCaps takes max(profile number, override), so these overrides can only
// RAISE a leg's bandwidth above its profile row; a leg whose estimate is lower still pays the profile number until resourceCaps
// takes the estimate as the number (the one follow-up edit, which is a policy.js change and so a bundle pin; not made here).
export const bandwidthOverrides = (estimate) => ({
  bandwidthDownOverride: Math.ceil(estimate.down_gb),
  bandwidthUpOverride: Math.ceil(estimate.up_gb),
});

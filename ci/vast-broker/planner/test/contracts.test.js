import test from 'node:test';
import assert from 'node:assert/strict';
import {
  plan,
  validateInput,
  validatePlan,
  validateAssignment,
  checkSchema,
  PlanError,
} from '../index.js';
import { fixture, clone } from './helpers.js';
const rejects = (fn) => assert.throws(fn, (e) => e instanceof PlanError);
for (const part of ['catalog', 'requirements', 'limits'])
  for (const key of ['unknown', 'version'])
    test('closed versioned ' + part + ' rejects ' + key, () => {
      const f = fixture();
      if (key === 'version') f[part].version = 2;
      else f[part].unknown = true;
      rejects(() => plan(f));
    });
for (const part of ['catalog', 'requirements', 'limits'])
  for (const n of [1, 2])
    test('proposed ' + part + ' cannot execute ' + n, () => {
      const f = fixture();
      f[part].status = 'proposed';
      if (n === 2) f.requirements.items[0].runs = ['merge_bar'];
      const p = plan(f);
      assert.equal(p.admitted, false);
      rejects(() => validatePlan(f, p));
    });
for (const selected of [false, true])
  test('unpinned catalog entry blocks execution even if unused ' + selected, () => {
    const f = fixture();
    const id = f.requirements.items[0].driver.host;
    const p = f.catalog.driver_pins.find((p) =>
      selected ? p.id === id : p.id !== id && p.os === 'linux',
    );
    p.status = 'UNPINNED';
    p.sha256 = null;
    p.size = null;
    const out = plan(f);
    assert.equal(out.admitted, false);
    rejects(() => validatePlan(f, out));
    validatePlan(f, out, { forExecution: false });
  });
for (const url of [
  'http://download.nvidia.com/XFree86/Linux-x86_64/580.159.04/NVIDIA-Linux-x86_64-580.159.04.run',
  'https://download.nvidia.com.evil.invalid/XFree86/Linux-x86_64/580.159.04/NVIDIA-Linux-x86_64-580.159.04.run',
  'https://download.nvidia.com:8443/XFree86/Linux-x86_64/580.159.04/NVIDIA-Linux-x86_64-580.159.04.run',
  'https://download.nvidia.com/XFree86/Linux-x86_64/580.159.04/NVIDIA-Linux-x86_64-580.159.04.run?mirror=1',
])
  test('official installer URL rejects ' + url, () => {
    const f = fixture();
    f.catalog.driver_pins.find((p) => p.version === '580.159.04').url = url;
    rejects(() => validateInput(f));
  });
for (const field of ['sha256', 'size'])
  for (const n of [0, 1])
    test('pinned installer needs digest and size ' + field + ' ' + n, () => {
      const f = fixture();
      const p = f.catalog.driver_pins[0];
      p[field] = field === 'size' ? (n ? null : 0) : n ? null : '1234';
      rejects(() => plan(f));
    });
for (const axis of ['host', 'windows_guest'])
  test('branch sweep cannot claim another branch ' + axis, () => {
    const f = fixture(),
      r = f.catalog.runs.find(
        (r) => r.id === (axis === 'host' ? 'host_tiny' : 'guest_windows_tiny'),
      );
    r.driver_axes = [axis];
    const s = f.catalog.driver_sets[0];
    s.id = '590';
    s.branch = 590;
    const q = f.requirements.items[0];
    delete q.driver;
    q.driver_sweep = { branches: ['590'] };
    q.runs = [r.id];
    q.evidence = 'driver';
    rejects(() => plan(f));
  });
for (const mode of ['kvm', 'vfio'])
  for (const n of [1, 2])
    test('guest capabilities remain mandatory ' + mode + ' ' + n, () => {
      const f = fixture();
      f.boxes.forEach((b) => (b.capabilities[mode] = false));
      f.requirements.items[0].runs = Array(n).fill('windows_gate');
      const p = plan(f);
      assert.equal(p.admitted, false);
      assert.equal(p.assignments.length, 0);
    });
for (const n of [1, 2])
  test('fixed host cannot swap driver ' + n, () => {
    const f = fixture();
    f.boxes.forEach((b) => (b.capabilities.can_swap_host_driver = false));
    f.requirements.items[0].runs = Array(n).fill('host_tiny');
    f.requirements.items[0].evidence = 'driver';
    const p = plan(f);
    assert.equal(p.admitted, false);
    assert.equal(p.assignments.length, 0);
  });
for (const n of [2, 3])
  test('same physical machine cannot appear as multiple rentals ' + n, () => {
    const f = fixture();
    f.boxes = f.boxes.filter((b) => b.die === 'TU106');
    f.boxes.forEach((b) => (b.machine_id = 'only-machine'));
    f.requirements.items[0].runs = Array(n).fill('spec_lane_a');
    const p = plan(f);
    assert.equal(p.admitted, false);
    assert.equal(p.assignments.length, 1);
    validatePlan(f, p, { forExecution: false });
  });
for (const kind of ['cycle', 'missing'])
  for (const n of [1, 2])
    test('catalog dependency ' + kind + ' ' + n, () => {
      const f = fixture(),
        r = f.catalog.runs.find((r) => r.id === 'provision');
      r.needs = kind === 'cycle' ? ['kf3_bins'] : ['missing-' + n];
      rejects(() => plan(f));
    });
for (const n of [1, 2])
  test('one shared producer receives only one inclusive setup credit ' + n, () => {
    const f = fixture();
    f.catalog.runs.find((r) => r.id === 'linux_image_build').produces.push('image-sidecar');
    f.catalog.artifacts.push({
      id: 'image-sidecar',
      producer: 'linux_image_build',
      gb: 0,
      scope: 'box',
      seedable: false,
    });
    const r = f.catalog.runs.find((r) => r.id === 'linux_apps_full');
    r.needs.push('image-sidecar');
    r.estimated_minutes = 50;
    r.includes_setup = n === 1 ? ['open_image', 'image-sidecar'] : ['image-sidecar', 'open_image'];
    f.requirements.items[0].runs = ['linux_apps_full'];
    const p = plan(f);
    assert.equal(p.assignments[0].fallback_minutes, 80);
    assert.equal(p.assignments[0].runs.find((r) => r.run_id === 'linux_apps_full').minutes, 20);
  });
for (const n of [1, 2])
  test('assignment cannot bypass trusted machine or quote ' + n, () => {
    const f = fixture(),
      p = plan(f),
      a = clone(p.assignments[0]);
    if (n === 1) a.machine_id = 'foreign';
    else a.pricing.reserved_usd = 0;
    rejects(() => validateAssignment(f, a));
  });
for (const n of [0, 1])
  test('malformed candidate aliases remain rejected ' + n, () => {
    const f = fixture();
    f.boxes[0].pricing.inet_up_usd_gb = n ? null : Number.NaN;
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.every((a) => a.box_id !== 'offer-0'));
  });
for (const n of [0, 1])
  test('conflicting offer ids rejected without object-order influence ' + n, () => {
    const f = fixture();
    const b = clone(f.boxes[0]);
    b.pricing.compute_usd_hour = 0;
    f.boxes.push(b);
    if (n) f.boxes.reverse();
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.every((a) => a.box_id !== 'offer-0'));
  });
// FALSIFIER: the published hourly/transfer fields imply a larger bill than
// the plan reserved; persisted ledger accounting would then exceed that quote.
for (const storage of [0.08, 0.2])
  test('reservation covers its own published rounded prices ' + storage, () => {
    const f = fixture();
    f.boxes.forEach((b) => (b.pricing.storage_usd_gb_month = storage));
    const p = plan(f);
    for (const a of p.assignments)
      assert.ok(
        a.pricing.reserved_usd + 1e-10 >=
          (a.pricing.hourly_usd * a.reservation_minutes) / 60 + a.pricing.transfer_usd,
      );
  });
for (const n of [1, 2])
  test('swappable provider need not claim a pinned current host ' + n, () => {
    const f = fixture();
    f.boxes.forEach((b) => (b.host_driver = null));
    if (n === 2) f.requirements.items[0].runs = ['merge_bar'];
    const p = plan(f);
    assert.equal(p.admitted, true);
    validatePlan(f, p);
  });
for (const n of [1, 2])
  test('fixed unknown host cannot claim target driver ' + n, () => {
    const f = fixture();
    f.boxes.forEach((b) => {
      b.host_driver = null;
      b.capabilities.can_swap_host_driver = false;
    });
    if (n === 2) f.requirements.items[0].runs = ['merge_bar'];
    const p = plan(f);
    assert.equal(p.admitted, false);
    assert.equal(p.assignments.length, 0);
  });
for (const variant of [0.06, 0.065])
  test('network-inclusive ranking flips with tier ' + variant, () => {
    const f = fixture();
    f.boxes = f.boxes.slice(0, 2);
    f.boxes[0].pricing.compute_usd_hour = 0.1;
    f.boxes[0].pricing.inet_down_usd_gb = 0.004;
    f.boxes[1].pricing.compute_usd_hour = 0.1 + variant;
    f.boxes[1].pricing.inet_down_usd_gb = 0.001;
    f.requirements.items[0].runs = ['merge_bar'];
    assert.equal(plan(f).assignments[0].box_id, 'offer-1');
    f.requirements.items[0].tier = 'small';
    assert.equal(plan(f).assignments[0].box_id, 'offer-0');
  });
for (const rate of [0.72, 0.8])
  test('profile storage affects ranking ' + rate, () => {
    const f = fixture();
    f.boxes = f.boxes.slice(0, 2);
    f.boxes.forEach((b) => {
      b.pricing.inet_down_usd_gb = 0;
      b.pricing.inet_up_usd_gb = 0;
    });
    f.boxes[0].pricing.compute_usd_hour = 0.1;
    f.boxes[0].pricing.storage_usd_gb_month = rate;
    f.boxes[1].pricing.compute_usd_hour = 0.3;
    f.boxes[1].pricing.storage_usd_gb_month = 0;
    f.requirements.items[0].runs = ['merge_bar'];
    assert.equal(plan(f).assignments[0].box_id, 'offer-1');
    f.requirements.items[0].tier = 'small';
    assert.equal(plan(f).assignments[0].box_id, 'offer-0');
  });
for (const cap of [0.1, 0.2])
  test('per-tier cap refuses expensive compute ' + cap, () => {
    const f = fixture();
    f.limits.price_caps.find((p) => p.tier === 'large').usd_hour = cap;
    f.boxes.forEach((b) => (b.pricing.compute_usd_hour = 0.3));
    const p = plan(f);
    assert.equal(p.admitted, false);
    assert.equal(p.assignments.length, 0);
  });
for (const key of ['machine_id', 'host_id'])
  for (const i of [0, 1])
    test('banned identity never offered to allocator ' + key + ' ' + i, () => {
      const f = fixture();
      f.boxes = [f.boxes[i]];
      f.limits[key === 'machine_id' ? 'bad_machine_ids' : 'bad_host_ids'] = [f.boxes[0][key]];
      assert.equal(plan(f).assignments.length, 0);
    });
for (const disk of [220, 299])
  test('requested allocated disk must fit ' + disk, () => {
    const f = fixture();
    f.boxes.forEach((b) => (b.limits.disk_gb = disk));
    assert.equal(plan(f).assignments.length, 0);
  });
for (const [field, values] of Object.entries({
  cpu_cores: [4, 7],
  disk_mb_per_second: [50, 99],
  inet_down_mbps: [100, 199],
  inet_up_mbps: [10, 49],
  reliability: [0.9, 0.979],
  dlperf_ratio: [0.3, 0.499],
}))
  for (const value of values)
    test('hard quality floor ' + field + ' ' + value, () => {
      const f = fixture();
      f.boxes.forEach((b) => (b.limits[field] = value));
      assert.equal(plan(f).assignments.length, 0);
    });
for (const n of [1, 2])
  test('maximum box count respected ' + n, () => {
    const f = fixture();
    f.limits.max_boxes = n;
    f.requirements.items[0].runs = Array(n + 1).fill('spec_lane_a');
    const p = plan(f);
    assert.equal(p.admitted, false);
    assert.ok(p.assignments.length <= n);
  });
for (const margin of [120, 150])
  test('margin removes usable box minutes ' + margin, () => {
    const f = fixture();
    f.limits.margin_minutes = margin;
    f.requirements.items[0].runs = ['spec_lane_a'];
    assert.equal(plan(f).assignments.length, 0);
  });
for (const hard of [180, 190])
  test('provider box hard limit is authoritative ' + hard, () => {
    const f = fixture();
    f.boxes.forEach((b) => (b.limits.hard_minutes = hard));
    f.requirements.items[0].runs = ['spec_lane_a'];
    assert.equal(plan(f).assignments.length, 0);
  });
for (const n of [1, 2])
  test('fixed known host cannot claim a different pinned target ' + n, () => {
    const f = fixture();
    const other = f.catalog.driver_pins.find((p) => p.os === 'linux' && p.version === '580.65.06');
    f.boxes.forEach((b) => {
      b.host_driver = other.id;
      b.capabilities.can_swap_host_driver = false;
    });
    if (n === 2) f.requirements.items[0].runs = ['merge_bar'];
    assert.equal(plan(f).assignments.length, 0);
  });
for (const axis of ['host', 'windows_guest'])
  test('selected pin OS matches driver axis ' + axis, () => {
    const f = fixture();
    f.requirements.items[0].driver[axis] = f.catalog.driver_pins.find(
      (p) => p.os === (axis === 'host' ? 'windows' : 'linux'),
    ).id;
    rejects(() => plan(f));
  });
for (const id of ['windows_gate', 'merge_bar'])
  test('different host drivers cannot silently share primary box ' + id, () => {
    const f = fixture(),
      q = clone(f.requirements.items[0]);
    q.runs = [id];
    f.requirements.items[0].runs = [id];
    q.id = 'other-primary';
    q.driver.host = f.catalog.driver_pins.find(
      (p) => p.os === 'linux' && p.version === '580.173.02',
    ).id;
    f.requirements.items.push(q);
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.equal(p.assignments.length, 2);
    assert.ok(p.assignments.every((a) => new Set(a.runs.map((r) => r.drivers.host)).size === 1));
  });
for (const count of [1, 2])
  test('driver cache keys cannot collide through delimiter aliases ' + count, () => {
    const f = fixture(),
      base = f.requirements.items[0].driver,
      lp = f.catalog.driver_pins.find((p) => p.os === 'linux' && p.version === '580.126.09'),
      w0 = f.catalog.driver_pins.find((p) => p.os === 'windows' && p.version === '580.88'),
      w1 = f.catalog.driver_pins.find((p) => p.os === 'windows' && p.version === '580.88');
    f.catalog.driver_pins.push(
      { ...clone(f.catalog.driver_pins.find((p) => p.id === base.host)), id: 'x:y' },
      { ...clone(lp), id: 'x' },
      { ...clone(w0), id: 'z' },
      { ...clone(w1), id: 'y:z' },
    );
    f.requirements.items = [
      {
        ...clone(f.requirements.items[0]),
        id: 'one',
        runs: Array(count).fill('windows_gate'),
        driver: { host: base.host, linux_guest: 'x:y', windows_guest: 'z' },
      },
      {
        ...clone(f.requirements.items[0]),
        id: 'two',
        runs: Array(count).fill('windows_gate'),
        driver: { host: base.host, linux_guest: 'x', windows_guest: 'y:z' },
      },
    ];
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.equal(p.assignments.length, 1);
    assert.equal(
      p.assignments[0].runs.filter((r) => r.run_id === 'windows_install_gate').length,
      2,
    );
  });
for (const count of [1, 2])
  test('host swap precedes its driver guest runs ' + count, () => {
    const f = fixture();
    f.requirements.items[0].evidence = 'driver';
    f.requirements.items[0].runs = [
      'guest_windows_tiny',
      'host_tiny',
      ...Array(count).fill('guest_linux_tiny'),
    ];
    const a = plan(f).assignments[0];
    const ids = a.runs.map((r) => r.run_id);
    assert.ok(
      ids.indexOf('host_tiny') < ids.indexOf('guest_windows_tiny') &&
        ids.indexOf('host_tiny') < ids.indexOf('guest_linux_tiny'),
    );
  });
for (const axis of ['host', 'linux_guest'])
  test('unknown selected minimum prevents executable data ' + axis, () => {
    const f = fixture();
    f.catalog.dies[0].minimum_driver_branches[axis] = null;
    assert.equal(plan(f).admitted, false);
  });
for (const run of ['merge_bar', 'windows_gate'])
  test('proposed selected die is not executable ' + run, () => {
    const f = fixture();
    f.catalog.dies[0].status = 'proposed';
    f.requirements.items[0].runs = [run];
    assert.equal(plan(f).admitted, false);
  });
for (const axis of ['inet_down_usd_gb', 'inet_up_usd_gb'])
  for (const rate of [0.021, 0.04])
    test('per-GB cap applies to ' + axis + ' ' + rate, () => {
      const f = fixture();
      f.boxes.forEach((b) => (b.pricing[axis] = rate));
      assert.equal(plan(f).assignments.length, 0);
    });
for (const die_family of ['Turing', 'Ampere'])
  test('requirement cannot supply die and die family ' + die_family, () => {
    const f = fixture();
    f.requirements.items[0].die_family = die_family;
    rejects(() => plan(f));
  });
for (const key of ['branches', 'patches'])
  test('requirement cannot supply driver and sweep ' + key, () => {
    const f = fixture();
    f.requirements.items[0].driver_sweep = { [key]: ['580'] };
    rejects(() => plan(f));
  });
for (const run of ['windows_gate', 'guest_linux_tiny'])
  test('isolated driver box uses explicit host transition ' + run, () => {
    const f = fixture(),
      q = clone(f.requirements.items[0]);
    f.requirements.items[0].evidence = 'driver';
    f.requirements.items[0].runs = [run];
    q.evidence = 'driver';
    q.runs = [run];
    q.id = 'other-driver';
    q.driver.host = f.catalog.driver_pins.find(
      (p) => p.os === 'linux' && p.version === '580.173.02',
    ).id;
    f.requirements.items.push(q);
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.equal(p.assignments.length, 1);
    const a = p.assignments[0];
    assert.equal(a.primary_evidence, false);
    assert.ok(a.runs.some((r) => r.run_id === 'host_tiny'));
    let active = a.runs[0].drivers.host;
    for (const r of a.runs) {
      if (r.drivers.host !== active) {
        assert.equal(f.catalog.runs.find((x) => x.id === r.run_id).host_swap, true);
        active = r.drivers.host;
      }
    }
  });
for (const kind of ['minimum', 'status'])
  test('input execution approval includes selected die metadata ' + kind, () => {
    const f = fixture();
    if (kind === 'minimum') f.catalog.dies[0].minimum_driver_branches.host = null;
    else f.catalog.dies[0].status = 'proposed';
    assert.equal(validateInput(f).executable, false);
    rejects(() => validateInput(f, { forExecution: true }));
  });
for (const run of ['merge_bar', 'windows_gate'])
  test('die-family target uses approved alternative over proposed cheap die ' + run, () => {
    const f = fixture();
    delete f.requirements.items[0].die;
    f.requirements.items[0].die_family = 'Turing';
    f.requirements.items[0].runs = [run];
    f.catalog.dies[0].status = 'proposed';
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.every((a) => a.die === 'TU116'));
  });
for (const id of ['host_tiny', 'host_full'])
  test('host driver action is never cacheable ' + id, () => {
    const f = fixture();
    if (id === 'host_full')
      f.catalog.runs.push({ ...clone(f.catalog.runs.find((r) => r.id === 'host_tiny')), id });
    f.catalog.runs.find((r) => r.id === id).setup = true;
    rejects(() => plan(f));
  });
for (const run of ['windows_gate', 'guest_linux_tiny'])
  test('host transition action is catalog data ' + run, () => {
    const f = fixture(),
      q = clone(f.requirements.items[0]);
    f.catalog.runs.push({
      ...clone(f.catalog.runs.find((r) => r.id === 'host_tiny')),
      id: 'host_full',
      estimated_minutes: 20,
    });
    f.limits.host_transition_run = 'host_full';
    f.requirements.items[0].evidence = 'driver';
    f.requirements.items[0].runs = [run];
    q.evidence = 'driver';
    q.runs = [run];
    q.id = 'other-driver';
    q.driver.host = f.catalog.driver_pins.find(
      (p) => p.os === 'linux' && p.version === '580.173.02',
    ).id;
    f.requirements.items.push(q);
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.equal(p.assignments.length, 1);
    assert.ok(p.assignments[0].runs.some((r) => r.run_id === 'host_full' && r.minutes === 20));
  });
for (const axis of ['windows_guest', 'linux_guest'])
  test('unknown selected guest ABI prevents executable assignment ' + axis, () => {
    const f = fixture(),
      id = f.requirements.items[0].driver[axis];
    f.catalog.driver_pins.find((p) => p.id === id).abi_version = null;
    const p = plan(f);
    assert.equal(p.admitted, false);
    assert.equal(validateInput(f).executable, false);
    rejects(() => validatePlan(f, p));
  });
// FALSIFIER: an inherited schema property admits an undeclared JSON field.
for (const key of ['constructor', '__proto__'])
  test('closed schema rejects inherited property name ' + key, () => {
    const f = fixture();
    Object.defineProperty(f.catalog, key, { value: {}, enumerable: true });
    rejects(() => plan(f));
  });

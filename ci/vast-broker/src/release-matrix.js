// Proposed release data, never a hardware qualification. Owner decisions: 2026-11-07.
import matrix from '../release-matrix.json' with { type: 'json' };
export const RELEASE_MATRIX = matrix;
const AXES = ['host_driver', 'linux_guest_driver', 'windows_guest_driver'];
export const driverVersion = (v) =>
  typeof v === 'string' && /^[0-9]{3}\.[0-9]{2,3}(?:\.[0-9]{2})?$/.test(v) && !v.includes('\n');
const object = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
const keys = (v, names) => object(v) && Object.keys(v).sort().join() === [...names].sort().join();
const compare = (a, b) => {
  const x = a.split('.').map(Number),
    y = b.split('.').map(Number);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const delta = (x[i] ?? 0) - (y[i] ?? 0);
    if (delta) return delta;
  }
  return 0;
};
export function validateReleaseMatrix(value, cfg) {
  const errors = [];
  const bad = (ok, reason) => {
    if (!ok) errors.push(reason);
    return ok;
  };
  if (
    !bad(
      keys(value, [
        'schema_version',
        'status',
        'date',
        'policy',
        'host_capabilities',
        'sources',
        'dies',
      ]),
      'matrix schema',
    )
  )
    return errors;
  if (
    !bad(
      value.schema_version === 1 &&
        value.status === 'proposed' &&
        /^\d{4}-\d{2}-\d{2}$/.test(value.date),
      'matrix schema version/status/date',
    )
  )
    return errors;
  const p = value.policy;
  if (
    !bad(
      keys(p, [
        'die_count',
        'dies_per_die_family',
        'boxes_per_die',
        'parallel_boxes',
        'full_profile',
        'subrun_profiles',
        'die_families',
        'allow_adjacent_driver_families',
      ]) &&
        Number.isSafeInteger(p.die_count) &&
        p.die_count > 0 &&
        Number.isSafeInteger(p.dies_per_die_family) &&
        p.dies_per_die_family > 0 &&
        p.boxes_per_die === 1 &&
        p.parallel_boxes === p.die_count &&
        p.full_profile === 'full' &&
        JSON.stringify(p.subrun_profiles) === '["middle","small"]' &&
        p.allow_adjacent_driver_families === false &&
        JSON.stringify(p.die_families) === JSON.stringify(Object.keys(cfg.families)) &&
        p.dies_per_die_family === cfg.releaseDiesPerFamily &&
        p.die_count === p.dies_per_die_family * p.die_families.length,
      'policy schema',
    )
  )
    return errors;
  bad(
    keys(value.host_capabilities, ['vast_kvm']) &&
      keys(value.host_capabilities.vast_kvm, ['can_swap_host_driver']) &&
      value.host_capabilities.vast_kvm.can_swap_host_driver === true,
    'host capabilities schema',
  );
  bad(
    Array.isArray(value.sources) &&
      value.sources.length > 0 &&
      value.sources.every(
        (s) =>
          object(s) &&
          typeof s.checked === 'string' &&
          typeof s.meaning === 'string' &&
          (typeof s.path === 'string' || typeof s.url === 'string'),
      ),
    'sources schema',
  );
  if (!bad(Array.isArray(value.dies), 'dies schema')) return errors;
  bad(value.dies.length === p.die_count, 'die count');
  const seen = new Set(),
    combinations = new Set();
  const driversValid = (drivers, minimum, label) => {
    if (!bad(keys(drivers, AXES), label + ' drivers schema')) return false;
    for (const axis of AXES) {
      if (!bad(driverVersion(drivers[axis]), label + ' driver version')) continue;
      const floor = minimum?.[axis];
      if (floor !== undefined && floor !== 'UNKNOWN' && driverVersion(floor))
        bad(compare(drivers[axis], floor) >= 0, label + ' minimum driver');
    }
    return AXES.every((a) => driverVersion(drivers[a]));
  };
  for (const row of value.dies) {
    if (
      !bad(
        keys(row, [
          'die',
          'die_family',
          'role',
          'status',
          'profile',
          'drivers',
          'windows_guest_abi',
          'minimum_drivers',
          'subruns',
          'cross_subrun',
          'blockers',
        ]),
        'die schema',
      )
    )
      continue;
    const label = row.die;
    bad(typeof label === 'string' && !seen.has(label), 'unique die');
    seen.add(label);
    bad(
      p.die_families.includes(row.die_family) &&
        cfg.dieRules.some((r) => r.die === label && r.cc === cfg.families[row.die_family]),
      label + ' die family',
    );
    bad(cfg.releaseDies[row.die_family]?.includes(label), label + ' releaseDies');
    bad(
      ['flagship', 'second'].includes(row.role) &&
        row.status === 'proposed' &&
        row.profile === p.full_profile,
      label + ' role/status/profile schema',
    );
    bad(
      row.windows_guest_abi === 'UNKNOWN' || driverVersion(row.windows_guest_abi),
      label + ' Windows ABI schema',
    );
    bad(
      keys(row.minimum_drivers, AXES) &&
        AXES.every(
          (a) => row.minimum_drivers[a] === 'UNKNOWN' || driverVersion(row.minimum_drivers[a]),
        ),
      label + ' minima schema',
    );
    if (driversValid(row.drivers, row.minimum_drivers, label)) {
      const ids = ['linux_guest_driver', 'windows_guest_driver'].map(
        (axis) => axis + '/' + row.drivers.host_driver + '/' + row.drivers[axis],
      );
      bad(
        ids.every((id) => !combinations.has(id)),
        'unique driver combinations',
      );
      ids.forEach((id) => combinations.add(id));
    }
    bad(
      Array.isArray(row.blockers) &&
        row.blockers.every((b) => typeof b === 'string' && /^[A-Z][A-Z0-9_]+$/.test(b)),
      label + ' blockers schema',
    );
    if (bad(Array.isArray(row.subruns) && row.subruns.length <= 32, label + ' subruns schema'))
      for (const run of row.subruns) {
        if (
          !bad(keys(run, ['os', 'driver_family', 'profile', 'drivers']), label + ' subrun schema')
        )
          continue;
        bad(['linux', 'windows'].includes(run.os), label + ' subrun OS');
        bad(p.subrun_profiles.includes(run.profile), label + ' subrun profile');
        const axis = run.os === 'linux' ? 'linux_guest_driver' : 'windows_guest_driver';
        const fullMajor = driverVersion(row.drivers?.[axis])
          ? row.drivers[axis].split('.')[0]
          : undefined;
        bad(
          typeof run.driver_family === 'string' && /^\d{3}$/.test(run.driver_family),
          label + ' driver family',
        );
        if (
          bad(
            Array.isArray(run.drivers) && run.drivers.length > 0 && run.drivers.length <= 16,
            label + ' subrun drivers schema',
          )
        )
          for (const driver of run.drivers) {
            bad(
              driverVersion(driver) && driver.split('.')[0] === run.driver_family,
              label + ' driver family/version',
            );
            bad(
              run.profile === (run.driver_family === fullMajor ? 'small' : 'middle'),
              label + ' subrun profile boundary',
            );
            const floor = row.minimum_drivers?.[axis];
            if (driverVersion(driver) && driverVersion(floor))
              bad(compare(driver, floor) >= 0, label + ' minimum driver');
          }
      }
    if (row.role === 'flagship') bad(row.cross_subrun === null, label + ' cross subrun');
    else {
      const cross = row.cross_subrun;
      const flagship = value.dies.find(
        (d) => object(d) && d.die_family === row.die_family && d.role === 'flagship',
      );
      if (
        bad(
          keys(cross, ['die', 'profile', 'drivers', 'windows_guest_abi']) && flagship !== undefined,
          label + ' cross subrun',
        )
      ) {
        bad(
          cross.die === flagship.die &&
            p.subrun_profiles.includes(cross.profile) &&
            AXES.every((a) => cross.drivers?.[a] === flagship.drivers?.[a]) &&
            cross.windows_guest_abi === flagship.windows_guest_abi,
          label + ' cross subrun drivers',
        );
        driversValid(cross.drivers, row.minimum_drivers, label + ' cross subrun');
        if (AXES.every((a) => driverVersion(row.drivers?.[a]) && driverVersion(cross.drivers?.[a])))
          bad(
            cross.profile ===
              (AXES.some((a) => row.drivers[a].split('.')[0] !== cross.drivers[a].split('.')[0])
                ? 'middle'
                : 'small'),
            label + ' cross subrun profile',
          );
        if (
          driversValid(row.drivers, undefined, label) &&
          driversValid(flagship.drivers, undefined, label)
        )
          bad(
            AXES.every((a) => compare(flagship.drivers[a], row.drivers[a]) > 0),
            label + ' new/older drivers',
          );
      }
    }
  }
  for (const dieFamily of p.die_families) {
    const rows = value.dies.filter((d) => object(d) && d.die_family === dieFamily);
    bad(
      rows.length === p.dies_per_die_family &&
        rows.filter((d) => d.role === 'flagship').length === 1 &&
        rows.filter((d) => d.role === 'second').length === 1,
      dieFamily + ' die family count/roles',
    );
  }
  return errors;
}
export function hostProvidesDrivers(capabilities, row) {
  if (capabilities?.can_swap_host_driver === true) return true;
  if (capabilities?.can_swap_host_driver !== false) return false;
  return [row.drivers, ...(row.cross_subrun ? [row.cross_subrun.drivers] : [])].every(
    (d) => d.host_driver === capabilities.host_driver,
  );
}
export function releaseEnvironment(row) {
  const env = {
    KF_HOST_DRIVER: row.drivers.host_driver,
    KF_LINUX_GUEST_DRIVER: row.drivers.linux_guest_driver,
    KF_WINDOWS_GUEST_DRIVER: row.drivers.windows_guest_driver,
    KF_RELEASE_SUBRUNS: btoa(
      JSON.stringify({ subruns: row.subruns, cross_subrun: row.cross_subrun, status: row.status }),
    )
      .replaceAll('+', '-')
      .replaceAll('/', '_')
      .replace(/=+$/, ''),
  };
  if (row.windows_guest_abi !== 'UNKNOWN') env.KF_WINGATE_GUEST_DRIVER = row.windows_guest_abi;
  return env;
}

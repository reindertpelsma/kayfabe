import { fixture, clone } from './helpers.js';
export function seeded(consumers = 1) {
  const f = fixture(),
    q = clone(f.requirements.items[0]);
  f.limits.seeding.enabled = true;
  f.limits.max_concurrent_boxes = 2;
  f.requirements.items = [
    { ...q, id: 'seed-source', runs: ['linux_image_build', 'kayfabe_build', 'windows_apps_full'] },
  ];
  f.boxes = f.boxes.filter((b) => ['offer-0', 'offer-1', 'offer-6'].includes(b.id));
  f.boxes[0].pricing.inet_up_usd_gb = 0.02;
  f.boxes[1].pricing.compute_usd_hour = 0.13;
  for (let i = 0; i < consumers; i++) {
    const die = 'TU11' + (6 + i);
    if (i) {
      f.catalog.dies.push({ ...clone(f.catalog.dies[1]), id: die });
      const b = {
        ...clone(f.boxes[2]),
        id: 'consumer-' + i,
        machine_id: 'consumer-machine-' + i,
        host_id: 'consumer-host-' + i,
        die,
      };
      f.boxes.push(b);
      f.catalog.driver_sets[0].supported_dies.push(die);
    }
    f.requirements.items.push({
      ...q,
      id: 'consumer-' + i,
      die,
      tier: 'medium',
      runs: ['linux_apps_full'],
    });
  }
  // A full source keeps running while a second first-wave box finishes.
  // Consumers then have a profitable direct-transfer window inside its lifetime.
  const blockerDie = 'TU102';
  f.catalog.dies.push({ ...clone(f.catalog.dies[0]), id: blockerDie });
  f.catalog.driver_sets[0].supported_dies.push(blockerDie);
  f.catalog.runs.push({
    ...clone(f.catalog.runs.find((r) => r.id === 'merge_bar')),
    id: 'fixture_blocker',
    estimated_minutes: 75,
    needs: ['source'],
    driver_axes: [],
  });
  f.boxes.push({
    ...clone(f.boxes[0]),
    id: 'blocker-box',
    machine_id: 'blocker-machine',
    host_id: 'blocker-host',
    die: blockerDie,
  });
  f.requirements.items.push({
    ...q,
    id: 'first-wave-blocker',
    die: blockerDie,
    runs: ['fixture_blocker'],
  });
  return f;
}

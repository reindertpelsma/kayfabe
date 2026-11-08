import { fixture, clone } from './helpers.js';
export function seeded(consumers = 1) {
  const f = fixture(),
    q = clone(f.requirements.items[0]);
  f.limits.seeding.enabled = true;
  f.limits.max_concurrent_boxes = 1;
  f.requirements.items = [
    { ...q, id: 'seed-source', runs: ['linux_image_build', 'kayfabe_build'] },
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
  return f;
}

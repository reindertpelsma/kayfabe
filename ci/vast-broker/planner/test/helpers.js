import { readFileSync } from 'node:fs';
const pinned = JSON.parse(
  readFileSync(new URL('../fixtures/official-driver-pins.json', import.meta.url)),
).driver_installers;
export function fixture() {
  const pins = Object.entries(pinned)
    .filter(([id, p]) => p.status === 'PINNED')
    .map(([id, p]) => ({
      id,
      driver_family: Number(p.version.split('.')[0]),
      abi_version: p.os === 'linux' ? p.version : p.version === '580.88' ? '580.65.06' : null,
      ...p,
    }));
  const l = pins.find((p) => p.os === 'linux' && p.version === '580.159.04').id;
  const w = pins.find((p) => p.os === 'windows' && p.version === '580.88').id;
  const driver = { host: l, linux_guest: l, windows_guest: w };
  const runs = [
    { id: 'provision', estimated_minutes: 10, needs: [], produces: ['source'], setup: true },
    {
      id: 'kayfabe_build',
      estimated_minutes: 20,
      needs: ['source'],
      produces: ['kf3_bins'],
      setup: true,
    },
    {
      id: 'linux_image_build',
      estimated_minutes: 30,
      needs: ['source'],
      produces: ['open_image'],
      setup: true,
    },
    { id: 'merge_bar', estimated_minutes: 11, needs: ['kf3_bins'], produces: [] },
    {
      id: 'linux_apps_full',
      estimated_minutes: 20,
      needs: ['kf3_bins', 'open_image'],
      produces: [],
    },
    {
      id: 'windows_install_gate',
      estimated_minutes: 33,
      needs: ['source'],
      produces: ['windows_base'],
      setup: true,
      capabilities: ['kvm', 'vfio'],
    },
    {
      id: 'windows_gate',
      estimated_minutes: 17,
      needs: ['windows_base'],
      produces: [],
      capabilities: ['kvm', 'vfio'],
    },
    {
      id: 'windows_apps_full',
      estimated_minutes: 94,
      needs: ['windows_base'],
      produces: [],
      capabilities: ['kvm', 'vfio'],
    },
    {
      id: 'spec_prepare',
      estimated_minutes: 30,
      needs: ['windows_base'],
      produces: ['spec_tree'],
      setup: true,
      download_gb: 43,
      disk_gb: 220,
    },
    {
      id: 'spec_lane_a',
      estimated_minutes: 150,
      includes_setup: ['spec_tree'],
      needs: ['spec_tree'],
      produces: [],
      disk_gb: 220,
    },
    {
      id: 'spec_lane_b',
      estimated_minutes: 120,
      includes_setup: ['spec_tree'],
      needs: ['spec_tree'],
      produces: [],
      disk_gb: 220,
    },
    { id: 'guest_linux_tiny', estimated_minutes: 5, needs: ['kf3_bins'], produces: [] },
    {
      id: 'guest_windows_tiny',
      estimated_minutes: 13,
      needs: ['windows_base'],
      produces: [],
      capabilities: ['kvm', 'vfio'],
    },
    {
      id: 'host_tiny',
      estimated_minutes: 12,
      needs: ['source'],
      produces: [],
      capabilities: ['can_swap_host_driver'],
      host_swap: true,
    },
  ].map((r) => ({
    scope: r.id.includes('tiny') ? 'driver' : 'die',
    driver_axes: ['provision', 'kayfabe_build', 'linux_image_build'].includes(r.id)
      ? []
      : r.id.includes('windows') || r.id.startsWith('spec')
        ? ['windows_guest']
        : r.id.startsWith('host')
          ? ['host']
          : ['linux_guest'],
    setup: false,
    capabilities: [],
    host_swap: false,
    parameters: {},
    download_gb: 0,
    disk_gb: 120,
    includes_setup: [],
    ...r,
  }));
  const artifacts = [
    { id: 'source', producer: 'provision', gb: 0, scope: 'box', seedable: false },
    { id: 'kf3_bins', producer: 'kayfabe_build', gb: 0.1, scope: 'box', seedable: true },
    { id: 'open_image', producer: 'linux_image_build', gb: 13, scope: 'box', seedable: true },
    {
      id: 'windows_base',
      producer: 'windows_install_gate',
      gb: 35,
      scope: 'driver',
      seedable: false,
    },
    { id: 'spec_tree', producer: 'spec_prepare', gb: 43, scope: 'box', seedable: false },
  ];
  const profiles = ['large', 'medium', 'small'].map((tier, i) => ({
    tier,
    rank: 3 - i,
    can_seed: i === 0,
    disk_gb: i ? 120 : 300,
    min_disk_gb: i ? 120 : 220,
    bandwidth_down_gb: [130, 87, 73][i],
    base_upload_gb: 2,
    max_inet_usd_gb: i ? 0.005 : 0.02,
    min_cpu_cores: 8,
    min_disk_mb_per_second: 100,
    min_down_mbps: 200,
    min_up_mbps: 50,
    min_reliability: 0.98,
    min_dlperf_ratio: 0.5,
    artifact_sources: artifacts.map((a) => ({
      artifact: a.id,
      policy: i && a.seedable ? 'optional_seed' : 'local',
    })),
  }));
  const catalog = {
    version: 1,
    status: 'executable',
    revision: 'a'.repeat(40),
    runs,
    artifacts,
    dies: [
      {
        id: 'TU106',
        die_family: 'Turing',
        status: 'executable',
        supported_branches: [580],
        minimum_driver_branches: { host: 580, linux_guest: 580, windows_guest: 580 },
      },
      {
        id: 'TU116',
        die_family: 'Turing',
        status: 'executable',
        supported_branches: [580],
        minimum_driver_branches: { host: 580, linux_guest: 580, windows_guest: 580 },
      },
    ],
    driver_pins: pins,
    driver_sets: [
      {
        id: '580',
        kind: 'branch',
        branch: 580,
        drivers: driver,
        supported_dies: ['TU106', 'TU116'],
      },
    ],
    profiles,
  };
  const requirements = {
    version: 1,
    status: 'executable',
    items: [
      {
        id: 'primary',
        die: 'TU106',
        runs: ['merge_bar', 'linux_apps_full', 'windows_install_gate', 'windows_gate'],
        driver,
        tier: 'large',
        evidence: 'primary',
        must: true,
      },
    ],
  };
  const limits = {
    version: 1,
    status: 'executable',
    hard_minutes: 300,
    margin_minutes: 15,
    max_concurrent_boxes: 2,
    max_boxes: 32,
    cost_cap_usd: 45,
    storage_month_hours: 720,
    price_caps: profiles.map((p) => ({ tier: p.tier, usd_hour: 2 })),
    bad_machine_ids: [],
    bad_host_ids: [],
    host_swap_policy: 'separate_from_primary',
    host_transition_run: 'host_tiny',
    seeding: {
      enabled: false,
      min_savings_minutes: 1,
      max_wait_minutes: 5,
      price_weight: 1,
      upload_cost_weight: 1,
      upload_speed_weight: 0,
    },
  };
  const boxes = Array.from({ length: 12 }, (_, i) => ({
    id: 'offer-' + i,
    machine_id: 'machine-' + i,
    host_id: 'host-' + i,
    die: i < 6 ? 'TU106' : 'TU116',
    provider: i % 2 ? 'vast' : 'local',
    host_driver: l,
    capabilities: { can_swap_host_driver: true, kvm: true, vfio: true },
    limits: {
      hard_minutes: 300,
      disk_gb: 400,
      cpu_cores: 16,
      disk_mb_per_second: 300,
      inet_down_mbps: 1000,
      inet_up_mbps: 1000,
      reliability: 0.999,
      dlperf_ratio: 1,
    },
    pricing: {
      compute_usd_hour: 0.1 + i * 0.001,
      storage_usd_gb_month: 0.12,
      inet_down_usd_gb: 0.001,
      inet_up_usd_gb: 0.001,
    },
  }));
  return { version: 1, source_revision: 'b'.repeat(40), catalog, requirements, limits, boxes };
}
export const clone = (v) => JSON.parse(JSON.stringify(v));

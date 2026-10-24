# BW3 round B1f ("7-visible-core deviation" profile), operator: Claude, 2026-10-23

STATUS: ANSWERED, 2026-10-23. Every arm is labelled "7-visible-core deviation". The lease was refused by the SSH identity guard before SSH. No arm ran.

- Offline inventory, CODE READ (`cpu_ram_inventory.json`): 15 CPU, RAM and time parameters across the operator, the run_kit, product 6e3fd3fd6 and readiness 9ff51a6e9.
  - Gates: offer floor 16; RAM 32 GB; upload MemTotal 32 GB, disk 110 GB and /dev/kvm; in-box `host_preflight.sh` nproc >= 8.
  - Parallelism: `CARGO_BUILD_JOBS` 8, 16 and 8 in the run_kit, and 16 in product `provision_bench_tree.sh`.
  - Guest: Windows `WINVM_SMP` 8; the provisioning QEMU uses `-smp 8`.
  - Native fuzz: 3 cores, crowd of 9 workers. The planner has no core arithmetic. Time bounds are listed and left unchanged.
- Profile (`run_kit/operator_lease_profile.py`, `run_kit/preflight_profile.py`):
  - Floors: offer and visible-core floor 7, rejecting 6.
  - Windows guest: `WINVM_SMP` 6, keeping one host CPU for QEMU I/O, the drainer/relay and the controller (INFERRED choice).
  - Builds: `CARGO_BUILD_JOBS`/`-j` 7 in the run_kit. The product's two lines stay unchanged, because the source is pinned and they only affect parallelism.
  - Fuzz: the pinned-3-core arm is kept, since 3 cores fit within 7.
  - Preflight: the pinned product preflight is unchanged and judged under the profile. Its only accepted deviation is exactly "only 7 cores" with nproc 7.
  - Time: no time bound is shortened, and the 3 h ceiling stays under the cost stop. Machine 99805 is preferred.
  - Defaults are unchanged (16 and 8).
- Tests: `rehearsal/test_lease_profile.py` 33/33. It was RED before the module existed, and 16 mutants are each killed by 2 or more tests. Country filter (CU/IR/KP/SY/CN/HK/MO) 5/5, harness 366/366, operator rehearsal 3/3.
- MEASURED, `market.json` at 22:19:43Z: m99805 was absent. The selected offer was 54877110: RTX 5070 (GB205), machine 39248, Quebec CA, verified, 10 effective cores, 0.267 USD/h.
- MEASURED, lease 57431305:
  - The watchdog was ARMED at 22:20:10.586Z, before the create at 22:20:11.897Z.
  - At 259.9 s, `console_pin.PinMismatch`: the instance console published 2 host-key fingerprints, and the scanned ed25519 key matched neither. SSH was refused by design, so no in-box mismatch could be observed.
  - The harness destroyed the lease by exact id, with API absence verified at 22:24:37.787Z. Timers were cancelled after that. The runtime estimate is 0.021 USD.

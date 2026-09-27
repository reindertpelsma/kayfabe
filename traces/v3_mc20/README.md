# v3-mc20 merge bar — evidence

**STATUS: LIVE, 2026-09-27.** The full merge bar (`scripts/bench/box/merge_check.sh`) for merge candidate
v3-mc20 = master `0d3ecde9` + `v3-drivers~1` (`8f9bdd14`; the held 535/545 allowlist commit `a50265f8` left
out), merge commit `5018bb57`. The bar ran at `c0ef7b75` = `5018bb57` + one docs-only commit (handoff §0).

- **Box:** vast 53004208, RTX 3060 (GA106, LHR, `10de:2504`), AMD EPYC 7K62 host, nested KVM (`kvm_amd
  nested=1`), 11 vCPUs, 49 GiB, 194 GB root. Host driver swapped to **580.159.04 open** (`driver.log`:
  `OPEN_MODULE=yes`, `VERSION_MATCH=yes`). Guest kernel 6.8.0-142, guest driver 580.159.04.
- **Driven through execd** behind a cloudflared quick tunnel (the cloud session has no SSH egress), with
  `provision_full.sh` then `merge_check.sh claude/kayfabe-gpu-testing-m0cv1q mc20`, unchanged.

| Bar item | Result | File |
|---|---|---|
| every `kf-*` crate test (`--no-fail-fast`) | **1639 passed / 0 failed** | `mc20.log` |
| v3 gates | **9/9** (`GATE1..9_VERDICT=PASS`) | `mc20_gates.log` |
| kf3 build of this revision | `KF3_RC=0`, `kf3-bins/c0ef7b75/qemu-system-x86_64` | `mc20_kf3.log` |
| raw client + fast guest rebuilt | `FG_RC=0` (no nbd flake) | `mc20_fg.log` |
| 30-arm thin-guest suite, budget 180 | **FAST_SUITE_PASS=30 FAIL=0 CRASH=0 NOTRUN=0** | `mc20_suite.out` |

Provisioning (`prov.log`): BOX_RC / DRIVER_RC / TREE_RC / LADDER_RC / FG_RC / KF3_RC all 0, READY at
`c0ef7b75`, 11.5 min from a fresh box. Bar wall time 18:08 → 18:27 UTC.

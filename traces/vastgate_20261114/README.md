# vastgate 2026-11-14: hardware gate of origin/next on a Vast RTX 3090

**STATUS: LIVE, 2026-11-14.** Measured on one rented box; inferences are marked.

- **Revision under test (`origin/next`):** `b20fd84bfde5e47d80147c1ca97f4a895f97ca0b` (merge of #410; the box
  `git rev-parse HEAD` printed the same value). kf3 binary `kf3-bins/b20fd84b/`.
- **Baseline for the A/B:** `origin/master` `921c39444faede8c36e69eef50f8aa5103328378` (kf3 binary `kf3-bins/921c3944/`).
- **Box:** Vast instance 60697244 (offer 60434436, Michigan US), RTX 3090 24 GB, host driver 580.159.04,
  AMD Ryzen 5 5600X (5 vCPU, 24 GB RAM visible), kernel 6.8.0-59, KVM image `vastai/kvm:ubuntu_cli_22.04-2025-05-16`,
  root disk 117 GB (110 GB free). 0.322 USD/h. Rented 21:15 UTC, destroyed 22:15 UTC (about 1.0 h, about 0.35 USD); `vastai show instances` empty afterwards.
- Run: `scripts/bench/box/merge_check.sh next vg1114` (gates, kf3 build, raw client, bare-metal suite, fast guest, suite, birth census), then the reset lane by hand.

## Results (measured)

| Lane | Result | Duration |
|---|---|---|
| provision (`provision_full.sh`) | `READY driver=580.159.04` | 11 min |
| `v3_gates.sh` | `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS` | in the 17 min merge_check |
| `build_kf3.sh` | `KF3_RC=0` | |
| bare-metal 30-arm suite (reference) | `BARE_SUITE_PASS=30 FAIL=0 CRASH=0` | |
| `fast_suite.sh vg1114 180` (kf3) | `FAST_SUITE_PASS=30 FAIL=0 CRASH=0 NOTRUN=0` | |
| birth census | `BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=12 all PRIVILEGED_CHANNEL=0` | |
| merge_check total | `EXIT rc=0` | 17 min 22 s (21:35:11 - 21:52:33) |
| reset lane, QMP `system_reset` x5 while the guest runs `--defer-liveness --ce-client-guest-ram` | 5 resets, every boot reached its arms, final boot `client rc=0`; **but 4 of 5 resets hit the 2 s wait bound (see below)** | 4 min |
| Windows gate | NOT RUN (proprietary image not available on Vast) | |

Logs scanned over the suite and gates: `walk report refused` 0, `D-05` 0, `PeerWritePtrOutOfRange`/`SeqNumGap`/`panic`/Code 43 0.
Refusals are the expected host-facts and invalidate "refused leaves absent" lines (`ev/suite_refusal_summary.log`).
`pramin_pool_empty=0` and `pramin_retire_full=0` on all 307 suite heartbeats and all 122 reset-lane heartbeats; `pramin_miss=0`, `pramin_kept=0`.
Host Xid: exactly 5 Xid 31 (4 in `--defer-liveness`, 1 in `missing-page-fault`), both arms "Xid-admitted" by design
(`V3_RAW_CLIENT.md`), the same set as `traces/linux_regression_20261010/README.md`. No other Xid; the reset lanes added none. No TDR.

## Finding: the 2 s reset wait bound is exceeded under a busy guest (pre-existing, not from this batch)

`V3_DEVICE_RESET.md` L1 expects `waits_timed_out=0` and no `NOT complete after 2s`. Measured:

| Binary | resets with a busy guest | `NOT complete after 2s` (stage Memory) | waits (us), completed in time |
|---|---|---|---|
| next `b20fd84b` (run vgreset2) | 5 | 4 (reqs 2, 4, 6, 10) | 150960, 106284, 128809, 1977688, 14275, 172715 (as logged) |
| master `921c3944` (run vgmaster1, same guest, same box) | 5 | 4 | 609623, 523806, 251876, 1861600, 15997, 70905 |

Each timeout says `stage Memory: every guest client already freed`; the reset then completes in the background, the guest
boots and the next arms pass, channel plane `EMPTY after 1 round`, `0 KEPT`. [inferred] The memory-plane stage of a reset
taken while `--ce-client-guest-ram` has ~24k invalidates in flight takes more than 2 s on a nested-KVM box; the batch did not
introduce it (master behaves the same: A/B on one box, one run each). [not measured] whether bare metal or a PC would meet 2 s;
a reset has no bare-metal equivalent (it is the kf3 device's), so the bare-metal-first rule gives no reading here.
Note: the first attempt (vgreset1, fixed 75 s timer) landed 2 resets in a mostly-booting guest and is kept in the logs only as such;
`reset_lane.sh` is the event-driven version (reset 20 s after each boot's arms start).

## Files

`logs.tgz` (about 58 KB): `ev/` with the merge_check logs, suite output, gates log, birth census, reset-lane QEMU lines, host NVRM dmesg,
pramin counters. `reset_lane.sh`: the lane as run. Raw logs also at `/workspace/bench/vastgate_20261114/` on the coordinator.

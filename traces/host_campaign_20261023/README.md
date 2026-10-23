# Host campaign 2026-10-23/24 (RTX 4070 host): TSG-mirror matched control and Energy CE witness

**STATUS: RESULT, 2026-10-24.** Evidence-only branch `claude/host-campaign-20261023` (from master `c25eb0c8b`). No product code changed.
Labels: MEASURED = a named run below; CODE READ = source; INFERRED = reasoning.
Steel Nomad (3DMark) scores are private by policy and are NOT in this repository; verdicts below are stated without the numbers
(the operator's private report holds them). No SPECviewperf scores are included.

## Item 1: TSG-mirror matched control (Steel Nomad, DX12 default, 3DMark Demo GUI)
Build: binary rev `0d7ba743` (branch `claude/tsg-mirror-20261018` @ `c2685412e`; candidate-5 + mirror; md5 `48ab1b70c2cf2c26d2c1287bf7311a9a`), the same
binary in both arms; the only flag difference is `KF3_TSG_MIRROR=0` (OFF) vs unset (ON). The mirror is not in master and not in candidate 6d
(trial merge of the mirror branch into 6d: one content conflict in `crates/kf-qemu/tests/channel_budget_lazy.rs`, not resolved: no unreviewed integration build).
Flags: launcher `broker_gap.sh` as `win-launch.sh` v4 of the mirror lane, `KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5`, 8192 MiB store, 12 GiB RAM, 8 vCPU;
`evidence/steel/flags-5001-5002.json`. Image: owner overlay (backing win11-full-b), fresh CoW overlay per boot, owner files unchanged.

| run | mirror | Steel Nomad runs | host Xid delta | guest 153 / 4101 |
|---|---|---|---|---|
| 5001 | ON | 2 valid | 0 | 0 / 0 |
| 5002 | OFF | 2 valid | 0 | 0 / 0 |
| 5003 | ON | 2 valid | 0 | 0 / 0 |
| 5004 | OFF | 0 valid (run failed, score 0) | +7 (Xid 13 SKEDCHECK on compute class c9c0, Xid 32, channel 0x5e) | 153 x3, one unload cycle |
| 5005 | ON | 2 valid | 0 | 0 / 0 |
| 5006 | OFF | 2 valid | 0 | 0 / 0 |
| 5007 | OFF | 2 valid | 0 | 0 / 0 |
| 5008 | ON | 2 valid | 0 | 0 / 0 |

Verdict (MEASURED): hypothesis H1 (ON scores higher than OFF) SUPPORTED; the falsifier (ON not above OFF by more than the run-to-run spread) is NOT met: the lowest of the
8 ON runs is above the highest of the 6 valid OFF runs, and the mean difference is more than twice the larger within-arm spread.
Unexpected: OFF boot 5004 faulted during its first Steel Nomad run (`evidence/steel/xid-5004.txt`); ON 0 of 4 boots faulted, OFF 1 of 4. n is too small for a rate; cause not established.
The preserved 0d7ba743 bundle had dangling firmware symlinks; repointed (`evidence/steel/deviations.txt`).
Not covered: master/6d with the mirror, Linux gates G2-G4, mixed loads, GDM-off arms.

## Item 2: Energy CE witness (SPECviewperf energy-04 on candidate 6d `714ec29d3`)
Lane script `ce714-energy-arm.py` (Q4 K3 harness), SPEC copy used in place in the guest image, runs 5101-5104, production flags.

| run | verdict | host new Xid | BORN lines | `ce_seed` nonzero / zero | kernel copy lifetimes | max `seeded_launches` | NoCeObject / ForeignClass |
|---|---|---|---|---|---|---|---|
| 5101-5104 (each) | PASS | 0 | 14 | 6 (0xc7b5) / 8 | 6 | 0 | 0 / 0 |

Verdict: NO WITNESS. The assigned falsifier (seeded_launches 0 while NoCeObject appears) is not met (no refusal in 4 of 4 runs, nor in Q4's 4630-4633 on the same revision),
and `seeded_launches > 0` never occurred. CE-class-at-birth is active (`ce_seed=0xc7b5` on the kernel copy births, MEASURED). CODE READ (`crates/kf-chan/src/tmode.rs:657-662`):
the counter increments only for a CE LAUNCH decoded while `!ce_explicit_bind`; staying 0 over about 120k submissions per run means the guest bound the CE class itself (INFERRED).
Per-run counters: `evidence/energy/energy_summary.json`, `evidence/energy/510x/`. Not done: a control arm that still has the failure, master `M`, the GPU-free replay of the GP34 segment.

## Host state afterwards (MEASURED)
GDM never stopped; no qemu left; cuInit 0; host dmesg Xid count 15 -> 22 (all 7 from run 5004); owner overlay unchanged.
Scripts used (copies): `evidence/steel/{hold.sh,arm_auto.py,sn_wait.py,ui.py,guest_events.ps1}`, `evidence/energy/run_energy_more.sh`.

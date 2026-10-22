# Q3 Energy failure-rate trial plan

CODE READ: owner approved the scoped claims exception for Q3 on 2026-10-22: check Q3-added/changed files only; evidence-only branch with [skip ci]; debt ledgers unchanged. The owner confirmed 11 new trials: D descriptor-only x4, B x3, A x2, candidate x2. No product change is authorized by this evidence lane.

CODE READ: pinned revisions are D 468a35c9d845afc1938683540acc251bdef90e5a, A 40583bc297264fb57a20d84cc795d05d493197b5, B a83394e66e960d8017caa0ef19fa579f1724806b, candidate afe00b3cbc7e345e2953b1bc199d9cce1054cc38. Reuse installed QEMU builds and Q2's K1-derived harness. Adapt only lane paths/state names, deadline parameter, and descriptor test-proof selection. FullHD, energy-04 alone, one iteration, fresh CoW, identical production flags and diagnostic settings.

INFERRED H1, stated before Q3 trials: SA-02 raises the observed Energy failure rate. FALSIFIER for this trial-level claim: SA-02-bearing A+candidate have a failure fraction no greater than D+B. Small samples cannot establish population-rate equivalence or causality; report per-build outcomes and uncertainty regardless.

INFERRED H2, stated before Q3 trials: descriptor-only is immune to this failure. FALSIFIER: any valid descriptor-only trial has the same refused-channel/event153 signature. No observed failure in four trials would leave rare failures unexcluded.

INFERRED H3, stated before Q3 trials: failures are explained by monotonically worsening host state/time rather than build. FALSIFIER: an early failure followed by a late clean run at comparable host-health checks; this only rejects a simple monotonic-drift explanation, not all host-state effects. Tabulate wall times, run order, Xids, restoration/reload and health.

CODE READ: fixed order is D1,A3,B2,C3,D2,B3,A4,D3,B4,C4,D4 (run4301..4311). Controls span early/middle/late positions; A/B order is reversed in the middle. Guest153/4101, reboot, poisoned/budget refusal or vendor failure are results followed by clean restoration. A fresh host Xid aborts that trial only. Harness/health/ownership/epoch safety failures stop the campaign rather than becoming workload failures. Preserve failing channel birth/RPC, forwarded count, GP index, first guest153 time and host-log excerpt; omit raw guest payload and identifiers from public evidence.

CODE READ: five-hour lane window started 2026-10-22 09:00:58 UTC; stop work by 14:00:58 UTC. Trial controller restore deadline is 13:50 UTC, leaving cleanup time. Host GPU/VM work is serialized with bounded flock waits. No cargo build is planned; final health requires NVIDIA bound, cuInit0, GDM active, with at most one owner-authorized reload on cuInit999.

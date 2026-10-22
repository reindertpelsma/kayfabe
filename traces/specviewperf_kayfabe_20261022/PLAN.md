CODE READ: K1 compares runtime baseline 89fc8a222 with descriptor Phase 1
468a35c9d845afc1938683540acc251bdef90e5a; source branch starts at master
e64b8e5f6d0f64c41c9c4d40775e74be08de7164. No product changes or merge.

CODE READ: Phase 1 final GPU-free evidence: 1046 pass, zero fail, four existing
ignores; 13 mutants each killed by at least two tests; clippy baseline equal.
See G1 traces/descriptor_bound_20261018/phase1.md on its source branch.

INFERRED hypothesis before hardware: Phase 1 removes the Pointer descriptor
budget refusal implicated in CATIA. Falsifier: CATIA fails after large Pointer
entries are exercised, with zero budget refusals. Failure without large entries
is inconclusive. New host Xid aborts an arm; more than three TDR cycles in a
15-minute window aborts it. Event 153 records are reported separately from
cycles. No timeout, guest driver, vsync setting, or descriptor limit is changed.

CODE READ: runner files copied unchanged from VF1 a84a9994a2eb67918627b5981ad8445910b7c27b.
Known wrapper limitation: process exit may be null. Official workload status
and vendor composites are preserved separately; partial numeric subtests never
become a completed composite. The same FullHD CLI, one iteration, same workload
order and production flags from run 3440 minus KF3_PROF, plus
KF3_DIAG_PROGRESS=1000. CoW disks and copied variables protect the owner image.

CODE READ: final restore deadline is 06:45 CEST, earlier than five hours from
this session start (05:03 CEST). Start cleanup by 06:35 CEST. VFIO iterations
took about 84 minutes each; unfinished arms will be reported honestly.

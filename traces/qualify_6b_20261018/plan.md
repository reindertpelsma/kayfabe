# Q4 qualification plan, recorded before execution

CODE READ: Q4 qualifies candidate baa2977527c4509145e8033143bd664f768175a6 without changing product behavior. Evidence starts at 6e0ac33871eee5acb3bef709bdf64cea9ec4ded1 on codex/qualify-6b-20261022. Master comparison is 89fc8a222 (K1); VFIO reference is benchmark-summary.json. No merge is authorized.

INFERRED H1: candidate meets the local quality gates. FALSIFIER: any failing GPU-free full-suite test, rustfmt mismatch, newly added clippy diagnostic versus master, or failed candidate hosted CI job. Record individual counts and exclusions; do not treat CI as hardware evidence.

INFERRED H2: candidate supports the native and Linux contracts. FALSIFIER: fewer than 9/9 native gates plus gate 10, fewer than 30/30 fast-suite arms under approved exact-match intentional controls, or any new per-arm host Xid. Capture the initial journal epoch before each arm and judge only its delta.

INFERRED H3: candidate is repeatably stable under Windows production settings. FALSIFIER: any workload failure, viewperf forced kill, guest TDR/153/4101, unexpected reboot, new host Xid, descriptor Budget refusal, or stranded progress in either complete nine-workload SPECviewperf iteration or four additional Energy-only trials. Preserve exact flags, timestamps, per-arm events and counters. A guard or ownership failure stops dependent work and is reported as an incomplete qualification.

INFERRED H4: regression workloads and the master display broker remain usable. FALSIFIER: a failed installed regression oracle, broker error, missing hover/grab cursor or failed input observation. Missing harness/application is unverified coverage, never a pass. Heaven uses the documented D3D11 windowed settings; Minecraft runs ten minutes if a harness exists; Steel Nomad and the app matrix run only if present.

CODE READ: use the K1 and Q1/Q2/Q3 harnesses, adapting only paths, pinned revision and requested workload/iteration selection. Windows uses a disposable CoW of the owner's disk, run 3440 production flags without KF3_PROF, KF3_DIAG_PROGRESS=1000, and a broker built from master. Keep owner media untouched. No behavioral changes, failing-first product tests or product mutants are planned in this evidence-only lane.

CODE READ: lane start is 2026-10-22 10:41:44 UTC; deadline is 17:41:44 UTC, with cleanup reserved before it. Cargo uses the bounded shared cargo lock, nice 19, two jobs, release profile and Q4-only targets, with disk checks before every invocation. GPU/VM work uses the bounded shared fastguest lock. Final restoration requires NVIDIA bound, a fresh-process cuInit result of zero, and active GDM; one module reload is permitted if cuInit returns 999. Public evidence excludes identifiers, private paths, credentials and raw guest payloads. All pushes are Q4-only, secret-scanned, scoped-claims-clean and marked [skip ci].

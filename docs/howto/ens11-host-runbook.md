1. Build the reported ENS11 head and verify its build-input seal; use the same binary, guest snapshot, workload and baseline environment for both arms.
2. Restore Windows default TdrDelay (2 s): remove the test override, reboot, and record the effective configuration before either arm.
3. Keep the candidate's sparse/ring flags identical, including `KF3_SPARSE_TWIN=1` when reproducing the sparse path; add `KF3_DIAG_LATENCY=1` for both arms.
4. Arm A: unset `KF3_DIAG_IRQ_STORM_MS` and `KF3_DIAG_IRQ_STORM_MODE`; capture the host log, guest ETL and workload start/end UTC.
5. Arm B: set `KF3_DIAG_IRQ_STORM_MS=2 KF3_DIAG_IRQ_STORM_MODE=all`; change no other flag, binary, timeout or workload input.
6. In each arm record several host `LATENCY host_ns=... unix_ns=...` pairs and contemporaneous guest QPC/UTC pairs, including uncertainty from clock synchronization.
7. Convert host slow-record times through those pairs to the ETL clock; compare DmaDpcComplete gaps, submit/complete events and engine/node identities without assuming token 0x80c equals node 3.
8. Read heartbeat stage count/max, loop-gap and loss counters; compare `LATENCY SLOW` publication, first observed marker and logical-vector opportunities, distinguishing normal versus storm on physical IRQ line 0.
9. Read `LATENCY SPLIT` request/queue/start/apply/ready/release/resume, shared walk ID, target, diff runs and RM costs; check whether coalescing reduces walks and layout work moves after release.
10. Preserve first-slow/shutdown dumps and ETL for both arms; GPU write timestamps are unavailable here, early/failed/masked opportunities and metadata loss limit inference, and storm is an intervention rather than proof of a stall cure.

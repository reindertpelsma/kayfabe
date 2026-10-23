STATUS: LIVE, 2026-10-23; GPU-free proof green, hardware pending.

CODE READ (2026-10-23, owner02:20 instruction): readiness deadlines may adapt in this bench lane; no product/guest TDR or workload oracle change. Shared implementation: scripts/bench/windows/readiness.py. SW2.md and bw-init-query HEAD75ae71456 read; its uncommitted work handles missing CIM samples, not a scaling policy. S4 owns only its branch/worktree.

INFERRED hypothesis (2026-10-23): standard-VGA baseline boot and one successful read-only CIM roundtrip can calibrate a bounded4–10x observation profile for the same rental. FALSIFIER before a new lease: the replay tests fail to clamp scale/deadline/lease reserves, consume timed-out output, retry an unknown failure or exceed the one-retry rule; on hardware, a calibrated bind still fails to reach readiness inside the stage budget. Neither timeout outcome establishes a driver defect.

CODE READ (2026-10-23): calibration is recorded once per owned box after its generic install-check boot. Scale = clamp4..10 of ceil(max(boot_s/60,query_s/2)); all values finite and positive. Each step is capped independently and by a persisted monotonic stage end derived from the170-minute lease minus20-minute export reserve. Readiness probes may wait only inside their measured deadline; each read-only query retries a transient timeout/banner at most once, with the same stage end. Side-effecting install/enable commands are never retried.

CODE READ (2026-10-23, commit4ede56ac7): SW2 published its shared policy during proof development. S4 reused that module and its tests exactly; the common Windows CLI now delegates scaling, budget and first-query retry to it. No second policy is shipped.

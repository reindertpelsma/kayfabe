# Q3 Energy rate verdict

MEASURED on 2026-10-22, runs4301–4311, exact revisions in builds.json/table.json: all 11 authorized fresh-overlay Energy trials completed. FullHD, one iteration, unchanged K1 guest-suite hash and matching Q2 flags; validation.json checks 11/11. Host start range 2026-10-22T11:17:51.934717+02:00 through 2026-10-22T12:05:55.329907+02:00; no excluded or extra Q3 trial.

MEASURED on 2026-10-22, at the revisions in table.json: Q3 SA-02-bearing A+candidate failed 2/4, versus D+B failed 0/7. Including the five earlier standalone trials, A+candidate failed 5/8, versus D+B failed 0/8. The pooled observations are separated from Q3 in rates.md; K1 mixed3454 and candidate mixed4260 remain outside these denominators.

INFERRED H1 result: the observed SA-02 group fraction is higher in Q3, so the predeclared observed-rate falsifier did not occur. This is an association in small samples; causality for SA-02 remains unresolved. The relevant source predicates agreed in Q2’s recorded birth corpus; no matched complete RPC-stream replay was performed.

INFERRED H2 result: descriptor-only had no observed standalone failure in four trials. Immunity is not established, and a lower or rare failure rate cannot be excluded. K1’s one mixed-sequence descriptor pass is an additional reference, not a fifth standalone trial.

MEASURED at rev 40583bc2, runs4263/4266/4302/4307: A changed from two earlier failures to two Q3 passes on the same installed build. MEASURED at the run revisions in table.json: a Q3 failure was followed by later clean trials; examples run4304 FAIL then run4305 PASS, run4304 FAIL then run4306 PASS, run4304 FAIL then run4307 PASS, run4304 FAIL then run4308 PASS.

INFERRED H3 result: simple monotonic host deterioration is falsified by the failure-then-pass order. Wall time alone does not explain that pattern. Broader intermittent host state, guest submission history and ordering remain unresolved because they were not independently controlled.

| New failing run | First153 guest clock | Detected host CEST | Channel | Forwarded / GP | Birth and RPC context |
|---|---|---|---|---|---|
| run4304 C3 | 2026-10-22T09:36:16.5823597+02:00 | 2026-10-22T11:36:30.596441+02:00 | 0x806 / 0x1002d | 35 / 34 | CHAN-FACTS and RmAlloc context: [4304/channel-excerpts.txt](4304/channel-excerpts.txt) |
| run4310 C4 | 2026-10-22T10:05:16.4176729+02:00 | 2026-10-22T12:05:28.546489+02:00 | 0x806 / 0x1002d | 35 / 34 | CHAN-FACTS and RmAlloc context: [4310/channel-excerpts.txt](4310/channel-excerpts.txt) |

CODE READ: the preserved birth links the runtime token and host handle to the CHAN-FACTS allocation, including client/handle, parent, FIFO class, engine, privilege/internalFlags, context-share, TSG and VA-space. RmAlloc seq=0/held-reply and NV50_DEFERRED_API context are retained. Production logging does not identify a unique complete original RPC transaction, so that exact replay is unverified. Q2 analysis characterizes NoCeObject as a missing software decoder CE tier, not proof that the physical host CE allocation is absent.

MEASURED on 2026-10-22 in per-run channels.json: new failing channel records=2; run4304:forwarded35/GP34, run4310:forwarded35/GP34. Clean corresponding-channel records are preserved separately; do not count post-reset forwarded0 retirement as the failing channel life. The guest clock is approximately two hours behind the host; both original timestamps are retained.

MEASURED on 2026-10-22, per-run result.json and validation.json: all 11 restorations succeeded, all 33 process-observer exit controls passed, fresh host Xids0, guest4101 zero, budget/scan-budget refusals0. Q3 introduced no product behavior change and no new cargo build; cpu-proofs.json records prior CPU references. No mutation campaign applies.

| Build | Q3 exported score n | Mean | Range |
|---|---:|---:|---|
| D | 4 | 39.38 | 35.51–45.74 |
| A | 2 | 40.24 | 39.29–41.19 |
| B | 3 | 41.82 | 38.90–45.35 |
| C | 0 | unavailable | unavailable |

CODE READ: scores are SPECviewperf’s exported guest composites for reference; these samples do not qualify broader application stability or performance.

INFERRED owner decision 1/recommendation: retain the failure as unassigned; use the preserved birth/submission evidence for a bounded diagnosis before changing product behavior. No fix is proposed because the causal omission is not established.

CODE READ: raw logs and fresh CoWs remain under /data/codex-Q3/windows-20261022/; guest status, observer records and exports under /data/codex-Q3/energy-bisect/. Public evidence contains bounded text and hashes only. Final host health and cleanup are recorded separately in final-health.json and final-host-cleanup.json.

MEASURED on 2026-10-22, runs4262/4263/4266/4304/4310: failure-census.csv/JSON includes all five standalone failures, including the earlier A and candidate observations. Every failing life has forwarded35, GP34, NoCeObject/subchannel4/method0x240, and the same initial client/handle/class/engine/parent/TSG/privilege birth facts. Bounded original RmAlloc context and birth/retirement excerpts for earlier runs are in prior-public/.

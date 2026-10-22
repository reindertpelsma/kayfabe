# Q3 Energy failure rates

MEASURED on 2026-10-22, runs4301–4311: all 11 authorized fresh-overlay trials completed, nine PASS / two FAIL, with the unchanged K1 FullHD / one-iteration / energy-04 harness and matching Q2 production flags. Installed build hashes and harness provenance are in [builds.json](builds.json), [harness-manifest.json](harness-manifest.json), and [harness-adaptations.patch](harness-adaptations.patch). No Q3 build or product change.

| Build | Revision | Q3 pass / fail | Combined standalone fail / n |
|---|---|---:|---:|
| Descriptor-only | 468a35c9 | 4 / 0 | 0 / 4 (0%) |
| B: descriptor + batched walk | a83394e6 | 3 / 0 | 0 / 4 (0%) |
| A: descriptor + SA-02 | 40583bc2 | 2 / 0 | 2 / 4 (50%) |
| Candidate: A+B | afe00b3c | 0 / 2 | 3 / 4 (75%) |

CODE READ: combined counts add earlier standalone runs4262/4263/4264/4266/4267. K1 descriptor mixed3454 and candidate mixed4260 are separate references, excluded from rate denominators. No Q3 attempt was excluded or replaced. See [rates.md](rates.md), [table.json](table.json), and [run-order.csv](run-order.csv).

INFERRED: SA-02-bearing builds have the higher observed failure fraction: Q3 2/4 versus controls 0/7; combined 5/8 versus 0/8. This supports an association, while SA-02 causality remains unresolved. Descriptor-only did not fail in four standalone trials; rare failures remain unexcluded. Failures at orders 4 and 10 were followed by clean controls, which rejects a simple monotonic host-deterioration explanation. Broader host-state/time effects remain unresolved. No product fix is proposed. The predeclared hypotheses and their falsifiers are in [plan.md](plan.md); verdict and limits are in [analysis.md](analysis.md).

MEASURED on 2026-10-22, runs4262/4263/4266/4304/4310: all five standalone failures share forwarded35 / GP34 / NoCeObject(subch4, method0x240), on the initial 0xc1d00019:0xff040004 channel, token0x806 / host0x1002d. See [failure-census.csv](failure-census.csv) and the per-run channel excerpts for allocation/birth/RmAlloc context. Production seq=0 logging does not give a unique complete RPC transaction replay. First guest153 times and host detection times are retained separately; the guest clock is approximately two hours behind the host.

MEASURED on 2026-10-22: [validation.json](validation.json) checks 11/11 and all 33 observer controls passed; all 11 restorations succeeded. [final-xids.json](final-xids.json) preserves the original 538-record journal prefix with fresh host Xids 0. [final-health.json](final-health.json) records cuInit 0, NVIDIA bound, GDM active, nvidia-smi success and no reload. [final-host-cleanup.json](final-host-cleanup.json) records no active Q3 processes, eleven owned timer children removed, and successful GPU-lock reacquisition. No cargo target was created. Earlier exact-revision CPU results are references in [cpu-proofs.json](cpu-proofs.json), not new Q3 test runs; mutation testing is inapplicable without a product behavior change.

CODE READ: owner approved the scoped claims exception and the corrected 11-run quotas. Evidence-only commits use [skip ci]; [scoped-claims.json](scoped-claims.json) checks Q3-added/changed files relative to base b6f6d3010, leaving inherited debt and all ledgers unchanged. Raw logs, exports and inactive CoWs remain under /data/codex-Q3/windows-20261022/ and /data/codex-Q3/energy-bisect/. This branch contains bounded text and hashes only.

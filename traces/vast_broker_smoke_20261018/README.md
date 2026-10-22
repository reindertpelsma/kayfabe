# V3 local Worker smoke — 2026-10-22

STATUS: IN PROGRESS, 2026-10-22. Run `v3-local-20261022`.

MEASURED: 2026-10-22 run=v3-local-20261022 commit=ca637ddde; npm test passed 376/376 (0 skipped), Node v22.22.1, npm 9.2.0, temporary Wrangler 4.151.3; no repo dependency added.
CODE READ: OIDC/JWKS and GitHub API hosts are fixed; provider create uses vm=true, runtype=args, args and onstart. The earlier design STOP remains a provider-contract question; this lane expressly authorizes one tiny live smoke using the existing contract, not switching to SSH launch mode.
INFERRED: H1 before smoke: native workerd/SQLite and real read-only Vast responses satisfy the mocked contracts. FALSIFIER: startup/storage failure, response schema mismatch, redirect refusal, auth oracle, or cleanup failure.
INFERRED: H2 before local identity support: a loopback-only identity configuration enables signed local JWTs without changing public authentication. FALSIFIER: omitted/false enable flag, public request origin, or non-loopback issuer/API can activate test trust; production JWTs stop working; fewer than two tests kill a guard mutant.
INFERRED: H3 before rental: existing launch contract starts a VM and executes its fixed callback sender. FALSIFIER: create rejected, VM never runs, no onstart evidence, callback unavailable, or stop cannot establish API absence. One run only; 10-minute TTL, USD 1 daily/rolling budgets, <=USD 0.20/hour unless cheapest policy offer is dearer; independent 15-minute own-run cleanup armed before create. No provider launch-mode redesign authorized here.
CODE READ: local CPU profiling is sampling evidence, not Cloudflare billing CPU or production Free-plan certification. Route wall time will be recorded separately.

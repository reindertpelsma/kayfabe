# WK1 transactional admission evidence

STATUS: LIVE, 2026-10-26. Scope follows the owner's final cut: audit plus transactional admission; four CI lines reuse Wrangler's existing workerd dependencies. No global-state refactor, lint or admin Worker.

CODE READ at base 48c3b7d36: the production gateway sends requests to one named BrokerLedger; createService and its queue live inside that DO. The defect established here is missing storage-level admission checks. Independent service closures are a deliberate test of admission without the queue, not evidence of a live production two-isolate incident. The audit is in [globals.md](globals.md).

Hypothesis: independent request services sharing real DO storage can both pass separate preflight reads and reserve above the configured caps. Falsifier, stated before testing: simultaneous workerd requests preserve the caps/attempt/SHA/machine invariants without sharing service state.

MEASURED 2026-10-26: the original source fails all 34 negative subcases (35 TAP failures including the parent), at test commit 692a0ce0a. Four independent simultaneous /rent requests create four fake contracts under account caps of one and two. EXPECTED RED commits contain only tests, before the fix.

MEASURED 2026-10-26 on the fixed local tree: 1910/1910 existing Node tests pass; 37/37 workerd TAP tests pass (36 subcases and parent). The latter include /availability and concurrent /rent through the real default gateway and BrokerLedger, with native workerd fetch and all outbound calls intercepted. The two production-route cases pass while the independent-service cases prove the transaction works without that service queue.

CODE READ: reserve rechecks cost-key uniqueness, SHA-gap rules, branch ordering, global/profile slots, bad/reserved machines, daily count/spend, rolling spend and sampled-credit outstanding liability in a single storage.transaction. All record/open/run/SHA/cost intent writes belong to that same transaction; no provider call runs inside it. Existing preflight refusals and their metadata stay intact. The existing direct reserve unit fixture now passes policy and sampled-credit context; MemoryStorage implements a Node-only transaction model. Existing assertions were retained.

Owner rules: construct handlers from env and durable state inside fetch/scheduled; module state is only constants or synchronously computed constant caches. Static utility methods may not write global state; static fields are constants. Every await may expose changed shared state; the Durable Object is the only lock.

## Mutation check

MEASURED 2026-10-26: all 15 mutants are killed by at least two real workerd subcases each; [raw results](mutations.jsonl). The reserved-machine mutant is caught across profiles, retained cost rows are checked after record retention, and removing the transaction leaves partial intent writes at both injected failure points.

| Mutant | Killing subcases |
| --- | --- |
| account slots | account slots, account slots four |
| profile slots | profile slots, profile slots three |
| daily count | daily rentals, daily rentals three |
| daily spend | daily spend, daily spend tight |
| rolling spend | rolling spend, rolling spend tight |
| credit reserve | credit reserve, credit reserve tight |
| run attempt | same attempt, same attempt nonce, attempt row survives record retention 1, attempt row survives record retention 2 |
| SHA gap | same SHA, same SHA rerun |
| older branch | older branch discovered, older branch cross profile |
| newer branch | newer branch discovered 1, newer branch discovered 2 |
| bad-machine cap | bad-machine entries 257, bad-machine entries 258 |
| bad-machine entry | bad-machine entries 1, bad-machine entries 2 |
| configured bad machine | configured bad machine 1, configured bad machine 2 |
| reserved machine | same machine, same machine different profiles |
| transaction removed | intent rollback at sha:, intent rollback at cost: |

Reproduce locally from ci/vast-broker after installing the committed dev dependencies:

```sh
node --test workerd/admission.test.js
node workerd/mutate.mjs
```

CODE READ: mutate.mjs temporarily edits only src/ledger.js and restores it in finally; do not run it alongside tests/builds in the same checkout. Workerd, Miniflare and esbuild come from the already pinned Wrangler dependency tree; no dependency was added.

Limits: no live provider/Cloudflare behavior is claimed. Broader scheduled-handler and redirect-specific workerd coverage from original task 3 is outside the final cut. The service scheduler remains the existing lifecycle serializer; removing it safely across every report/stop/blob path is not part of this patch.

Ledger source SHA256 for the mutation campaign: `2abef20b9566ae47999e23512f4a0600c5e6c9e4ffba61d49aebe0ded5ea368f`.

# SW3 die sweep closeout

STATUS: PARTIAL / STOPPED, 2026-10-24. Branch `codex/sw3-20261024` starts at master `a9105acb6`. The containing commit is the evidence head; its exact publication status is recorded in the external SW3 lane report.

1. Requested work: extend Linux native and kf3 coverage in GA104, GA106/GA102, TU106/TU116, AD103/AD106 order; native gates 9/9 plus gate10 and bare 30/30 must precede a guest verdict. Production candidate `714ec29d3fdca0a013d9d478755346b0f1af6fa7` was planned. No binary from that revision was built or executed here.

2. Results (provider and local test receipts only):

- MEASURED (`market_summary.json`, market snapshot 2026-10-24): 54 offers returned, zero GA104 candidate names; actual dies were never probed.
- MEASURED (`lease1/create_safe.json`, `leases.json`): created 57565088 on offer 51156488 / machine 54516, advertised RTX3090; requested-disk rate 0.290000 USD/hour.
- MEASURED (`lease1/readiness.log`, `lease1/provider_failure.json`): provider state became offline before authenticated rental SSH. The later final snapshot returned created; this does not establish permanent machine failure.
- CODE READ (`market_summary.json`, `docs/OWNER_RULINGS.md` T.12): offer 51156488 was deverified, contrary to the binding verified-only rule. This was an operator selection error. The optional sweep profile and price cap did not waive that rule.
- MEASURED (`lease1/watch_ready.json`, `leases.json`): watchdog armed before create; evidence hashes checked before exact-ID teardown; API absence at 21:11:01.886994 UTC, then timer cancellation.
- MEASURED (`lease2/create_safe.json`, `leases.json`): one provider retry created 57565292 on verified offer 57543128 / different machine 145338; requested-disk rate 0.516667 USD/hour.
- MEASURED (`lease2/readiness.log`, `cleanup.json`): second lease stopped before authenticated rental SSH after discovering the first-offer policy violation. No identity check was bypassed and no GPU command ran.
- MEASURED (`lease2/watch_ready.json`, `leases.json`): second watchdog armed before create; evidence hashes checked before exact-ID teardown; API absence at 21:13:32.232312 UTC, then timer cancellation.
- MEASURED (`cleanup.json`): both exact owned IDs absent again at 21:14:08 UTC; zero authenticated rental SSH connections and zero GPU runs. No third lease.
- INFERRED (`cost.json`, recorded allocation/absence times and rates): combined runtime estimate 0.025877469 USD, below stop 3.00 / cap 3.50. This is not settled billing.
- MEASURED (`cleanup.json`, `cost.json`): shared credit 23.670138 USD at closeout, above the untouched 8 USD reserve; account delta is not exclusive lane spend.
- MEASURED (`payload_tests.log`): existing GPU-free payload tests 140/140 passed; no committed source changes or mutation campaign.
- CODE READ (`operator_plan.json`, `leases.json`): planned native/guest falsifiers remain untested. No native exclusion or kayfabe failure is established.

3. Matrix and tests:

All driver versions, flags and the production revision below are plans. None was observed on a GPU in this lane. `UNRUN` is distinct from a native failure or an excluded native cell. BAR1 and Xid deltas were unmeasured.

| Die | OS | Host / guest driver | kf3 revision | Flags | Status | Evidence |
|---|---|---|---|---|---|---|
| GA104 | Linux planned; native binding first | unmeasured / unmeasured | planned 714ec29d3 | KF_DEVICE=kf3, KF_FROM_HOST=1 planned | UNRUN, no candidate offer | market_summary.json |
| GA102 candidate; actual die unknown, lease 57565088 | Linux planned | unmeasured / unmeasured | planned 714ec29d3 | same, unexecuted | UNRUN, provider pre-SSH / selection violation | lease1/, leases.json |
| GA102 candidate; actual die unknown, lease 57565292 | Linux planned | unmeasured / unmeasured | planned 714ec29d3 | same, unexecuted | UNRUN, lane stopped pre-SSH | lease2/, leases.json |
| GA106 | Linux planned | unmeasured / unmeasured | planned 714ec29d3 | same, unexecuted | UNRUN, no lease | operator_plan.json |
| TU106 / TU116 | Linux planned | unmeasured / unmeasured | planned 714ec29d3 | same, unexecuted | UNRUN, no lease | operator_plan.json |
| AD103 / AD106 | Linux planned | unmeasured / unmeasured | planned 714ec29d3 | same, unexecuted | UNRUN, no lease | operator_plan.json |

Hardware n=0; gates, gate10, bare suite and fast suite all UNRUN. No before/after or spread exists. Mutation table: not applicable; no product or committed harness behavior change. The 140 existing local tests are preparation evidence only.

4. Owner decisions:

1. CODE READ (`docs/OWNER_RULINGS.md` T.12, lease57565088): recommend keeping all matrix cells UNRUN. A new explicitly authorized continuation should require verified host and verified VM status in the selection assertion, with all inherited limits retained. This lane stopped under COMMON's contradiction rule; no further rental was used to repair the operator.
2. Recommend retaining machine 54516 in `known_bad_machines.json` for the observed pre-SSH failure; its permanent health and the cause are unknown.

5. Unverified: actual dies, host and guest driver versions/flavours, native binding, BAR1 aperture, gates, bare and guest suites, actual production binary identity, Xid deltas and final invoices. No Windows installation, driver change, native probe or guest boot occurred.

6. Cleanup: MEASURED (2026-10-24, lease57565088, lease57565292, `cleanup.json`, `leases.json`): own IDs 57565088 and 57565292 are API-absent; timers cancelled after absence. No owned target directory, rental workload, GPU job or lock was created. Read-only inventory queries on the trusted host changed no files and ran no GPU work. The owned readiness process was terminated. Raw provider captures remain in the lane's prescribed private capture directory; only projected provider fields, text and hash manifests are committed. No secret, executable, image, closed-binary trace or other session's resource was imported or acted on.

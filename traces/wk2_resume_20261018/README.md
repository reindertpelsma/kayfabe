# WK2 composed workflow validation

STATUS: REVIEW, 2026-10-28.

CODE READ (`source-sha256.json`, `validation.json`): this batch merges the original A and B histories into current approved next. The only textual merge conflict was the Site README; current claims rules remain intact. No Worker source, bundle, config or secrets change.

MEASURED 2026-10-28 (`validation.json`, source4150f6990): composed Node2679/2679, real workerd105/105, Site129/129 with pinned Zola/mdBook render and zero broken internal links; no skips. Documentation freshness111 tests pass. Claims new=0; debt files unchanged. These are local mocked/runtime/build checks, not a live workflow or provider verdict.

CODE READ (`gate_reuse.py`, `site_policy.py`, `release_gate.py`): review fixes bracket mutable PR pagination with head/base/count checks; require one complete fixed-schema original verdict with both CUDA modes and four unique stages; bind every qualifying CI check to its intended workflow run/job/check suite; keep copied required status pending until final report and independently require child completion for release. All successful push/PR CI duplicates are accepted; conflicting or incomplete records disable reuse.

MEASURED 2026-10-28 (`validation.json` RED commits): F1 has12 expected failures before its fix. F2 stage identities have6 expected failures/84 controls; extended schema has15 behavior failures plus3 fixture-order errors corrected in the next fix. F3 has30 expected failures/192 controls. F4 has18 expected failures/126 controls. Original RED histories and historical mutation evidence remain preserved in their original trace directories.

MEASURED 2026-10-28 (`public-parent-proof.json`, run40588779558): the stricter final helper accepts the recorded e54bab555 to858ca1e8 same-tree evidence, including host/guest starts and merge_bar rc=6 result=PASS. This read-only query/download initiated no provider or Worker request.

MEASURED 2026-10-28 (`extra-mutants.json`, `gate-mutants.json`, `site-mutants.json`): all69 isolated mutants have at least two named killers each. The original22 gate and27 Site mutants were rerun on the composed code;20 extra mutants cover the review fixes and Site skip cases.

Reproduce ordinary checks from the repository root:

```sh
node --test ci/vast-broker/test
node --test ci/vast-broker/workerd/*.test.js
python3 -m unittest discover -s site -p 'test_*.py'
node ci/vast-broker/workflow-guard.js
python3 scripts/ci/debt.py claims
```

Build the Site with its pinned tools before the full Site discovery (output tests require generated HTML). The extra mutants use `python3 traces/wk2_resume_20261018/mutants.py`. The original gate/Site mutation runners can be evaluated with their output path redirected to this directory to retain the historical JSON; every mutation uses an isolated copy and simulated APIs.

UNMEASURED as of 2026-10-28: live fork status delivery, Pages deployment, copied GPU workflow execution and release dispatch. No rental, dispatch, deployment, protection change or merge is part of this validation. F4 closes a source-ordering window; no live hardware-proof bypass was demonstrated.

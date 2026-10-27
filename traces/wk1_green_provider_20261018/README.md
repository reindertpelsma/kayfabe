# WK1 GATE-GREEN: bounded provider replacement

MEASURED 2026-10-26 at EXPECTED RED rev0913479ae:12 of16 Node/BASE-runner cases failed;
the real-workerd replacement race cases admitted zero replacements (2 failures,12 rejection
controls passed). Five Python cases showed missing classification/forwarding. Boot fixtures
were then refined to require the exact owned-instance frozen error rather than equating
no callback with provider failure.

MEASURED 2026-10-26 at EXPECTED RED rev9540c554d:2 of26 cases failed because a replacement's
actual contract could exceed the original allowance. MEASURED 2026-10-26 at EXPECTED RED
rev e78344e85:6 of28 cases failed because the replacement box env retained the original nonce.
Both defects are covered independently from the original replacement tests.

CODE READ from source in this branch: the transactional replacement retains prior spend/count,
requires ended cleanup proof and its exact prior nonce, admits one fresh machine/capability,
and preserves original time/money limits. Public behavior and boundaries:
[provider retry](../../docs/howto/vast-provider-retry.md).

Run `python3 traces/wk1_green_provider_20261018/mutate.py` alone from repository root;
all live source and mtimes are restored in finally. An initial one-line removal of the ended
check survived because receipt consistency and spend checks independently refused it. The
meaningful mutant instead trusts a settled cost receipt over the still-unended run record;
the source checks are retained. Final results belong in mutations.jsonl.

The full local Python suite reproduced the previously recorded unchanged transport control:
`test_outer_deadline_kills_a_stuck_transport` takes about2.01s against its1.5s assertion on
this dev-box environment. No timer/poll change or test weakening. Changed payload cases and
provider/runner/workerd checks are checked separately; hosted full-suite results remain owed.

No deployment, Worker setting edit or direct provider API call.

MEASURED 2026-10-27 on this branch before publication:2257 Node tests,67 real-workerd tests,
208 existing payload tests and9 new provider Python tests pass. All89 existing payload
mutants and27 provider mutants have at least2 independent killing tests (minimum2).
Evidence: [provider mutants](mutations.jsonl), [existing payload mutants](payload-mutations.json).
The final source bundle is7d6cbb536b550db511b7b4fde13d7acc575747dfdd05c31fd1822ee4beda5e86.
The EXPECTED RED rev87a21c204 additionally demonstrates4 failures: rejected actual-contract
spend retained the offer price, and a replacement ending without a result lost its provider
classification. Actual product failures on a replacement remain FAIL.

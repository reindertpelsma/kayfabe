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

MEASURED 2026-10-27 at original PR159 rev8cbd2a7e8:2257 Node tests,67 real-workerd tests,
208 existing payload tests and9 new provider Python tests pass. All89 existing payload
mutants and27 provider mutants have at least2 independent killing tests (minimum2).
Evidence: [provider mutants](mutations.jsonl), [existing payload mutants](payload-mutations.json).
The source bundle at rev8cbd2a7e8 is7d6cbb536b550db511b7b4fde13d7acc575747dfdd05c31fd1822ee4beda5e86.
The EXPECTED RED rev87a21c204 additionally demonstrates4 failures: rejected actual-contract
spend retained the offer price, and a replacement ending without a result lost its provider
classification. Actual product failures on a replacement remain FAIL.

MEASURED 2026-10-27 at fix rev44842eff5: all33 provider mutations have at least2
independent killing tests; current evidence is [mutations.jsonl](mutations.jsonl).
The owner incident fixture is [provider-57890737.txt](../../ci/vast-broker/test/fixtures/provider-57890737.txt).
Its NOT_READY/GPU-unusable/RmInitAdapter failure no longer needs a separate NVIDIA_SMI_RC line.
The new real-workerd incident round trips exercise exact-id cleanup, machine exclusion,
fresh host/nonce, stale-capability rejection, one replacement, two dollar receipts/one daily
attempt, and admission at256 bad-machine rows followed by refusal above256.
The UTC-day and incomplete-marker controls are in green-attempt-budget.test.js.

CODE READ at fix rev44842eff5: the replacement keeps its original configured profile dollar
cap and deadline, and the first-admission185-minute BUSY guard does not apply to replacement.
Fresh ordinary attempts still use that guard. No timeout or poll interval changed.

MEASURED at commit 7381c38b0, incorporating master 7eec1e79e: 2277 Node tests, 75 real-workerd tests and 11 provider Python tests pass.
The combined source bundle is2c789c616ea87534c5ee41fa9ddc825606c90ae0215d34d08864d922fbbc7acc.
The repaired canonical byte-reserve mutation passed its baseline and both named killers;
full hosted box-script CI is still pending for this update.

MEASURED at EXPECTED RED commit 4caef0fe9: two failed-provision cases with complete clean watcher evidence fail through the actual stage_grade wrapper. The following correction passes208 existing payload and21 provider tests; all38 provider mutations and89 existing payload mutations have at least2 killing tests (mutations.jsonl and payload-mutations.json).
CODE READ: only an already-classified provider failure can retain its matching failed-command
exit with complete clean watcher proof. Changed/missing evidence, reserved exits6/7/124 and
contradictory statuses remain stops. Default product-pass checks still reject failed commands.
The live PR158 host_xid result exposed this parser limitation; its complete watcher receipt
was not available in the public summary, so no claim is made that its actual Xid delta was clean.

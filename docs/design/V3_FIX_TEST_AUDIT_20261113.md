**STATUS: LIVE, 2026-11-13.** Audit of the fixes merged to `next` since 2026-10-22 (first-parent PRs and fix commits) for
missing or weak regression tests. Verdicts are MEASURED where a mutant was run (the mutant removed only the fix lines;
`CARGO_TARGET_DIR` outside the tree, GPU-free targets) and INFERRED where only the diff and the test names were read.
Nothing here touched a GPU, a Vast box or the PC.

# Fix test audit, 2026-11-13

Verdict words: COVERED (>= 2 independent killers), WEAK (source-text pins or one killer), MISSING, NOT-TESTABLE (hardware
signature named). "Killers" = tests that fail when the fix lines are reverted.

## Table

| Fix (PR / commit) | What failed | Pinning tests | Verdict |
|---|---|---|---|
| kf-chan kernel CE ring decodes against host class (#356, energy-04 / `NoCeObject`) | run 3017: kernel COPY2 ring poisoned, GP 34 | `tmode_ce_tests` (10 tests), `tests/ring.rs`, `birth_ce_object` unit tests. MEASURED: reverting `ce_tier()` to `tier` at the `NoCeObject` site fails 10 tests | COVERED. The kf-qemu birth wiring (`ring.set_ce_object`) is a source-text pin only (INFERRED, a revert there passes the unit tests) |
| kf-chan Phase 2 streaming, descriptor bound (#320, #360; creo-04) | oversized user-ring segment refused after migration | `user_turn_unit.rs` (~2000 new lines), props, `scripts/descriptor_mutation_check.py`, `scripts/stream_mutation_check.py` | COVERED (INFERRED from the repo's own mutation runners; I did not re-run them) |
| `kf-qemu` contended slot gives the ring back (`899b0b5ba`) | TUR1 HOST-FENCE-OVERDUE, stranded fence | `hostidx_model.rs` behavioural model, `HostIdx` unit tests, source guards on `serve`/`owe_ring` | COVERED (INFERRED) |
| Windows USER ring admits Turing 0xc46f (`cf58769c0`) | every Turing channel refused | predicate tests (all `0x??6f` ids), class tests, source guard | COVERED (INFERRED) |
| Walk wake, adaptive wait (#241, `4f420b373`) | walk collected only every 1 ms | `completion::tests` (age to wait table, never spin, idle backstop). MEASURED: `clamp` replaced by the max fails one test | WEAK by the rule (one killer); the wiring `PollTimeout::Micros(completion_poll_us())` in `device.rs` has no test. Latency itself is NOT-TESTABLE without a GPU: a lane should assert `walk_avg_us` far below 1000 for ~85 us walks |
| kf-disp class pushbuffer format, 64 KiB on GB20x (`450e1c276`) | GB203 core ring halted at PUT 0x1d70 | `blackwell_pushbuffer` (2 killers), engine tests. MEASURED | COVERED |
| kf-disp GB20x tile type 0 (`54410b7db`, `d76bf9a62`) | Blackwell display never modesets | `ca73_postcomp_caps`, `blackwell_caps_topology`. MEASURED: dropping the two bits fails 1 test (cargo stops at the first failing binary, so `blackwell_caps_topology` was not run against this mutant) | WEAK by the rule until the second target is run per mutant (INFERRED COVERED) |
| kf-abi answers the Windows guest's 0x20809064 / 0x2080852c (`e04a7d445`, #253) | 52 of 53 calls refused | two row tests, a host-control size test. MEASURED: changing the 852c row's input fails 1 test | WEAK by the rule for this mutant (the size mutant would fall to the hostabi test, INFERRED) |
| `--concurrent-fuzz` control seed stepping (#254) | 0.79 % of seeds red the instrument | two tests incl. the measured red seed | COVERED (INFERRED) |
| kf-chan / userstore stress flakes (#244, `5b00345a0`) | injection kinds did not fire under load | the stress test itself now loops until all kinds fire | COVERED by construction (INFERRED) |
| kf_fetch verify-then-rename (#348), setup cache timeout probe (#339), DNS fallback (#340), Vast payload Xid/native fault (#332, #321) | see PRs | `test_kf_fetch.py`, `test_vast_setup_build.py` (2 tests), `test_vast_setup_dns.py`, payload tests plus `*_mutants.py` runners | COVERED (INFERRED) |
| ignore ladder / stripped tree (#326, #329), GPU UUID scrub (`8dbd3227f`, `004fa0b1a`) | see PRs | `test_ignore_ladder.py`, `test_box_stripped.py`, mutants scripts, scrub tests | COVERED (INFERRED) |
| Windows gate: QMP event logger retries its attach; `no-events-file` is OPERATOR (`4300fc5d8`, gate race `d8119db77`) | GA106 run dca106a: logger never attached, second QEMU never started, FAIL instead of RETRY | none before this audit | MISSING, now COVERED: `scripts/bench/tests/test_qmp_events_attach.py` (below) |
| Windows gate: BAR1 refusal read only after `realize refused` (`d18032f3e`); no-flags signature is a REFUSED line (`a5071c24b`) | a log naming `bar1-size` could mask a real failure as the Vast BAR1 exception; a served trace line failed a PASS | the gate `--selftest` stubs both (`bar1_refusal() { return 0; }`, hand-written `gsp_signals.txt`), so the real greps were untested | MISSING, now COVERED: `scripts/bench/tests/test_windows_gate_signals.py` (below) |
| Worker (`ci/vast-broker`, `ci/vast-reaper`) fixes (#277, #278, #338, #344, #355, #361 and the review-fix commits) | see PRs | read only, not audited (reserved for the #362 author) | NOT AUDITED |

## Tests added

Both files are new, GPU-free and need no box. Complicated-setup tests (fake clock, extracted shell functions) were run
against the true parent of each fix and against the fix (owner rule 2026-11-13).

1. `scripts/bench/tests/test_qmp_events_attach.py` (11 tests): attach retried until the greeting arrives; each failure
   kind retried; gives up at the deadline and never creates the events file; attach window from `KF_QMP_EVENTS_ATTACH_S`;
   a failed handshake closes its socket (mock and a real unix socket); the gate's reboot-wait loop (cut out of
   `windows_gate.sh`) calls `fail_op` on `reason=no-events-file`, `fail_guest` on a guest that really did not restart.
   - Local run 2026-11-13 at next 56adcfaa3: against the parent of `4300fc5d8` 11 tests fail or error; against the fix 11 pass.
   - Mutants (MEASURED): single-attempt attach fails 7 tests; no `s.close()` fails 2; `fail_op` to `fail_guest` fails 2.
2. `scripts/bench/tests/test_windows_gate_signals.py` (10 tests): `bar1_refusal` and the `refused_0x2080121f` /
   `served_0x2080121f` counters of `collect_evidence`, each run on fixture logs.
   - Parent of `d18032f3e`: the BAR1 test fails; parent of `a5071c24b`: 4 tests fail or error (MEASURED). Fix: all pass.
   - Mutants (MEASURED): old BAR1 regex fails 2 tests; counter without the `REFUSED` filter fails 3; dropping
     `! ssh_seen` fails 1 (WEAK: a single killer for that sub-clause).

## Findings

- No test written here failed on the current `next`; no fix was found incomplete.
- WEAK items that are not worth new code now: the `device.rs` walk-wake wiring and the `kf-qemu` CE birth wiring (source pins
  or none; a hardware lane covers them: `walk_avg_us`, and a Windows kernel COPY2 ring running without `NoCeObject`).
- Suggested follow-up for the owner of the touched crates (not done, those files are shared): a second killer for
  `completion::poll_us` (assert the monotone age to wait mapping for ages 0..2000 us), and run `blackwell_caps_topology` in
  the tile mutant.

## Not checked

The 64 master first-parent entries were not read separately from `next`. The Worker fixes, `kfhost`, `kfseed`, `kfml` internals,
the site, and the Vast payload python mutants were not re-run. The Phase 2 mutation runners were not re-run. Traces-only
commits were skipped.

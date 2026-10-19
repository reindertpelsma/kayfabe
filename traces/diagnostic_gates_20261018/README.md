# Diagnostic gates, 2026-10-18

MEASURED: source base `b233037d95da2ae157cfcf50066fb500991c5b7b`.
Tests-only commit `8591194e6783b9b186eacdb31aab7ed46c96277e` deliberately
fails seven Rust assertions and five broker clock assertions; four explicit-on
Rust oracles pass. No Rust compile failure was used as red evidence.

Falsifier stated before the runs: an off-path clock call, sample, ring allocation,
dump, altered request decision, or changed explicit-on fragment/text disproves the gates.
MEASURED: green release suites: kf-qemu 325 passed (base 318), kf-broker 160
(base 151), kf-disp 247 (base 247); zero failed, 12 ignored across these crates.
Broker `make check` and `make check-sanitize`, including Python socket tests,
pass. The five new stats-clock assertions pass in both.

MEASURED: forced-gate mutants, distinct failed tests:

| Mutant | Kills |
| --- | ---: |
| step forced on, default off | 3 |
| step forced off, explicitly on | 3 |
| ring forced on, default off | 5 |
| ring forced off, explicitly on | 3 |
| broker stats clock forced on | 2 |
| broker stats clock forced off | 2 |

CODE READ: only LatStat samples/clock pairs, the formatted `lat[...]` suffix,
EventRing contents/dump bookkeeping and diagnostic `declined_for` dedup are gated.
`last_seq`, `last_present`, `open`, `refresh_ns`, `last_answer_ms`,
`ticks_since_open`, TickCounters, frame credits, live/silent transitions and the
broker poll/errno decisions remain active. The existing periodic FPS fragment is
unchanged. Two old ring tests explicitly enable diagnostics to retain their original
bounded-ring assertions; no decision assertion was weakened. A source wiring test
normalizes whitespace after rustfmt, retaining the complete original argument sequence.

CODE READ: no vCPU wait, lock, unsafe block, timeout change or new boot requirement
is introduced. Configuration is read once per construction, never per event.
The on-path ring remains bounded to 48 entries and six dumps. MEASURED: rustfmt
checks pass. Clippy reports existing denied arithmetic/indexing lints outside the
modified gate code; candidate-4 comparison is part of the integration verification.
No hardware or VM claim is made here.

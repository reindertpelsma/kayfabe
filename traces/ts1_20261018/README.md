# TS1 GPU-free evidence

The base is `3bfb0965640f12c3971293b2b1b2d989d6e0cfef`. No host, VM or GPU operation is part of this evidence.

CODE READ: the original early-empty branch returned NotOurs; its before-queue member snapshot could miss a queued birth. The test-first commits add inert seams and assertions before runtime integration:

| Commit | EXPECTED RED result |
|---|---|
| `06fe5db1e` | Replay 19 failures / 5 passes; mirror composition 8 failures; six production wiring failures; four RM class/lifetime failures |
| `5e6de253c` | Both new queued-ticket cap assertions fail; freed keys cannot be allowed to hide outstanding acts |
| `45e6a60fc` | Default-off facts: 11 failures / 1 pass |
| `d1847d94f` | Native readback layout/refusal seam: four failures |

MEASURED, TS1 GPU-free run on 2026-10-26, evidence `traces/ts1_20261018/validation.json` and `traces/ts1_20261018/mutations.json`: the green suite and the hand-mutant results are summarized in `validation.json` and `mutations.json`. The mutation harness compiles copies of the actual state/fact modules and the unchanged mirror rules, exercising the actual test bodies with fake RM. Production source-composition and RM-link mutants are listed separately. A compile error is never counted as a killing test. `source_hashes.json` pins the tested source text.

CODE READ: `crates/kf-qemu/src/tsgmirror.rs` and all pre-existing mirror/lifecycle tests have no diff from the base. The new mirror tests cover explicit several-ctxshare groups, legacy members, mirror-OFF shards, free ordering, ctxshare cap, refused member allocation, and contradictory modes. They do not validate real firmware or imply a performance benefit.

Hardware is outstanding; the correctness-first plan and fact/readback limits are in [the design note](../../docs/design/V3_TSG_TIMESLICE_REPLAY.md).

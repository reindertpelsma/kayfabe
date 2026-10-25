# PF1 interrupt-chain census

STATUS: LIVE, 2026-10-25. CODE READ: `crates/kf-qemu/src/intrdiag.rs` and its
call sites add observation only. Interrupt decisions still come from `CpuIntr`;
there is no single-flight suppression, marker removal, or deferred mask write.

Ordinary builds omit the `diag-intr` feature and compile out every observer hook.
For an instrumented binary, use the normal host build flow with
`KF3_BUILD_DIAG_INTR=1` and `nice -n 19`. At runtime only the exact value
`KF3_DIAG_INTR=1` allocates the census. With that variable absent, the diagnostic
binary performs no counter updates, timestamps, snapshots, or formatting; its
hooks only check the absent observer. No diagnostic flag is needed to boot.

CODE READ: the bounded observer uses atomic counters and fixed vector arrays.
There is no observer lock, allocation, or output on a vCPU event hook. The existing
status heartbeat prints `DIAG_INTR unix_ns=...` followed by JSON: cumulative
`counts`, `at_ns` on the census-relative monotonic clock, `window_ns`, and `per_s`
rates since its preceding report. Scalar zeros, including `stranded`, are printed;
zero vector bins are omitted and mean zero. Status snapshots are independently
atomic observations, not a transaction across all producers.

| Counter | Definition |
|---|---|
| `marker_{gate,batch,staged}_{nsi,plain}` | Successfully published authored tagged-reference markers, by site and interrupt payload. All three current emitters carry NSI; their plain bins remain zero. Guest methods are not inspected. |
| `edge_{engine,fifo}` and `_vN` | Received fd readiness callbacks, including unarmed events. These are dataless notifications and can already be coalesced by RM. The shared completion fd includes asynchronous host CE as well as FIFO_EVENT_MTHD. |
| `source_*` | Latch request origin; `source_tree` counts messages requested by guest tree writes. |
| `messages`, `raise_vN` | Logical message requests and contributing CPU-tree vectors. A root unmask can contribute several vectors to one message. Vector 512 is unmapped/unknown. |
| `eventfd_v0`, `eventfd_errors` | Actual successful and failed eventfd writes on physical MSI-X vector 0, independently of logical attribution. |
| `held`, `out_of_range`, `was_asserted` | Masked latches, refused vectors, and latches whose vector was already enabled and pending. |
| `cycle_starts`, `isr_cycles`, `isr_ns` | First message through the next changing TOP_EN_SET root unmask, using the Windows tree protocol. Repeated messages do not restart the clock. This is an observed tree cycle, not ETW proof of ISR execution. |
| `merge_candidates` | Another message to the same guest before any first trapped tree write since its preceding message flight. Reads do not close the flight. |
| `first_tree_writes`, `first_write_ns` | First trapped writes and summed oldest-message-to-first-write latency. |
| `write_*`, `read_*` | EN_SET, EN_CLEAR, TOP_SET, TOP_CLEAR, W1C, trigger, and TOP classes. Only already-trapped reads are observable; ordinary shadow reads remain untrapped and uncounted. |
| `stranded` | Observed enabled pending flights with no first tree write for strictly more than 1 ms, counted once per flight. It diagnoses overdue service, not proof that work or an interrupt was lost. |
| `resets` | Interrupt-tree reset boundaries. Temporal state clears; cumulative evidence remains. |

For a workload frame window, subtract two cumulative snapshots and divide all
counts by the workload's presents in that same interval. `Snapshot::since`,
`per_second`, and `per_frame` implement that arithmetic in the simulation. Actual
ETW presents and ISR entries must use the same clock-mapped boundaries. Boot
totals must never be divided by workload frames. A window spanning a reset or
containing eventfd errors is not an accepted attribution window.

MEASURED: `expected-red.txt` records two failing wiring tests on the base before
the implementation (tests-only commit `0a586ede8`). Runtime contracts were also
committed there before the new feature existed. MEASURED: `validation.txt` records
the host test, format, clippy, and mutation receipts for this implementation.
Mutation results are in `mutation-results.json`; every listed mutant must have
at least two distinct killing tests. `source-hashes.json` binds the reviewed files
to the host test source copies. Hardware validation has not been performed.

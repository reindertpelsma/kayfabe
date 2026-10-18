# P1 Steel Nomad parity pre-registration

STATUS: RESEARCH, 2026-10-18. Lane P1; correctness is the release gate.

## Scope and provenance

CODE READ: lane instructions authorize GPU-free Part A immediately. Hardware starts only after the TSG mirror DONE marker or 04:30 CEST, subject to the shared lock and the TSG owner's exclusive use. Hardware stops at 06:30 CEST on October 19; report due 07:00.

CODE READ: branch starts at production candidate-4 `0452613b5`. Evidence sources are read-only: Steel Nomad gap `4f37bd12c`, submit latency `e97a35ffb`, subcontext feasibility `61dc81b73`. These measurement branches are not the production tree; no merge or TSG/channel-birth change is authorized here. Harness files may be copied with source attribution. No closed-binary analysis is a source.

MEASURED (prior reports, not repeated by P1): VFIO scores 3841–3962 versus kayfabe 2369–2520; tiny DIRECT/COPY/COMPUTE medians 834/231/1485 us. The prior per-hop study assigns DIRECT 390 us to last-marker→first-host-interrupt and 234 us to the guest ISR chain. INFERRED: these costs explain the Steel Nomad fixed dependent-step overhead; this lane must test that link.

## Hypotheses and falsifiers, recorded before new inspection or runs

1. W1: separate host TSGs cause most of the GPU-leg excess. FALSIFIER: a mirror binary that passed its owner's gates leaves DIRECT/COMPUTE GPU legs at or above 300/600 us, or Steel Nomad does not improve beyond measured spread. Check the mirror report first; failed gates mean use production for W2–W4. Do not modify or merge mirror code.
2. W2: multiple host event sources or late delivery cause avoidable guest ISR cycles. FALSIFIER: source/vector/time/arming records show distinct required events, or removal of independently proven redundant forwarding leaves ISR calls/frame and fps within spread. Owner ruling X requires every armed host edge, including cross-tenant edges, to survive. An identical vector alone never proves redundancy. Diagnostic output must distinguish FIFO, engine, paced trailing delivery and IRQ emission; retain raw counts and trace saturation.
3. W2 pacer experiment: loss-free pacing at 20 or 50 us reduces ISR cycles/cost relative to 0. FALSIFIER: no improvement beyond spread, any lost trailing delivery, stalled probe, TDR or new host Xid. Never change the default or turn this experiment into a timing fix. Read and run existing loss-free tests first.
4. W2 backstop: a worker safety-net discovery indicates a missing event. FALSIFIER for event-complete operation: any backstop finding of actionable work during Steel Nomad. Record existing counters; investigate its missing notification rather than change a timer or poll GP_PUT.
5. W3: a cheaper mask/unmask write path can preserve synchronous state application. FALSIFIER: the proposed path cannot apply the arbitrary written mask/W1C value before an immediately following guest read, or needs a blocking operation on a vCPU. Report only unless equivalence is proved and red-first tests plus hardware gates can be completed.
6. W4: lazy observe, PUT caching or guest GET write-back contributes materially. FALSIFIER: DIRECT/COPY/COMPUTE and Steel Nomad remain within spread with lazy observe disabled, PUT cache disabled, or (separate opt-in experimental arm) GET write-back suppressed except the full-ring guard. Every arm changes one variable. A stall/refusal is a correctness failure, never a performance result.
7. W5 (conditional on W1 residual): GPU-leg delay can be localized using bounded default-off channel timestamps. FALSIFIER: available timestamps cannot distinguish doorbell, fetch and interrupt or their measurement perturbation dominates. Do not claim a GPU execution timestamp from a CPU observation.
8. Diagnostic feasibility: forwarding can be observed without blocking a vCPU or a lock a vCPU takes, with no production measurement code. FALSIFIER: required hooks run under such a lock/on a vCPU, cannot be buffered with a fixed bound, or production builds retain measurement work. Use compile-time exclusion plus an optional default-off KF3_DIAG_* runtime flag if feasible; record any trace losses explicitly. Failing-first tests precede implementation.

## Run order and controls

GPU-free: binding docs and evidence; W2/W3/W4 source paths and prior art; red-first diagnostics tests (EXPECTED RED commit alone); fix; crate tests before/after, mutants killed by at least two tests each, formatting and clippy; immutable revision host build using documented build flow. Push and verify the lane branch after each stage; secret-scan each diff.

Hardware: mirror readiness/report first; stop the owner's VM only through its stop marker, then bounded acquisition of the shared GPU flock. Record before/after host Xids, source revision, binary identity, complete flags and disk. W1 first; W2 edge attribution next; pacer 0/20/50; lazy-observe default/0; PUT-cache default/0; GET experiment only after its tests/gates. Same disk/session, restart between arms or alternate/order-balance. At least three independent runs/arm, five if spreads overlap. Report medians and min–max; never replace failed runs silently.

Any behavior candidate: touched-crate release tests and >=2 kills/mutant; v3 gates 9/9; fresh raw-client/guest fast suite 30/30; Windows desktop plus 15-minute Steel Nomad/Heaven loop, zero new System-4101 and Xid; Heaven unchanged. Unproven behavior stays opt-in and never becomes morning production.

Metrics: Steel Nomad score/guest fps; broker new-content fps, 1%/0.1% lows from content-gap percentiles, >33/>50/>100 ms gaps/min and max, repeat presents/s; guest-vs-content counts on capped runs. Collect ISR calls/frame and durations, tiny-probe per-hop times, source/arming/vector counts and backstop counters. Missing observability is stated, not inferred as zero.

Morning: restore production `0452613b` unless one exact candidate passed every promotion criterion in the lane instructions. Never replace an existing binary. Record exact recovery commands and final VM readiness in the required morning note. Release P1's lock, remove P1 target artifacts, leave no P1 measurement process running. No other session or agent is contacted.

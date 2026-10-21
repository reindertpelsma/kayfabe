# HW2 independent review, working notes

CODE READ, B' c452ca7cb: `user_loop::Found` and `found(Wait)` no longer exist.
The old base classified Popped as Carried, Contended as Contended, Marker as
Work. B' instead increments both WorkerStats::timeouts_with_work and
Stats::backstop_found_work at worker.rs:476-479 when `after_timeout &&
(n > 0 || rang)`. The decision does not retain wait identity or reason and
does not increment either legacy carried/contended bucket. Thus zeros in those
two buckets do not establish which obligations were found.

MEASURED (read-only host log extraction, inherited runs): YT37 final work=366,
carried=0, contended=0; YT50 final work=662, carried=0, contended=0. These are
cumulative final summaries, not sums of the three summaries in each boot.

MEASURED UPDATE 2026-10-21: the real-worker probe committed alongside these
results reproduced this observation race twice on exact B'. Fresh direct-token
work and fresh relay demand, each with an already-signalled doorbell, both
incremented work=1 and timeouts_with_work=1 while completing successfully.
Neither case had a Marker/Popped/Contended obligation. This proves the counters
can count normal Work; it does not attribute the inherited hardware instances.

Original INFERRED mechanism: a new normal ring arriving after a timed-out epoll_wait returns and
before the next worker_pass can increment this counter despite a ready wake
source. Likewise rang may be pop writeback, record/storage busy, demand, or a
satisfied marker. Existing logs cannot attribute the 1028 hits to a wait type
or distinguish a missed wake from that observation race. This does not waive
the lane's literal >0 falsifier; B' cannot advance to Windows.

CODE READ, M 89fc8a222: userstrand.rs resets its age on a touch, not on proven
GPU progress; requires aggregate loop counters to move; caps start events at
64 and end events at 128 using a shared count. A stalled turn or all-workers-
blocked case can be silent. Unlike B', M does not carry progress.rs's
worker_stuck monitor. The Windows diagnostic flag on M therefore does not
promise that detector. Preserve independent guest and host thread evidence.

CODE READ, M test scope: user_loop_model.rs explicitly substitutes
legacy_look_needed and legacy_idle, so its green count tests the historical
design and is not fresh production sleep-safety coverage. v2, fairness,
handoff_protocol and qemu PtLock tests must be assessed separately.

CODE READ, grader adapter: grade_adapter.rs only implements simple,
look_every and budget. Its comment incorrectly associates the current M branch
name with the old LOOK_EVERY/budget lineage. Current M has none of Look/LookIn,
LoopPolicy, set_park_net or may_retry. An unmodified overlay cannot grade the
handoff lineage faithfully. Do not relabel it as the old budget lineage.

RESOLVED 2026-10-21, MEASURED: the amended owner grant authorized repairs;
startup ownership and owned-child/descendant aborts now have EXPECTED RED
commits, passing tests and at least two mutant killers for each change. The
running cohort uses a lane-owned direct-child runner; native/fast uses the
separately deployed descendant guardian. Inherited scripts remain read-only.

Historical CODE READ, harness blocker: inherited session3.sh uses a name-wide TERM pkill
on Xid; its run_fast_guest.sh unconditionally uses name-wide pkill -9 before
startup. Neither proves process ownership. Neither has been executed by HW2.
Options reported to owner: authorize tested lane-owned harness repair, or stop
hardware work and restore previous production. That earlier approval request
was superseded by the complete amended owner briefing.

CODE READ, M handoff blocking concern (H-M-HANDOFF-NONBLOCK): Handoff::register
and take use a blocking Mutex<Vec<u32>>. Device::ram_add / ram_del now call
handoff_released, which may call take; C kf3_region_add/del memory listeners
reach those methods through the FFI. The listener's adjacent doorbell path
explicitly documents main-loop or vCPU PCI configuration execution with BQL
held. INFERRED: overlapping RAM topology updates can therefore expose the new
shared registry mutex on a vCPU/BQL path. A measured contention stall or a
complete QEMU call-graph proof was not obtained. Do not repeat the handoff's
"never blocks" comment as established fact; this requires review before
claiming the standing no-blocking contract is met.

CODE READ, B' test scope: relay_stress.rs guards its I7 quiet window with
settled(), but Outcome has no quiet_checked field and a failure to settle
does not itself report an I7 violation. RS_STRICT=1 cannot promote an assertion
that was skipped into coverage. The separate generated grade_stress G5 test
does carry quiet_checked and asserts it. Do not use a random relay_stress
holdout alone to claim idle-efficiency coverage.

CODE READ, B' declared loosenings (design note section 5.4): I7 HARD to SOFT;
I6 wake-in-flight and kernel-view exclusions; quiet-window preconditions and
wakeup counts replacing park counts; seed 1 moved to ignored; ReadIndex i1
bounded at six million states; lifecycle/lazy tests restated by GP_GET policy.
The act-holder phantom-demand case remains red, and two three-worker explorer
cases exceed their state budget. Our passing holdout does not erase those reds.

MEASURED: exact B' did pass fresh strict random seeds 4294967296..4294967328
(exclusive upper bound), infinite park, chaos-hooks: 32 seeds, zero failures,
zero soft findings. Its dedicated grade_stress G5 idle test also passed with
two and three workers and its explicit non-vacuity check. These results do not
identify the cause of the inherited hardware counter hits.

CODE READ: no production seed or thread-name special case was identified in
the reviewed worker/relay/handoff changes. B' chaos hooks return no decision
value; their callback can perturb scheduling. The negative-control and optional
diagnostic/park knobs are explicit. This is a bounded code review, not a proof
of every source path or an exact-hardware validation.

MEASURED independent HW3 follow-up: full default --tests1051/0,5ignored,
all1027 prior M case/status entries preserved;192new holdout scenarios pass.
Clippy independently matches93unchanged-line errors,zero new errors,two new
test-style warnings relative to M. CODE READ: the new cell's SC release-epoch
handshake covers late registration and unlocks before ringing; scheduling is
queued in act order and allocation identity is checked. Generic-cell execution
and structural adapter pins are distinct. Actual QEMU/GPU paths remain
unmeasured; no READY handoff or worker-RM-view fix is claimed.

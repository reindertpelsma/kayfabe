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

INFERRED: a new normal ring arriving after a timed-out epoll_wait returns and
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

CODE READ, harness blocker: inherited session3.sh uses a name-wide TERM pkill
on Xid; its run_fast_guest.sh unconditionally uses name-wide pkill -9 before
startup. Neither proves process ownership. Neither has been executed by HW2.
Options reported to owner: authorize tested lane-owned harness repair, or stop
hardware work and restore previous production. Required approval is pending.

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

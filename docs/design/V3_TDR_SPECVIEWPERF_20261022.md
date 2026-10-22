# Windows SPECviewperf TDR loop, run 3440

**STATUS: LIVE-RESEARCH, 2026-10-22. Lane T1. Diagnostic additions are DESIGN-ONLY.**

## Corrections to the handoff, before interpreting the failures

MEASURED: VF1's preserved workload run logs contradict the supplied identification
of the first failing subtest: **catia-07 03_ThreeLoftJetsDarkMirror** was running
at the first event-153 group, not subtest 01. Catia 01 and 02 completed, as did
04 and 05 after recovery. Creo 05, 10 and 11 also completed; all six energy
subtests subsequently completed. Thus "every new viewperf.exe dies" is too broad.
The supplied snapshot2 SPEC run-log file is a PowerShell argument error, not a
workload-progress record. The replacement evidence is the already-exported text,
read without invoking the guest agent. See [snapshot3 and workload evidence](../../traces/tdr_specviewperf_20261018/snapshot3-and-workloads.json).

CODE READ: `seed life N` numbers BAR1 seed placements (`CpuWindow::reseed`), not
`BootClean`'s fn-72 RM generations. Below, "seed-3/seed-9 teardown" identifies the
requested log markers; it does not assert that they are RM generation IDs.

INFERRED: **STOP on a subtest-01-specific reproducer or a global poisoned-life
conclusion.** Options: (1) reproduce the whole catia viewset, retaining the order;
(2) isolate subtest 03 using the installed harness's supported selection;
(3) retain the original subtest-01 arm as a negative control. Recommendation:
(1), then (2) once its selection is verified. No substitute hardware experiment
or product design change was improvised in T1.

## Result

MEASURED: snapshot2 contains eight `USER_RING_GP_FORMAT Budget` refusals and nine
BAR1 unload markers; snapshot3 extends the same byte prefix to twelve refusals
and thirteen unload markers. The first refusal precedes the first refused
`RESET_CHANNEL` and teardown in log order. The extra, very short recovery cycle
has no new Budget refusal: this finding does **not** explain every reset.

CODE READ: the refused descriptors exceed the existing 917,504-byte source
staging policy. `inspect_entry` checks that limit before Pointer/Scan selection;
`user_ring_serve` makes the relay dead and cancels its obligation on refusal.
INFERRED: a workload-dependent relay admission failure is the leading explanation
for the repeated application kills. Stable guest descriptor bytes and exclusive
causality still need a controlled run. Changing a limit or timer is not a fix.

MEASURED: all 5,058 snapshot2 status samples, and all 6,097 snapshot3 samples,
report `unreconciled=0` and `root_moves=0`. Every one of snapshot3's 197 logged
T-space retirements says `released=true` and `stale_binds=0/...`. These lower the
priority of an indefinitely armed trigger or observed stale T-space bind; they
are not a complete channel/VA lifetime proof.

## Scope, evidence and clock contract

CODE READ: branch base is `origin/master` `e64b8e5f6`; the lane identifies the
runtime binary as `89fc8a222`. The decoder, turn admission, staging constant,
device walk-busy gate and BAR1 logic were checked against that runtime lineage.
`user_stream.rs`, `user_turn.rs`, `device.rs` and `bar1phys.rs` have no diff
between those revisions; the VA-manager diff is formatting. The refusal path
was also read directly at `89fc8a222`.

MEASURED: research began 2026-10-21 23:19 UTC; deadline 2026-10-22 02:19 UTC.
Host access consisted only of file reads and bounded extraction. No VM, guest
agent, GPU, broker or lock operation was performed. No agent/session was messaged.
No closed-driver binary was inspected or reverse engineered.

MEASURED: the committed [snapshot2 extraction](../../traces/tdr_specviewperf_20261018/snapshot2-summary.json)
records source basenames, SHA-256 hashes, original line numbers, compact counter
samples and bounded excerpts. The snapshot3 supplement records its hash, verifies
snapshot2 is a byte prefix, and includes the exported workload run logs. Full
logs remain on the trusted host; their private paths belong only in T1's local
report. No raw guest profiles, host paths or machine identifiers were copied.

INFERRED: clock conversion uses the lane's guest-displayed time + 2 h = host
CEST, and the lane's VM start 21:56:17 CEST as an approximate origin for device
uptime. The start has only second precision and is not a measured identity with
`Device::born`. Guest event time, application error time and BAR1 unload are
separate events; do not force their timestamps to coincide.

CODE READ: the drainer emits changing status lines after at least two seconds;
the interval can grow when the drainer stalls. These lines, large-apply lines,
capacity lines and the Budget refusal have no per-operation timestamp. A count
of ninety heartbeats is a **nominal** three-minute view, not an exact wall-clock
window. The last display anchors surrounding the refusal are 9548.766 s and
9761.210 s, a 212.444 s bracket. No operation times were linearly interpolated.

## Three minutes before the first event group

MEASURED for guest times from the exported run log/event text; INFERRED for
host conversion and approximate uptime in this table:

| Guest-displayed time, Oct 21 | Host CEST, Oct 22 | Approximate uptime | Event |
|---|---|---:|---|
| 22:35:26 | 00:35:26 | 9549 s | Catia workload starts; blender workload marked failed |
| 22:35:53 | 00:35:53 | 9576 s | Catia 01 starts |
| 22:35:57 | 00:35:57 | 9580 s | Requested three-minute pre-event window begins |
| 22:37:32 | 00:37:32 | 9675 s | Catia 01 completes; 02 starts |
| 22:38:18 | 00:38:18 | 9721 s | Catia 02 completes; 03 starts |
| 22:38:57–58 | 00:38:57–58 | 9760–61 s | Three nvlddmkm event 153 records |
| 22:39:00 | 00:39:00 | 9763 s | OpenGL driver error 3/subcode 2, viewperf PID 17080 |
| 22:39:04 | 00:39:04 | 9767 s | Catia 03 marked failed; 04 starts |

MEASURED: the bounded device-clock bracket contains 105 status samples. The
last ninety occupy lines 104843–113435. Their endpoint deltas are:

| Counter | First | Last | Delta |
|---|---:|---:|---:|
| Invalidate arrivals / cleared | 14272 / 14272 | 14512 / 14512 | 240 / 240 |
| Walks reconciled / submitted | 115402 / 115402 | 156688 / 156688 | 41286 / 41286 |
| Mapped runs | 1550106 | 3172268 | 1622162 |
| Unmapped runs | 1534168 | 3067015 | 1532847 |
| Statements received / settled | 156 / 156 | 161 / 161 | 5 / 5 |
| Root registrations | 88 | 91 | 3 |
| Unreconciled / root moves | 0 / 0 | 0 / 0 | 0 / 0 |
| Refusal sample-list length | 16 | 16 | 0, already saturated |
| Poisoned relay count | 0 | 1 | 1 |

CODE READ: `roots` counts registrations, not simultaneously live rooted spaces.
The 91 value therefore cannot establish audit VA-2's >64-live-space condition.
`refused=16` is the bounded VA refusal sample list, so a flat value does not prove
no new refusal. `vat` averages are differences since the last status call;
`vat.max_us` retains the cumulative maximum (`VaTiming::since`).

MEASURED: across the nominal ninety samples, heartbeat walk averages range
0–7295 us and apply averages 0–10984 us; nonzero arrival-to-clear averages reach
511 us. The cumulative invalidate maximum is 95264 us, already present at the
window's beginning. This cannot locate that slow invalidate inside the window.
The final heartbeat has zero new walks and host calls while the counters remain
settled; it does not show a permanently unfinished memory batch.

MEASURED: the wider 212.444 s bracket contains 455 logged large applies, 0–103 ms,
with 8461 ms summed logged duration; 130 exceed 12 ms and 77 exceed 50 ms. The
largest is line 107410: 17184 maps, six batch verbs, 103 ms. Line 106960 maps
16232 runs with 4025 map verbs in 96 ms. These are per-entry applies selected by
>=1000 changed runs, with integer-millisecond truncation, not a distribution of
all applies or trigger-to-clear latency. INFERRED: they show substantial mapping
cost and deserve measurement under §AD; they do not prove the first TDR cause.

MEASURED: 56 capacity retry records in that bracket hold 24 slots throughout,
with ceilings zero. The first has grows=93/retries=158; the last has 122/213,
1889024 free slot-pool runs, largest slot/walk 423168. Slot 94 grows to 122624
runs at line 113233. During subsequent teardown, slots held fall to 15 and
retries reach 215. This is observed capacity recovery, not a ceiling failure.

MEASURED: ten deferred context acts occur in the bracket: three InitializeCtx,
three PromoteCtx, four EvictCtx. The failing channel's client/object pair is
initialized at line 112652 (13 us) and promoted at 112653 (3 us), both satisfied
by the host twin's context. Earlier clients using the same host handle 0x61 were
evicted and freed before reuse. No failed deferred-context act was observed in
this bracket. Their raw request bodies were not interpreted beyond kayfabe's
own decoded log labels.

MEASURED: the logged HELD-minus-POSTED balance is zero at the bracket start,
Budget refusal, first display-loss marker, first unload, seed-9 unload and
snapshot2 EOF; global logged maximum depth is one and EOF balance is zero.
There is no persistently unmatched reply in this trace. Reply latency and
unlogged/capped cases cannot be reconstructed from this balance.

### First explicit fatal relay anomaly

MEASURED, snapshot2 line 113366:

```text
USER_RING_REFUSED 0xc1d00188:0xff040031 host 0x61:
USER_RING_GP_FORMAT Budget entry=0x0faa26023110102c
source_va=0x4101ee0 cursor=0x3bdc put=0x3c76 entries=32768;
no completion/reuse credit
```

CODE READ: the public C56F decoder yields length
`((entry >> 42) & 0x1fffff) * 4 = 1026596` bytes and VA `0x23110102c`.
The staging budget is `user_storage::MAX_SOURCE_BYTES = GP_OFFSET = 0xe0000`,
917504 bytes. The descriptor passes direct-format decoding but exceeds that
policy by 109092 bytes. This is a per-descriptor admission budget, not a walk
capacity error or a count of iterations exhausted. A logged word is not proof
of a race-free guest descriptor lifetime.

CODE READ: `user_turn::Relay::plan_step` calls `inspect_entry` before software
lookup and Pointer publication. `userpump::user_ring_serve` handles the refusal
by setting `r.dead`, incrementing `poisoned`, and `record.cancel()`. Retrying a
wake, polling GP_PUT, or changing an observation interval cannot admit this
same descriptor. A safe size-handling change requires a bounded continuation
or a complete mode contract; no allocation/staging bound was raised here.

MEASURED: line 113368 first reports `poisoned=1`; line 113470 logs the first
refused `RESET_CHANNEL` (`fn76/0x906f0102=0x56`). Display loses its window at
9761.210 s (113512) and goes black at 9761.460 s. BAR1 returns physical at
9761.968 s (115421), fn 47 unload follows, and fn 72 appears at 115454.
This is log order, not a measured refusal-to-TDR latency. No preceding fatal
relay refusal appears in the snapshot2 prefix. Large applies are earlier
performance anomalies, so "first explicit fatal relay anomaly" is narrower
than "first abnormal event of any kind".

MEASURED: at retirement the failing USER relay is Pointer mode, with 211919
Pointer entries and 13 Scan entries; its lazy counters record 210942 cache hits
and 9209 fresh PUT reads. Its host handle is 0x61 and token 0x33. The retire
ledger says rung=0/forwarded=0 for that Passthrough-route bookkeeping; CODE READ:
those inline-doorbell counters do not measure relayed USER-ring publications.
They must not be reported as proof that no work reached this channel.

### Last observable Translated progress

MEASURED: the last pre-loss status samples retain these endpoints. These are
last **observed cursor values**, not timestamps of the last doorbells:

| Token(s) | Last PUT / reported GET | Forwarded entries / submissions |
|---|---|---|
| 0x3 | 192 / 192 | 192 / 405 |
| 0xd | 1 / 1 | 1 / 1 |
| 0x801 | 2 / 2 | 2 / 2 |
| 0x802 | 1 / 1 | 1 / 1 |
| 0x80c, busy kernel copy channel | 11100 / 11100 | 4074332 / 1089060 |
| 0x804, 0x806, 0x80b, 0x1805, 0x2007, 0x2008, 0x200a, 0x2809 | 0 / unread | 0 / 0 |

MEASURED: token 0x80c transiently has PUT 7660 / GET 7201 at line 113007, then
4406 / 4406 at 113086 and 11100 / 11100 at 113368. A modulo cursor cannot be
subtracted from a cumulative forwarded-entry count. There is no complete
"forwarded minus executed" counter in these snapshots. Its retirement reports
124334/124334 splits and `dead=None`; all thirteen Translated channels in the
first teardown report `released=true` and zero unresolved-wait counters.

CODE READ: GP_GET may precede execution; neither cursor equality nor the silent
USER-only `PROGRESS_STRAND` watcher proves all semaphore releases/interrupts
landed. Last per-channel doorbell times, execution witnesses, semaphore values,
invalidate pending age and walk-busy age were not captured in this run. They
remain explicitly unverified.

## Later cycles and state crossing an RM life

MEASURED: snapshot2's marker sequence below uses seed labels only. The interval
is from the preceding unload, not the new viewperf process's measured lifetime.
Every row with a Budget refusal is followed by a viewperf OpenGL error in the
preserved guest event text; reset-to-refusal timing itself is not timestamped.

| Seed label at unload | Uptime seconds | Previous-unload interval | Budget refusal line / bytes |
|---:|---:|---:|---|
| 3 | 9761.968 | first | 113366 / 1026596 |
| 4 | 9914.191 | 152.223 s | 126035 / 980944 |
| 5 | 9919.806 | 5.615 s | none newly logged |
| 6 | 9999.208 | 79.402 s | 139098 / 981632 |
| 7 | 10034.814 | 35.606 s | 146095 / 981284 |
| 8 | 10056.791 | 21.977 s | 152469 / 979592 |
| 9 | 10079.804 | 23.013 s | 158444 / 981756 |
| 10 | 10137.520 | 57.716 s | 168353 / 981252 |
| 11 | 10162.980 | 25.460 s | 174914 / 980956 |

MEASURED: snapshot3 adds four Budget refusals (954976, 981700, 981416, 947340
bytes) and extends unload seed labels through 15. Its twelve refusal lengths
span 947340–1026596 bytes, all above 917504. The last logged Budget refusal is
line 207504; the subsequent energy run reports six completed subtests. The
actual completed/failed subtests provide a stronger discriminator than the
assumption of a permanently broken new RM life.

MEASURED: comparison of the seed-3 and seed-9 teardown clusters:

| Observation | Seed-3 cluster | Seed-9 cluster |
|---|---|---|
| Translated retirement records | 13 | 13 |
| Ring releases / distinct retired ring VAs | 13 true / 13 | 13 true / 13 |
| Stale-bind checks / stale / late / indeterminate | 1150184 / 0 / 0 / 0 | 41486 / 0 / 0 / 0 |
| Nonzero unresolved walk/acquire/refused counters | 0 | 0 |
| Status at last pre-unload sample: leaked T-space slots | 0 | 0 |
| Last capacity line: slots held / ceilings | 15 / 0 | 25 / 0 |
| Held replies / posted during preceding marker interval | 6370 / 6370, includes earlier boot history | 650 / 650 |

CODE READ: the T-space is VM infrastructure, hardwired by §AB. Its continued
existence across a guest RM unload is not itself a leftover channel. Ring VAs
and several host handles recur after successful release. `VasKey` is built
from `(guest_client << 32) | guest_vaspace`; reuse is possible and is not a
monotonic incarnation ID. The fixed key 13965662590488021072 occurs in both
clusters, while other keys change. Existing evidence cannot prove that every
host VA object/lease was retired exactly once; no identity-complete host census
was collected. `views.held` fluctuates (294 at the seed-3 pre-unload sample,
282 at seed-9), rather than demonstrating monotonic growth.

CODE READ: `boot framebuffer scrub REFUSED: newer guest RM life ...` is the
explicit generation-mismatch guard in `device.rs`, not an RM/CUDA allocation
refusal. BAR1-mode restoration and fn-47 unload both request restoration; the
fn-47 path updates `unload_life`. MEASURED: in the first cycle, the skip at
9761.968 s is followed by fn 47 and a `scrubbed first` line at 9762.028 s,
before the next logged fn 72. INFERRED: that message alone cannot establish
that a new RM already began or that old data corrupted recovery. Its wording
is broader than the exact generation comparison that produced it.

MEASURED: the supplied dmesg tail has two old `unknown NVRM ioctl command: 0xf0`
lines and no Xid line. T1 did not query the live GPU or obtain a complete,
time-bounded kernel journal. The lane's unchanged Xid count of 26 remains
supplied evidence; absence from this short tail is not an independent recount.

## Ranked hypotheses, falsifiers and decisive experiments

CODE READ: the initial hypotheses and falsifiers were pushed in `688d7507e`
before failure-window inspection; the Budget refinement was recorded before
its parser/context inspection and pushed in `bcc007267`. Four hypotheses remain,
ranked now by the evidence above. Experiments below are proposals, not T1 runs.

1. **H3: relay admission/publication failure, especially the oversized entry.**
   FALSIFIER: all failing-channel descriptors are stable, admitted and published,
   with real execution/required interrupt completion before failure; for the
   Budget candidate specifically, recovery already began before refusal, or
   equivalent refusal preserves and completes work without reset. MEASURED:
   repeated refusal followed by cancellation is present, with no preceding
   logged fatal relay failure. INFERRED: leading candidate, not exclusive cause.
   Cheapest decisive sequence: GPU-free fixtures using the twelve captured
   descriptor words and real `inspect_entry`/`Relay`; then fresh-VM whole catia
   and isolated subtest 03, matched to VF1's reference. Capture timestamped
   admission/refusal and execution witnesses. Do not "test a fix" by raising
   `MAX_SOURCE_BYTES`. Lost notification remains a separate subcase requiring
   GP_PUT-advanced/relay-not-read evidence; none was found here.
2. **H1: memory reconciliation/invalidate or walk-busy stalls kernel progress.**
   FALSIFIER: complete trigger/channel observations show all relevant invalidates
   cleared, walk-busy released and kernel work executed before every failure.
   MEASURED: sampled unreconciled/root-move counts stay zero, capacity ceilings
   zero; long applies exist. INFERRED: indefinite VA-4 root-move stall is not
   supported in this run; brief stalls and VA-1/VA-2 are not excluded by sampling.
   Cheapest decisive experiment: same fresh catia arm with pending-age,
   walk-busy-reason and execution telemetry below. Correlate per-sequence
   trigger-to-clear age, not the cumulative maximum or created-root count.
3. **H2: retained channel/VA state amplifies recovery failures.**
   FALSIFIER: an identity-complete census proves all nonpersistent old-life
   bindings/channels/leases retired and new-life operations remain valid through
   failures. MEASURED: observed T-space releases succeed, stale checks stay zero,
   some later viewperf workloads complete. INFERRED: a blanket poisoned-life
   claim is contradicted; an unobserved host lease leak remains open. Cheapest
   decisive experiment: fresh catia-only versus matched post-recovery catia,
   with per-incarnation birth/retire/host-resource counts; reuse of a bare handle
   must not be counted as the same incarnation. No synthetic reset is needed
   until the natural workload sequence has been measured.
4. **H4: workload/guest-driver failure also occurs without kayfabe.**
   FALSIFIER: repeated matched fresh kayfabe failures and VFIO success on the
   same image/driver/viewset/settings, plus an identified kayfabe progress defect.
   CODE READ: VF1's published final disposition at `584ebaede` reports zero
   native samples: its open-device safety gate refused before GPU detach or
   native QEMU launch, and it stopped. The desktop restoration is reported
   partial (GDM greeter rather than the owner's previous desktop session).
   See [VF1's final evidence](https://github.com/reindertpelsma/kayfabe/blob/584ebaedee73eac4c46846301a023c3eb5990627/traces/specviewperf_vfio_20261022/README.md).
   INFERRED: unresolved, with no VFIO comparison available this night. T1 did
   not retry or work around that refusal. The cheapest decisive experiment
   remains a future owner-authorized matched reference, comparing per-subtest
   pass/fail and settings first; obtain repeated catia-03 controls before
   blaming a driver.

CODE READ: relevant audits are
[`claude/audit-race-20261021`](https://github.com/reindertpelsma/kayfabe/blob/claude/audit-race-20261021/docs/audits/2026-10-21-race-correctness-audit.md)
(VA-4, VA-2/VA-1, RC-01, RC-03) and
[`claude/audit-security-20261021`](https://github.com/reindertpelsma/kayfabe/blob/claude/audit-security-20261021/docs/audits/2026-10-21-security-audit.md).
The VA-4 root-move unreconciled branch and device-wide walk-busy dependency are
present in the runtime lineage. The audit RC-01/RC-03 shapes do not follow from
a Budget refusal or a silent watcher; separate reproductions are required.

CODE READ: `KF3_PUT_CACHE` and `KF3_LAZY_OBSERVE` select USER Pointer observation
behavior; they do not change `inspect_entry`'s admission limit. MEASURED: run
3440 logged cache=true/observe_interval=16. INFERRED: a same-binary both-off arm
is a useful timing negative control after the main reproducer, not a cure or
proof about the Translated path. Neither switch may replace doorbell correctness.

CODE READ: the earlier [Windows bring-up findings](V3_WINDOWS_BRING_UP_20261009.md)
include both an uncleared invalidate and paging-node completion loss from a
Translated refusal. They are precedents, not a diagnosis of this run. CODE READ
of run-3004's preserved investigation note: its investigator attributed ~24 GiB
RSS to two mappings of one 12 GiB guest RAM store, and reported the fixed-range
VA allocation failure had a fallback. INFERRED: neither the RSS number nor that
old failure alone proves a cumulative memory leak or the cause of run 3440.

## Minimal default-off diagnostics: TEST-FIRST design

INFERRED — proposed design, pushed before any implementation. This lane writes
no runtime code. The additions below are observation only; no boot flag,
timeout, polling repair, forced completion, retry policy, scrub policy or mapping
semantics changes. The above stopped subtest-01 premise is not used as a design
input. Three bounded records and one shared clock are sufficient:

1. **Timestamped refusal and Translated progress record.** A new
   `KF3_DIAG_TDR=1` is read once at construction, unset/0/invalid = off. Record
   construction-relative monotonic time, fn-72 generation and a separate
   monotonically assigned channel incarnation. On existing worker reads/events,
   capture last known GP_PUT/GP_GET, last actual host submission, real private
   fence sequence/observation, last decoded semaphore release and observed value
   (or explicit unread), scheduled/stopped/disabled/dead and unresolved reason.
   Include token, guest object and host handle as identifiers, never pointers.
   Instrument the already existing refusal path with decoded length and budget,
   mode, source generation and cursor; no pushbuffer dump. Existing doorbell
   callbacks may add only gated atomic count/time stores; distinguish actual
   guest doorbells from internal re-rings and inline from worker-served routes.
   A last-known value must carry its observation time. An unchanged GET is not
   automatically a stall; report a pending obligation with no execution progress.
2. **Invalidate pending age.** At VA-thread observation of a newly armed sequence,
   record sequence, first-observed time and PDB scope. Preserve age across repeated
   observations of that same sequence; close with Cleared/Superseded/Abandoned
   separately and measure completed latency using the request's existing arrival
   time where available. Explicitly distinguish first-observed pending age from
   vCPU-arm-to-clear latency. Never clear or retry an invalidate because the
   diagnostic ages it. Keep count/max and fixed histogram buckets including
   >12 ms; report the still-pending age independently of completed samples.
3. **Walk-busy age and causes.** At the existing VA-loop publication capture
   in-flight walk, pending wants, unsettled statements and armed-trigger bits,
   plus walk-waiter count. Busy age runs from false-to-true until all causes clear;
   cause changes while busy do not reset it. Capture last VA-loop progress time,
   so a diagnostic reader can say the owner stopped publishing during an RM call.
   Retain separate ages for the armed sequence and continuous walk-busy episode.

CODE READ: `KF3_COMPLETION_PROBE` already records Translated fence/semaphore
information and `PROBE-DUMP` has cursors; reuse its event semantics rather than
inventing a new meaning of completion. Its existing `probe_tick` takes the slots
map read lock and channel locks. INFERRED: the new watcher should consume copied
telemetry, not add acquisitions of those production locks or new USERD/guest
memory polling. A design that simply wraps that method in another timer would
not meet the observer contract.

INFERRED — ownership/bounds acceptance contract: fixed records limited by the
trusted channel/token budget; allocate fallibly only when enabled. No per-event
heap growth. Writers use existing owned observations and publish to separate
telemetry records with bounded nonblocking operations; a telemetry-only
`try_lock` may drop a sample and count it, never delay or alter production
handoff. The reader takes no production lock, performs no RM verb or guest
memory read, and cannot signal a doorbell or release a semaphore. Contention,
dropped samples and unread fields are explicit. One start/end record per
pending episode, at most 64 starts/64 ends per VM plus a saturation summary;
channel incarnation prevents a reused token inheriting age. A diagnostic thread
checks the device stop flag and exits; no thread is required when off. An age
threshold controls logging only and must never be reused by scheduling logic.
Disabled channels retain their state but are not classified as runnable stalls.

INFERRED — implementation sequence, with required checks (none claimed run):

1. Push this design. Add a **test-only EXPECTED RED commit**, with the two
   required commit trailers, showing the current binary lacks the observations.
   Use real integration seams plus a fake clock and fake sink; do not settle for
   tests of a detached model that production never calls. Missing API compile
   failures alone are not sufficient red proof: existing worker/refusal/VA paths
   must fail observable assertions. Commit that red state alone, then push.
2. Add the smallest instrumentation integration in a green commit. GPU-free
   crate tests cover the actual affected crates, `nice -n 19`, `-j2 --release`,
   lane-owned target, and disk guard before every cargo invocation. Compare
   enabled/disabled fake-I/O action sequences for exact equality. Test fixed
   allocation/log bounds and shutdown, with no GPU or host action.
3. Mutate the integration/model by hand, restore each mutation afterwards, and
   require **at least two independent killing tests for every behavior change**.
   Clippy baseline and final use identical commands/configuration; added
   diagnostics must be zero. Do not claim baseline warnings are new or suppress
   them to obtain a green count. Open a draft PR only if code is written.

INFERRED — required tests and planned mutation killers; this is a design table,
not measured results:

| Mutant | Two required independent killers |
|---|---|
| Enable diagnostics when env is unset/invalid | `off_constructs_no_records_or_thread`; `off_executes_no_clock_or_sink_calls` |
| Omit worker/refusal integration | `real_budget_refusal_emits_length_budget_mode_and_time`; `translated_existing_submit_and_completion_publish_witnesses` |
| Re-arm same sequence resets pending age | `same_sequence_retains_first_observation`; `old_pending_sequence_crosses_12ms_without_new_event` |
| Clear A hides newly armed B | `superseded_a_does_not_end_b_episode`; `rearm_has_new_identity_and_age` |
| Busy cause change resets continuous age | `walk_to_trigger_keeps_busy_origin`; `unsettled_to_pending_keeps_busy_origin` |
| Token/host-handle reuse inherits old state | `new_incarnation_has_no_old_semaphore_or_age`; `late_retire_cannot_delete_replacement` |
| GET equality is treated as execution | `prefetched_get_with_unreached_fence_remains_pending`; `only_real_fence_observation_closes_submit_episode` |
| Telemetry changes scheduling or blocks | `on_off_have_identical_fake_host_and_guest_actions`; `contended_telemetry_drops_sample_without_wait_or_rering` |
| Remove storage/log caps or stop check | `hostile_event_stream_stays_at_record_and_log_caps`; `shutdown_exits_reader_with_pending_episodes` |

## Verification and limits

MEASURED: thirty artifact-consistency assertions passed, covering JSON-derived
counts, source-prefix confirmation, descriptor-field arithmetic, reply balance,
and local document links; source hashes, counter deltas and bounded excerpts
were also inspected. These are artifact checks, not product tests.
No code was changed; before/after Rust test counts are 0/0,
mutation kills and clippy comparison are not applicable. The design table above
is unimplemented and is not a test pass. No draft PR is opened for this
research-only branch.

INFERRED: the most useful next owner decision is a test-first bounded handling
design for oversized legal direct descriptors, validated against the matched
reference. Do not approve a staging-limit increase as the remedy. Required
unverified items remain exact per-operation chronology, descriptor stability,
complete cross-life host ownership, the extra reset without a Budget refusal,
and matched VFIO results. There is no claim of a repaired TDR loop, an exact
last-doorbell history, a hardware performance improvement or general Windows
stability.

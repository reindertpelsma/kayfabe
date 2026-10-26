**STATUS: LIVE, 2026-10-22.** Owner GO supersedes the earlier documentation-only scope.

2026-10-22 closure correction supersedes the earlier held/UNRUN state below: second clean N0
on own57148807 also passed, native32 absent. Windows580.88 disabled bind22 passed; precision-only
ON reproduced0x50. OFF emitted the known exact-shaped QEMU32pair before actual CIMCode0 proof
was recorded, so the strict guard aborted and B1 stayed UNRUN. This is not a Code43 verdict or a
natural0x116 measurement. The enable helper's20s observation sleep precedes the diagnostic call,
creating a proof-capture gap; a future independent actual-state observer must preserve the same
pre-fault init gate. No permissive exception/threshold change is proposed.
Two watchers also overwrote the first fatal reason. GPU-free test-first d1e2da41c/9f1af1513 ->
a22567a3d now atomically preserves the first whole receipt while both owned controllers still stop.
Full bench119->125pass/0fail; three new mutants5/2/2killers (34retained total). This fix was after
teardown, not part of the measured run. The first original guard reason remains unverified.
Ownlease APIabsent03:47:36UTC, lane runtime4.832257USD<7, accountcredit40.452861/reserve5 intact.
Detailed evidence: traces/blackwell_validation4_20261022/RESULT.md; no next rental grant implied.
The reviewed classifier defects below were repaired before this lease: product/guard `8f3a9bb3a`,
allocated-class fixture `a3f39a633`, and independent detached-QEMU termination guard `a634bdc194a`.
Native Xid 32 remains fatal. Post-closeout CPU review found one further gap: a1000-RPC prefix
proves progress without proving actual Windows Code0. The test-first correction now additionally
requires host-observed actual CIM Code0/580.88 for the same arm/PID/starttime, with both observations
before the fault. Full Python72->86executions/0fail; four additional mutants>=2killers each.
See [Code0 proof](../../traces/blackwell_validation_20261022/guard_code0/). This last fix was
GPU-free only after lease closure; future runs must deploy it with the updated arm-kit recorder.
The closed lease ran no Windows arm, so this gap did not contaminate its native N0 result.
Clean N0 on lease 57132969 passed (n=1); Windows validation is held
at the post-bind shutdown census, with every Windows arm UNRUN pending the requested decision.

CODE READ (commit 007ba1eb0:28): quoted historical statement, evidence not revalidated by BW9. MEASURED correction to the historical parked-patch checkpoint below: fixture EXPECTED RED
`524ea6c4d` precedes fix `a3f39a633`; release kf-harness 15 ->19 pass, zero fail/ignored,
two mutants each killed by two tests, clippy 284 ->284 / added 0, rustfmt. Guard test-first
`b22e1be62` and supplemental `59f07cff0` precede `8f3a9bb3a`: 65 executions /59 distinct
cases, 12 mutants each with at least two distinct killers. Detached-QEMU test-first `b3d25ec19`
precedes fix `5cc2ce0b8` and watcher-wiring proof `a634bdc194a`: 72 executions /66 distinct
cases total, 14 mutants each with at least two killers. The deployed independent guard's actual
pidfd smoke passed; it retains complete-boot history and revalidates PID/starttime/executable.
Evidence: [this lease](../../traces/blackwell_validation_20261022/), including N0 verdict,
source-bound native controls and guard/source receipts. No wildcard native exception was added.

CODE READ (commit 007ba1eb0:39): quoted historical statement, evidence not revalidated by BW9. MEASURED: N0 gates 9/9 +10, raw 30/30, CUDA 4/4 and GR/CE PASS; only five exact intentional
source/PID/device controls. The prior native Xid32 did not recur. INFERRED: this supports the
fixture hypothesis, without establishing a physical-channel mapping or general causal proof.
The old counterfactual/unfinished statements below are historical, superseded by this correction;
the baseline-contamination diagnosis and classifier review findings remain applicable.

The retained full-boot journal contains native `kf-gr-tier` PID 90663, Xid 32 at
2026-10-21 22:20:19.302750 UTC, physical channel `0x0d00000a`, `intr1 00000010`,
`HCE_DBG1 0000c7b5`. Its positive fixture printed PASS. That does not establish host health.
See [kernel projection](../../traces/blackwell_resume_cont_20261022/lease_metadata/final_kernel_xids.json)
and [fixture log](../../traces/blackwell_resume_cont_20261022/lease_metadata/sweep_native_grtier.log).

CODE READ: the strongest cause candidate is **our native CE inert-bind fixture**, not an
intentional negative control or an established NVIDIA driver defect. `RUN31_CE[4]` in
`crates/kf-harness/src/bin/kf-gr-tier.rs` is the historical `0xc7b5` SET_OBJECT operand.
The fixture stages it on `host2`, whose log identifies engine `0xd` / CE4 and channel object
`0xcafe0015`. The generated Blackwell family CE set is `0xc9b5, 0xcab5`, selected against
actual supported host classes by HostRm. `HostRing::push_inner` rewrites CE bindings only
when `gr_context.is_some()`; this pure CE host ring has no GR context. T-mode accepts the
recognized historical CE bind without selecting the host's allocated CE class.
The separate GR segment has the rewriting path; the later gate subtest does not submit this bind.

INFERRED: the unique old-class operand makes the CE inert-bind subtest the leading suspect,
and its value equals the recorded debug word. The physical-channel-to-RM-handle mapping was
not retained, so assignment of physical channel `0x0d00000a` to that subtest is not measured.
Neither the meaning of `intr1` bit 4 nor the causal interpretation of HCE_DBG1 is established
by the public headers read here. Do not turn this source diagnosis into a decoded-hardware claim.
FALSIFIER for a future run: the same native Xid persists after using the actually allocated
CE class with everything else fixed. Such a result stops the lane before Windows.

Earlier lease 57108853 and bisect lease 57117614 used the same runtime revision
`2c30a158c170a66baa37b77c19000993af1546a6`; there is no product revision change to blame.
The earlier raw suite was 29/30 (one concurrent-fuzz violation), then three isolated clean
reruns; the later raw suite was 30/30. That flake is distinct from this later native GR/CE
fixture fault and does not excuse it. A complete earlier native journal was not exported;
absence of the same Xid on that lease is therefore UNVERIFIED.

Review of classifier `cc593e3be`:

| CODE READ (commit 007ba1eb0:78): quoted historical statement, evidence not revalidated by BW9.  Finding | Evidence / consequence | Required future policy |
|---|---|---|
| The explicit exception is appropriately narrow | Own QEMU PID, Xid 32, `intr 00200000`, at most one plain chid below 0x100 and one `0x0e0000xx`, no repeated chid, maximum two, approved ON/OFF phase | Keep every bound; EXPECTED remains a product failure, never a clean baseline |
| Parsing is too permissive | Unanchored regex accepts trailing payloads and can skip an unexplained first Xid marker on the same line | Exactly one Xid record per journal record; complete anchored shape; unknown shape fatal |
| Ownership is caller-dependent | PID 0/1 can be accepted when supplied as the claimed owned PID; process name and PCI device are not bound | Valid PID >1 plus live process starttime/executable and selected PCI device receipts; never trust an old PID file |
| Initialization is retrospective | In inherited `arm.sh`, eventual total RPC count >=1000 sets the boolean at cleanup | Consecutive RPC prefix plus actual host-observed Code0/580.88 for this VM life must both predate each fault; missing or partial proof fatal |
| Native-to-arm gap is uncovered | Per-arm journal followers started after native Xid 32 at 22:20:19 | Full-boot source-verified baseline frozen before GR/CE fixture, continuously guarded through bind and every arm |
| Per-call limits can be reset | Calling the stateless classifier separately for each new journal slice grants a fresh limit | Track cumulative per-arm records and immutable accepted history; lost/replaced history or third/duplicate event fatal |

CODE READ (commit 007ba1eb0:87): quoted historical statement, evidence not revalidated by BW9. MEASURED CPU-only [review probes](../../traces/blackwell_resume_cont_20261022/review_drafts/classifier_review_probes.json)
reproduce EXPECTED for trailing junk, PID 0, PID 1 and a hidden first marker. A supplied
pre-init boolean correctly refuses; the native `intr1` record itself is also refused if fed
into the classifier. Its absence from the arm slice caused the campaign coverage failure.
The existing 18 classifier/capture tests pass; those tests alone do not establish controller safety.
Retrospective timing proof for the old QEMU pairs remains valid: already 4033 / 4009 / 3553
canonical RPCs before the first faults in on_c1 / on_c3 / off_c respectively. This does not
repair the native baseline or make those arms acceptance PASS.

Native controls remain source-bound: exact FAULT_MARK address, fault/access type, channel,
source process and positive recovery from this run. No wildcard Xid 31; native Xid 32 and
all unrelated faults remain fatal. The old native helper hardcodes AD106 engine/client labels
(`CE0/HUBCLIENT_CE1`, `GR0_PBDMA0/HUBCLIENT_ESC`); Blackwell journal labels differ.
Do not import neighbor-die route constants or simply discard route/provenance checks.
A future implementation must obtain allowable route and process/channel identity from this
run's selected engine and host topology, retaining names in the receipt. The AD106 guest
missing-page `probe-launch-dma` versus native `sem-release` difference stays explicit; its
separate guest approval is not a generic Blackwell or native-Xid-32 exemption.

Pre-steering experiments are preserved as **unapplied text patches**, not runtime fixes, in
[review_drafts](../../traces/blackwell_resume_cont_20261022/review_drafts/README.md).
Native fixture tests were committed alone in `524ea6c4d` (four EXPECTED RED cases); its parked
candidate measured kf-harness release 15 ->19 passed, no failures/ignored, two mutants with
two killers each, and clippy 284 ->284 / added 0. That counterfactual is still hardware UNRUN.
The guard seam `888c95b17` and test-first `b22e1be62` remain deliberately unfinished on this
sweep branch. Their new tests are EXPECTED RED; the isolated CAPC PR does not include them.
The parked guard candidate had 40 CPU tests green, but mutation verification is incomplete.
Review also found unhandled subprocess-timeout failure paths and the untested multiple-marker
case. It must not be deployed. No new code behavior was enabled by this documentation task.

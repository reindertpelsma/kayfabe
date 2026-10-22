**STATUS: RESEARCH, 2026-10-22.** SW2 GPU-free review; no hardware or rental authorized by
this task. Recommendation: do not reuse the old unattended controller as a validation gate.
The native Xid 32 stays fatal; no new exception is proposed.

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

| Finding | Evidence / consequence | Required future policy |
|---|---|---|
| The explicit exception is appropriately narrow | Own QEMU PID, Xid 32, `intr 00200000`, at most one plain chid below 0x100 and one `0x0e0000xx`, no repeated chid, maximum two, approved ON/OFF phase | Keep every bound; EXPECTED remains a product failure, never a clean baseline |
| Parsing is too permissive | Unanchored regex accepts trailing payloads and can skip an unexplained first Xid marker on the same line | Exactly one Xid record per journal record; complete anchored shape; unknown shape fatal |
| Ownership is caller-dependent | PID 0/1 can be accepted when supplied as the claimed owned PID; process name and PCI device are not bound | Valid PID >1 plus live process starttime/executable and selected PCI device receipts; never trust an old PID file |
| Initialization is retrospective | In inherited `arm.sh`, eventual total RPC count >=1000 sets the boolean at cleanup | Timestamped consecutive init prefix must already exist before each fault; missing or partial timing proof fatal |
| Native-to-arm gap is uncovered | Per-arm journal followers started after native Xid 32 at 22:20:19 | Full-boot source-verified baseline frozen before GR/CE fixture, continuously guarded through bind and every arm |
| Per-call limits can be reset | Calling the stateless classifier separately for each new journal slice grants a fresh limit | Track cumulative per-arm records and immutable accepted history; lost/replaced history or third/duplicate event fatal |

MEASURED CPU-only [review probes](../../traces/blackwell_resume_cont_20261022/review_drafts/classifier_review_probes.json)
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

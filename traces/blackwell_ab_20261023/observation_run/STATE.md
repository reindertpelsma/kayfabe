# Owner 13:05: pin readiness fixed; exact fallback unavailable

MEASURED on 2026-10-23: tests-only EXPECTED RED commits5e5a16ce6 and58683b816 precede fix at commit a36511ec3. `console_pin_1305/all_tests.txt` records250 passing helpers, including22 new pin/operator tests. `console_pin_1305/console_pin_mutations.json` records11 mutants, each killed by at least2 tests. The previous operator fails4 tests when replayed with the corrected test harness.

CODE READ: `scripts/bench/blackwell/console_pin.py` distinguishes missing console publication from a different published ed25519 identity. Missing pins trigger console re-reads every20s, within60s times factor4–10 anchored to the first scanned candidate, capped at1200s since lease creation. Neither repeated reads nor SSH retries restart those bounds; late successful pins cannot shorten the total ceiling to600s. A different published pin still refuses immediately. The operator records public fingerprints and console hashes, without raw console content. StrictHostKeyChecking, native/Xid guards and the per-arm observation protocol are unchanged.

MEASURED: `console_pin_1305/single_recheck.json` at 2026-10-23 11:07:30 UTC contains no verified GB206/GB205 alternative; the only returned candidate is de-verified. Offer57344521 is absent from the search. Under the explicit exact-offer exception, one create request for57344521/machine150531 was sent with --cancel-unavail, using its earlier verified snapshot as the approved fallback provenance. `console_pin_1305/create_refusal.json` records provider400/not available. No additional market poll or create attempt followed.

MEASURED on 2026-10-23: `console_pin_1305/closure.json` confirms no new lease, timers or runtime spending, all17 historical ownids API-absent, no live own-label instance and both previous timers gone. Phase estimate remains5.200540477629074USD, stop7.5/cap8/reserve5. The exception was attempted once but allocated no lease; the bad list remains intact. B/B1 and runtime page/hold/TDR evidence are unrun. No claims gate stopped live work.

FALSIFIERS BEFORE ANY LATER LIVE RUN: absent-then-present attestation inside the frozen bounds must pass; a different published pin must refuse before SSH; missing pin after either bound must expire as readiness. For inherited-die B, 0x50/refusalCode0 is an actual-die inheritance finding, notGB202. After an admitted Xid32, guest0x116/TDR/exit falsifies survival; no admitted Xid leaves fault survival untested. None of these hardware hypotheses was evaluated by the refused create request.

--- Earlier checkpoints ---

# Owner 12:50 attempt closed: SSH identity refusal before work

MEASURED: `traces/blackwell_ab_20261023/observation_run/lease_57359730/closure.json` records the 2026-10-23 attempt. The single recheck found verified RTX5060Ti offer50694293/machine45428, advertised GB206, which took precedence over the one-time exactoffer57344521/machine150531 exception. That exception remains unused and its bad-list entry is retained. No additional GB202 query or market recheck occurred.

MEASURED on 2026-10-23: lease57359730 was created10:40:25.278347UTC; auto timer1319268 was ready10:40:25.304323 and budget timer1319269 ready10:40:25.406253, both before readiness work. The fixed deadline was13:10:25UTC and was never extended. Readiness eventually received an SSH host key absent from the authenticated instance console; the unchanged identity check refused. This is a startup safety refusal, not a provider_host_error classification. No native/guest workload, media upload, Windows install, B or B1 ran. The actual die, driver and served page are unverified.

MEASURED on 2026-10-23: `lease_57359730/destroy.json` records exact-id teardown and API absence at10:50:14.258738UTC, followed by cancellation of both timer processes. `lease_57359730/closure.json` confirms all17 known own ids absent and both timer PIDs gone. `COST_FINAL.json` estimates0.05246288645USD for this lease and phase5.200540477629074USD, below stop7.5/cap8/reserve5. Account credit is a separate lagging observation. No retry followed the safety refusal; no live authority file remains.

CODE READ: source at commit 48b027267 has own GB206 page rows; that would have been the expected selection for this lease, rather than GB202 inheritance. `expected_page_policy.json` remains a source-level expectation. The future inherited-die 0x50/refusalCode0 falsifier is unchanged and untested; the Xid32 survival/TDR question is also untested. Default/native guards, the per-arm observer, SSH pinning and PR110's no-retry-after-safety-refusal rule are unchanged.

MEASURED on 2026-10-23: `lease_57359730/helper_tests.txt` records228 passing GPU-free tests after the operator configuration edits. The earlier26 mutation checks apply to the unchanged observation/projection/protocol implementation. No publication gate stopped the rental; the actual SSH identity guard did. The presented host key and complete console bytes were not retained, so no root cause for the identity mismatch is asserted.

--- Earlier checkpoints ---

# Owner 12:20 RTX50 expansion: provider-policy refusal

MEASURED: `traces/blackwell_ab_20261023/observation_run/rtx50_rental_refusal.json` records the 2026-10-23 attempt: one verified RTX 5070 Ti offer 57344521 / machine 150531 (advertised GB203), excluded by the unchanged known-bad machine list after own lease 57296911 failed with provider_host_error. No other queried verified candidate met the existing requirements. Six approved non-GB202 SKUs were queried once; GB202 was not queried. No creation request, timer, retry reservation or spend occurred. All sixteen historical own ids are API-absent. Phase runtime remains 5.148077591179074 USD.

CODE READ: owner 12:20 broadens die eligibility, preserving provider-retry policy. `run_kit/operator_ops.py` now prefers GB205, GB203, GB206 and GB207 last. The exclusion was preserved; no eligibility, safety, budget or timeout check was waived. `run_kit/arm.sh` adds the existing T3 provenance line to its finite public keyline selectors. Default guards and the per-arm observation protocol are unchanged.

CODE READ: `expected_page_policy.json`, derived at commit 48b027267, records each anticipated row scope. GB202 and GB206 have their own 48-word pages at host branch 595; GB203/GB205/GB207 use GB202 with the explicit provisional DISPv0502/CA layout reason and agreement of 328 decoded fields. These are expected source selections. No host die, driver or page actually served was established in this attempt.

FALSIFIERS BEFORE LIVE: a future inherited-die B reaching 0x50 or refusing Code 0 is an inheritance finding for that actual die, never a GB202 result. After admitted Xid 32, guest 0x116/TDR or exit falsifies survival; no admitted Xid leaves fault survival untested. B1 remains conditional on a clean build. These hardware outcomes are all unrun.

MEASURED on 2026-10-23: `rtx50_config_green.txt` records 228 passing GPU-free tests after the configuration and keyline selector edits; AST and shell syntax checks pass. The earlier 26 mutation results remain attached to the unchanged observer/projection/protocol implementation. Publication checks occur after live work; no live experiment existed here.

--- Earlier preparation and GB202-only availability checkpoint ---

# BW3 Windows observation preparation, 2026-10-23

CODE READ: owner 11:05 authorizes one additional GB202 lease with the same phase budget (prior runtime 5.148077591179074 USD, stop 7.5, cap 8, reserve 5) and a 150-minute destruction timer armed before SSH or work. Provider replacements are restricted to pre-SSH failures under PR110. No lease has been created for this observation attempt.

MEASURED: `market_all_rental_preflight.json` at 10:00:07 UTC returned no suitable verified GB202. Its three offers were two de-verified machines and one unverified machine. No creation request was issued. This availability result, rather than a publication check, prevents the hardware run.

CODE READ: `scripts/bench/blackwell/windows_observation.py` implements an explicit expiring ON-arm capability, bound to the owned QEMU process lifetime, GPU, lease, arm and single enable. Only Xid 32 with intr 00200000 and the observed channel class is eligible. The first plain ID and first 0x0e0000xx ID are pinned for that arm; any different ID stops. Eight records are allowed per arm, including across phase gaps. Another Xid, another PID, an additional kernel error, lost journal history or failed nvidia-smi/cuInit health check stops through the existing pidfd mechanism. Default/native guard sources remain unchanged.

INFERRED: the conservative pair pin may stop after a guest channel reset produces different IDs. An optional clarification about changing IDs between pairs remains outstanding; no broader interpretation is assumed.

CODE READ: `run_kit/controller.sh` runs exact page candidate at commit 48b027267 first, then optional B1 if its build/tests succeed and at least 25 minutes remain. Both use a fresh clone of one sealed disabled 580.88 parent, the same 19 flags and one enable. QEMU source writers are serialized. The observer is ready before the old native watcher exits. Early Code 0 independently starts the 600-second hold even if a later SSH diagnostic fails. No application workloads are included.

CODE READ: B1 combines public parents 48b027267a0e05c9ffdb4a049d34f4d719fd1837 and c130aa98ac2b922202282cd793f2a6d64be1f091. The clean merge tree is 15a57dc7d10e1536277a5fc8ed93181aab947fa3 and reproducible commit object cb80cbd29b7a05f904acd822f172485fd49938a9. `run_kit/b1_commit.txt` and `b1_merge.json` retain its provenance. This object is prepared locally; a rental build must reconstruct and verify it, run the three affected crate suites, and produce its own binary receipt before B1 can start.

FALSIFIERS stated before hardware: candidate B reaching 0x50 after fourth CA7A again falsifies page sufficiency; guest 0x116/TDR or exit during the post-Code0 hold falsifies survival; no admitted Xid 32 means survival does not test recovery from that fault. B1 reaching the same fatal guest outcome after a Translated single-channel RC falsifies sufficient guest isolation for this case. Failure to build B1 means B1 is unrun, without substituting a different build.

CODE READ: guest collection includes System events 153/4101/41/1001, tolerant public dump headers and RPC ordinals observed within 40 seconds before each host Xid or QMP panic. `channel_projection.py` joins public allocation handles/classes to channel births. The Xid/token association is explicitly INFERRED from public doorbell runlist/vector fields; all reused-ID candidates and opaque original tokens are retained. Engine object classes and allocation ordinals are separate metadata. Private reply bodies are excluded. Full bounded RPC streams stay on the rental when they exceed the export ceiling; the public projection and original stream hash are retained.

MEASURED on 2026-10-23: `traces/blackwell_ab_20261023/observation_run/full_green.txt` reports 228 passing GPU-free tests; `observation_mutations.json` and `projection_protocol_mutations.json` record 26 mutations, each killed by at least two distinct tests. Existing five-line native journal text round-trips exactly through the new JSON renderer. AST, shell syntax, Black 100 and new-code line checks pass. Hardware observation, B1 build, hold, event collection and crash-header collection are unrun.

CODE READ: the exact held evidence head 160dc031ad51cda74ac3848f33386237fcd8c2d4 was pushed under the controller's explicit release and verified on origin. The inherited claims diagnostics are separately audited under `../bfirst_run/claims_baseline_audit.json`; no claims-debt or checker edits are permitted. New observation commits remain local until publication checks are resolved. No publication check may interrupt future live hardware work.

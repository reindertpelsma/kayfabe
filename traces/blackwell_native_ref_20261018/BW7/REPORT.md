# BW7 — native GB202 reference PASS; both leases closed

1. Lane, branch and request

Lane BW7. The controller separately authorized attempt 2 after attempt 1's correct identity refusal: native-template-only staged pin admission, RED then change, >=2 mutation killers, unchanged BW5/BW6 behavior, then one new RTX 5090 native Windows lease. H-REF and its falsifier remain unchanged in `attempt2/operator_plan.json`: stock Windows 580.88 reaches Code 0 and holds 600 seconds without System 1001/6008/4101 or nvlddmkm/Display error events; a 0x113/any 1001 during the hold, or no Code 0, falsifies it.

Publication branch `codex/bw7b-20261024` descends from its previous origin head `834d1c0a888fd50cbf2af0f187c230ad75d38025`, originally based on origin/master `a9105acb656ec212daf5da7b8b916387f5da1671`. Only BW7 files change. Attempt-2 RED commits are `5da1a43e0` and `7e48fac1a`; the pre-lease implementation is `2f5e34b43`. Final publication head and origin equality are recorded in `/root/codex_overnight_20261018/reports/BW7.md` and its origin receipt after push. No force-push, rebase, PR, merge, product-code change or additional retry. The earlier result is retained in `attempt1/REPORT.md`.

2. Result (<=15 lines)

- DONE — MEASURED (2026-10-24, lease 57565048; `attempt2/native_hold_result.json`, `attempt2/hold.txt`): H-REF PASS. All 11 samples, n=0..10, were Code 0 on driver 32.0.15.8088 / 580.88; first-to-last span602.024182 seconds; script elapsed602.949798 seconds; the boot identity stayed constant throughout the hold.
- MEASURED (2026-10-24, lease 57565048; `attempt2/compare0.txt`, `attempt2/compare_since_boot.txt`, `attempt2/hold.txt`): System 1001 = 0, 6008 = 0, 4101 = 0, nvlddmkm/Display errors=0 during the hold and in successful whole-boot queries before/after it. No recovery or bugcheck was observed.
- DONE — MEASURED (2026-10-24, lease 57565048; `attempt2/offer_identity.json`, `attempt2/state0.txt`): the exact BW6 machine 152098 / host 702330, Chile CL, RTX 5090; native BAR1 total 32768 MiB. GB202 is the BW6-probed die on this machine and the offer's class; no new Linux die probe was performed.
- DONE — MEASURED (2026-10-24, lease 57565048; `attempt2/compare0.txt`, `attempt2/windows_ubr.txt`): Windows 11 Enterprise LTSC Evaluation 10.0.26100.1742; bound driver 580.88; package sha256 matches the pinned 90c49f925b41ee062e02cc8094a16b6d057abfde4e85235a76d63ffe267f63a9. CODE READ (`attempt2/template_verification.json`, `attempt2/operator_plan.json`): native template/install source and Windows media URL match the reference kit.
- DONE — MEASURED (2026-10-24, lease 57565048; `attempt2/compare_since_boot.txt`, `attempt2/outcome.json`, private capture): no .dmp files in captured Minidump, LiveKernelReports, WER or MEMORY.DMP locations; native 0x193/0x1A8/0x1B8/0x141/0x113 dumps absent from this run.
- DONE — MEASURED (2026-10-24, lease 57565048; `attempt2/native_identity_result.json`): strict SSH succeeded only after latest console pin = scanned pin = Windows-marker pin for 135.012999 continuous seconds / 10 consecutive polls; elapsed 545.178245 seconds at the proof, within the fixed 1200-second first-reachable deadline. The untrusted intermediate scanned key never authorized login.
- DONE — MEASURED (2026-10-24, lease 57565048; `attempt2/capture_all_verified.json`, `attempt2/final_cleanup.json`): 93 guest manifest files, 91 .evtx logs; 133 combined guest/console files, 31769425 bytes, rehashed before exact-id destroy; API absence verified and both timers exited.
- INFERRED (2026-10-24, lease 57565048; `attempt2/outcome.json`, owner rule in `attempt2/operator_plan.json`): this native pass on the exact BW6 machine attributes BW6's Code 0 then 0x113 failure to kayfabe. The specific bugcheck root cause is not established.

3. Tests, pin timeline, hardware and spend

MEASURED (2026-10-24; `attempt2/rehearsal/expected_red.txt`, commit 5da1a43e0): policy RED ran 19 tests, 14 failures and 2 errors against the legacy settle behavior. MEASURED (2026-10-24; `attempt2/rehearsal/operator_expected_red.txt`, commit 7e48fac1a): operator wiring/budget RED ran 7 tests with 6 failures. MEASURED (2026-10-24; `attempt2/rehearsal/green.txt`, `attempt2/kit/dry_loop_pass.json`):29 lane tests pass; 122 dry-loop tests pass; static 0; both timer teardown paths exercised. The native operator's 3 replay cases cover staged success, fixed-deadline refusal without login, and rejection of non-native/wrong-template paths.

MEASURED (2026-10-24; `attempt2/rehearsal/test_native_pin.py`, `attempt2/kit/source_lock.json`): legacy `console_pin.py` sha256 remains 169f99eef6216e2ff0b69185877af20f3cc74794a875b73158c7495812b8208f; its two-match acceptance and immediate-flap refusal regressions pass. Every pre-lease Python source hash was rechecked unchanged after the hardware run. CODE READ (`attempt2/kit/native_pin.py`, `attempt2/kit/native_ready.py`): only the named native template dispatches to the new policy; 120 s stability, 2 consecutive matches and 1200 s absolute deadline are constants; mismatches/missing scans/missing pins/changed pairs reset stability; a Windows marker must also match. The legacy helper is unchanged.

MEASURED (2026-10-24; `attempt2/rehearsal/mutation_kills.json`): all 9 mutants were restored; every mutant had at least 2 killing tests.

| Mutant | Killing tests |
| --- | ---: |
| No stability window | 10 |
| Deadline constant raised | 12 |
| Unbounded wait | 11 |
| Accept scan without console pin | 9 |
| Accept older console pin | 2 |
| Mismatch does not reset window | 2 |
| Native-template scope removed | 3 |
| Old attempt budget restored | 2 |
| Strict host checking removed | 2 |

MEASURED (2026-10-24; lease 57562949 `kit/console_pin_57562949.jsonl`, repeated in `attempt2/operator_plan.json`): attempt1 scanned the earlier Linux key while the future Windows pin was latest, then had 3 missing scans, then an unauthenticated third key; the legacy guard correctly refused PinFlap and no retry followed within that grant. MEASURED (2026-10-24, lease 57565048; `attempt2/kit/console_pin_57565048.jsonl`): attempt2 recorded 27 mismatches then 10 consecutive matches across 37 polls; scanned identities included Linux, an untrusted intermediate key, then the console-bound Windows key. First SSH-reachable 21:09:32.874467UTC fixed the deadline 21:29:32.874467UTC; strict SSH succeeded 21:18:43.301747UTC. The window was not shortened or the deadline extended. This measures the granted staged wait; it does not identify the intermediate endpoint's implementation.

MEASURED (2026-10-24, lease 57565048; `attempt2/offer_identity.json`, `attempt2/state0.txt`, `attempt2/compare0.txt`): offer 56509993, machine 152098, host 702330, Chile CL, GeForce RTX 5090. Advertised Vast Linux host driver 595.91.07; measured native Windows bound driver 580.88. BAR1 total 32768 MiB, used 32740 MiB, free 28 MiB in the snapshots; no resizing. GPU hold n=1, hardware spread unavailable; all 11 device-code samples were0. This is the same box type and exact machine as BW6.

MEASURED (2026-10-24, lease 57565048; `attempt2/owned_instances.json`, `attempt2/kit/precreate_watch_ready.json`): watchdog ARMED 21:08:39.786834UTC before create 21:08:41.394136UTC; fixed lease deadline 22:53:57.786834UTC; exact-id API absence2026-10-24T21:38:43.862048+00:00. Quote $0.736111/hour, observed $0.816667/hour; runtime cost estimate$0.40889319; shared account credit 23.72422022->23.34052068, observed delta$0.38369953. This delta is not a settled exclusive invoice. Attempt2 stop $2.00 / cap $2.50 and reserve $8 preserved; one new lease created/destroyed 57565048, no retry.

MEASURED (2026-10-24; both owned-instance ledgers): BW7 created and destroyed 57562949 and 57565048 under two separate grants. Runtime estimates: attempt1$0.08285934, attempt2$0.40889319, total$0.49175253. Both API absences and timer exits are retained. Private roots: `/data/bw7_private_capture/57562949/` and `/data/bw7_private_capture/57565048/`.

4. Owner decisions

1. Attribute BW6's 0x113 failure to kayfabe under the matching-native rule; recommendation: keep the combined fix's GB202 validation open and investigate using the existing BW6 private 0x113/0x141 evidence.
2. Keep the native staged policy scoped to its template; recommendation: retain the unchanged BW5/BW6 policy and the granted 120 s / 1200 s bounds, with the staged timeline from lease 57565048 (2026-10-24; `attempt2/native_identity_result.json`) as regression evidence.

5. Limits and collection correction

The result covers one idle native hold on one machine, not applications, load, second-life cycles or general RTX 5090 reliability. No causal bugcheck diagnosis or closed-binary analysis was performed. The installed OS edition/build, template hash and driver package hash agree with the requested kit; the native install ISO hash was not retained across the installer's destructive reboot, so media provenance is CODE READ from the identical template/media URL, not a new native ISO digest measurement.

MEASURED (2026-10-24, lease 57565048; `attempt2/compare1.txt`): the first post-hold summary exited 1 on DateTime.Parse of the hold JSON selection. The completed hold and raw event export were unaffected. MEASURED (2026-10-24, lease 57565048; `attempt2/compare_since_boot.txt`, `attempt2/capture_guest.txt`): a separate read-only whole-boot query exited 0, reports all requested event counts 0 and dump count 0, and covers the complete hold; all 91 event logs were captured privately. The failed summary is retained transparently; no driver, pin guard, timeout or experiment arm changed to obtain these counts.

6. Cleanup

MEASURED (2026-10-24, lease 57565048; `attempt2/final_cleanup.json`): API absence independently rechecked at2026-10-24T21:38:46.012110+00:00; both owned timer PIDs absent; native readiness, setup wait, hold, collectors and capture pull finished. Nothing owned remains running. No dev target directory, GPU lock or kayfabe/QEMU job was created. Both private captures remain; only bounded text, hashes and receipts are public.

The identity extension boundary is CODE READ from `attempt2/kit/native_pin.py` and `native_ready.py`: only native template 183b10af7fd788d08aca02e31811d70d may wait through unmatched staged keys. It never admits an unpinned scan or an older console pin. Every mismatch resets the continuous 120 s window; acceptance requires the latest console pin, the scanned key and the Windows-marker pin to agree, with at least 2 consecutive matches. The 1200 s deadline is fixed at first SSH-reachable, never restarted by a key transition; failure is a real PinFlap followed by collection and teardown, with no further lease retry. Malformed identity data remains a refusal. BW5/BW6 retain the original byte-identical settle helper and immediate-flap behavior. No refusal was reclassified as provider-retryable.

MEASURED (2026-10-24; `attempt2/publication_gates.json`): final publication claims new=0, frozen code changes=0, GPU UUID new=0/stale=0, secret/IP/binary and whitespace checks pass. No debt baseline changed. All publication paths belong to BW7; no master/integration merge or PR. Coordinator-directed local worktree cleanup follows a clean verified push, with raw captures retained.

CODE READ (`attempt2/publication_normalization.json`): two command outputs have trailing whitespace/EOF blank lines normalized for publication; original byte copies and both hashes are retained privately. This does not alter the reported Parse failure, counts, identity proof or hold evidence.

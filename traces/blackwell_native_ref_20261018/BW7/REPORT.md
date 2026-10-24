# BW7 — native GB202 reference UNRUN after identity refusal

1. Lane, branches and request

Lane BW7. Request: one RTX 5090 / GB202 native windows-on-vast lease, the BW6 Windows image and 580.88 driver, Code 0 plus a 600-second hold; collect bounded event/dump metadata and decide BW6 attribution under the owner's rule. H-REF and its falsifier were recorded before create in `operator_plan.json` (execution source commit `6fcf93ca8`, following EXPECTED RED `e39cbc1ba`).

Execution branch `codex/bw7-20261024` remains local. Publication branch `codex/bw7b-20261024` is created from origin/master and contains only BW7 new files under `traces/blackwell_native_ref_20261018/BW7/`. Runtime receipts retain the original execution location `traces/blackwell_native_ref_20261024/BW7/`; code and runtime receipt bytes are preserved; `publication_copy.json` records report attribution edits. Final publication head and origin equality are recorded in the lane's external report `/root/codex_overnight_20261018/reports/BW7.md` and final message. No product changes, force-push, PR, merge or second lease.

2. Result (<=15 lines)

- STOPPED — MEASURED (2026-10-24; lease 57562949; `attempt1/ready.txt`, `kit/console_pin_57562949.jsonl`): settled readiness recorded four mismatches then an actual `console_pin.PinFlap`; no authenticated SSH or Windows probe ran. The real identity refusal ended hardware work; no retry.
- UNRUN — MEASURED (2026-10-24; lease 57562949; `outcome.json`): H-REF, Code 0, 600-second hold, System 1001/6008/4101 and nvlddmkm/Display event counts, BAR1 and native 0x193/0x1A8/0x1B8/0x141/0x113 dump presence have no samples. This is neither a native pass nor a native GPU failure.
- DONE — MEASURED (2026-10-24; lease 57562949; `attempt1/offer_identity.json`): the selected offer was 56509993 on BW6's exact Chile machine 152098, host 702330, advertised RTX 5090 / GB202 and host driver 595.91.07. No in-box driver/die/BAR1 probe was possible.
- DONE — MEASURED (2026-10-24; `template_verification.json`, lease 57562949 `attempt1/rent.txt`): the native template was `183b10af7fd788d08aca02e31811d70d`; its installer sha256 matches BW3 Round REF exactly. CODE READ (`operator_plan.json`, `kit/media_manifest.json`): intended media is Win11 Enterprise LTSC 2024 evaluation and Windows driver 580.88; actual installed/bound versions remain unverified.
- DONE — MEASURED (2026-10-24; lease 57562949; `attempt1/pre_ssh_console_capture.json`): seven available console files, 798942 bytes, were pulled and sha256-rechecked before normal teardown. Private raw root: `/data/bw7_private_capture/57562949/`; no Windows guest capture exists.
- DONE — MEASURED (2026-10-24; lease 57562949; `attempt1/owned_instances.json`, `attempt1/final_cleanup.json`): exact-id destroy and API absence verified; both timers exited after absence; one instance created and one destroyed, 57562949.
- INFERRED (lease 57562949; `kit/console_pin_57562949.jsonl`): the readiness wiring encountered a staged native installation identity transition; the later scanned key is not authenticated as either stage. No provider/security root cause or Windows GPU result is established.
- INFERRED (lease 57562949; `outcome.json`): BW6 attribution remains unresolved because this reference produced neither a native pass nor a native bugcheck.

3. Tests, hardware, identity evidence and spend

MEASURED (2026-10-24; `rehearsal/expected_red.txt`, commit `e39cbc1ba`): 13 tests ran against the unadapted reference and produced 15 failing assertions/subtests. MEASURED (2026-10-24; `rehearsal/green.txt`, `kit/dry_loop_pass.json`): 37 lane tests pass; final pre-lease dry loop 130 tests, static problems 0; both watchdog teardown paths exercised on a fake provider. The exact readiness branch passed three cases: absent then two matches, two matches before login, and mismatch refusing before login. These fixtures did not establish that staged native installation key transitions would authenticate successfully.

MEASURED (2026-10-24; `rehearsal/mutation_kills.json`): five mutants restored after checking; each had at least two killing tests.

| Mutant | Killers |
| --- | ---: |
| Scope reverted to GB205 / RTX 5070 | 2 |
| Price ceiling raised | 2 |
| Old stop/cap restored | 2 |
| Settled pin replaced by old single-pin helper | 4 |
| Normal capture gate removed | 3 |

MEASURED (2026-10-24; lease 57562949; `kit/console_pin_57562949.jsonl`): check 1 at 20:51:50.196300 UTC scanned the console-published Linux key while the latest console key was explicitly marked for future Windows. Checks 2–4 returned no scan key. Check 5 at 20:52:50.608343 UTC scanned a different key, matching neither the original Linux scan nor the latest Windows pin, and raised PinFlap. The guard was not changed or bypassed. All full console text remains private; public evidence carries key lines, fingerprints and hashes.

MEASURED (2026-10-24; lease 57562949; `attempt1/owned_instances.json`, `kit/precreate_watch_ready.json`): watchdog ARMED 20:47:53.121921 UTC; create receipt 20:47:54.698495 UTC; API absence 20:53:59.955959 UTC. Fixed deadline was 23:08:38.121921 UTC, never extended. Quote $0.736111/hour; observed $0.816667/hour. Estimated runtime cost $0.08285934; shared account credit 23.80500587 -> 23.78923248, observed delta $0.01577338. The credit delta is not a settled exclusive invoice. Stop $2.50 / cap $3.00 / reserve $8 preserved. Hardware leases n=1; native Windows measurement n=0, spread unavailable.

MEASURED (2026-10-24; `market_watch/status.json`): an initial cached preflight had no qualifying offer; the fresh read-only watcher matched the exact preferred machine on its first poll, 20:47:15 UTC, and exited. No denied country or machine blacklist was used; VM-deverified and over-price offers stayed excluded.

4. Owner decisions

1. Keep BW6 attribution open; recommendation: no kayfabe attribution or exoneration from this UNRUN reference.
2. Before a separately authorized native lease, add a tests-first readiness design for the installer's Linux, reinstall and Windows identity stages, preserving console binding and settled pinning; recommendation: reproduce this refusal from the retained key receipts, without retrying BW7 or weakening the current guard.

5. Limits

Native Code 0, Windows image/build and bound driver, continuous hold, bugcheck/TDR absence or presence, System event counts, display-dump presence, BAR1, actual probed die/host driver and GPU health were not measured. No inference from a missing probe is a native failure. No dump-header decoding or closed-binary analysis was performed. The same machine/template/media targets are established; the requested native-versus-kayfabe comparison remains unmeasured.

6. Cleanup

MEASURED (2026-10-24; lease 57562949; `attempt1/final_cleanup.json`): API absence independently rechecked at 20:54:49.706859 UTC; both owned timer PIDs absent; all owned foreground jobs exited; the read-only watcher exited on MATCH. No remote work, QEMU, GPU lock or dev target directory was started by BW7. Nothing owned remains running. Raw console evidence remains private; local execution commits are retained.

MEASURED (2026-10-24; `publication_gates.json`): publication claims new=0, frozen files changed=0, GPU UUID gate clean, and secret/IP/binary checks pass. Publication is restricted to BW7 new files; code lines fall under the COMMON.md traces/ exemption. No claims or clippy debt baseline changed.

STATUS: SUPERSEDED, 2026-10-23. Historical first-takeover report for lease57324106. The owner09:45 correction in COMMON.md supersedes its publication-stop recommendation; see bfirst_run/RESULT.md for the subsequent live B result and real safety stop.

1. BW3; branch `codex/bw-native-edge-probe-20261022`; head `5e3f448c1a13a1c653604e1b7ae2d489394c3d6b` verified on origin after the push. Asked to repair crash-header evidence parsing, run same-die arm B only with >=25 minutes left, retain A/B falsifiers, and retire handed-over lease57324106 without extension.

2. Result (2026-10-23):
- DONE — CODE READ at5e3f448c1: `parse_hexline` retains unsupported public header bytes and records "unparsed header, raw bytes retained in evidence". Strict `parse` and malformed transport refusal remain unchanged. No guard, Xid, product, guest-flag or timer change.
- DONE — MEASURED lease57324106, `traces/blackwell_ab_20261023/dump_headers_on_derived.jsonl`: the valid minidump parses as0x50; the zero-signature WATCHDOG header remains explicitly unparsed with raw header hex.
- DONE — MEASURED lease57324106, local `arms/on_derived/rpc_timing.jsonl` and `allocations.jsonl`:278 canonical RPCs, fourth CA7A at ordinal277, four public success headers with sequences272/274/276/278. QMP and Windows event1001/WER independently corroborate0x50. The minidump time is guest time.
- PARTIAL — MEASURED at5e3f448c1: tests and parser fix were committed/pushed, but the mandatory claims gate failed (`existing-debt=3141 new=817 resolved=13`). The shell lacked fail-fast and proceeded despite that failure and CRLF whitespace diagnostics. This was my execution error; no gate waiver is claimed, no debt entry was added, and no force push was used.
- NOT DONE — MEASURED lease57324106, final_state.json: arm B never started, no on_measured marker, no measured-page outcome. Work stopped under the lane stop rule after the publication-gate failure, although >25 minutes remained. No new allocation was taken.
- DONE — MEASURED lease57324106, FINAL_BOX_AUDIT.json: zero QEMU, controller exited, GPU lock free, exactly five accepted native Xids and no XID_STOP; product6e3fd3fd6/readiness9ff51a6e9/native618f04318/candidate48b027267 all clean.
- DONE — MEASURED lease57324106, destroy_receipt.json: exact-id API absence verified2026-10-23T07:18:51.364057UTC. Only afterward, timer PIDs1089302/1089314 were cancelled after starttime/cmdline verification; both are gone. Deadline07:49:39UTC was never extended.

3. Tests and mutation:

| Check | Before | After |
| --- | --- | --- |
| Evidence parser regression tests |4 run,3 errors,1 pass at3d72c7097 EXPECTED RED |4 pass at5e3f448c1 |
| Existing capture helpers |18 pass |18 pass |
| Guard regression tests |Not rerun before edit |54 pass, guards_green.txt |
| Revert fallback mutant |Not applicable |Killed by3 new tests; mutant_revert_fallback.txt |

MEASURED at commit `5e3f448c1`: 76 tests pass across these three suites. Python Black line length100 applied; changed code lines <=100. No Rust/product build or new GPU workload. No performance measurement. Existing hardware A sample countn=1, no spread; Bn=0.

4. Owner decisions:
1. Recommend repairing the claims-gate state and reviewing the accidental noncompliant push before any additional publication; do not add debt entries to bypass it.
2. Recommend a separately authorized future same-die A/B lease only after publication requirements pass; this lease is destroyed and cannot resume. Reproduce A, then B48b027267 using the same disabled parent,19 flags,one enable per arm and unchanged guards.

5. Verification limits and exact handoff:
- INFERRED falsifier stated before B: B reaching the same fourth-CA7A/0x50 falsifies measured-page sufficiency. A not reproducing prevents causal attribution. B was not run, so page sufficiency remains unresolved.
- CODE READ: frozen controller and collector are in `traces/blackwell_consttab_ab_20261023/run_kit/` in the read-only precomp worktree; original runtime controller was `/root/sweep2/controller.sh`. Remote evidence parser was never changed before destruction.
- MEASURED lease57324106: baseline disabled580.88 parent was prepared and sealed; A used6e3fd3fd6. B48b027267 binary SHA256 was4fbe887c8a1e1be0933cb25d39df4c3412de5d7ab16f09003f0b3c296553f054, built separately without B1; its source/build receipt is in measured_binary.json. The destroyed rental held the VM images and binaries; no executable/dump body was copied back.
- Published evidence at5e3f448c1 includes the parser proof, original collector error, raw public headers, parsed A header and measured build receipt. Additional A metadata, final audit, destruction receipt and this handoff remain LOCAL/untracked under traces/blackwell_ab_20261023 because further publication is blocked. Existing committed collector text retains CRLF whitespace and must be normalized in a future authorized append-only repair, never rewritten.
- Public header bytes contain no module/context/memory analysis; unsupported WATCHDOG data has no invented bugcheck interpretation. The parser fix is evidence-only and is not a product compatibility claim.
- No hosted-CI status claimed; required local claims gate is red. No arm-B Code0/43/hold/apps or measured-page outcome was verified.

6. Cleanup — MEASURED on 2026-10-23, lease57324106 destruction receipt: lease absent and both inherited timers gone. No BW3 background jobs remain; all commands exited. No BW3 cargo target directory was created, so none requires deletion. The final pre-destroy audit proved the GPU lock released; the rental is destroyed. Other leases and trusted host were untouched. No other agent/session was contacted.

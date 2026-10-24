# BW6 — GB202 hold failed; lease closed

1. Lane, branches and request

Lane BW6. One GeForce RTX 5090 / GB202 lease to test combined fix `007ba1eb03db9e21d351e465e95b46cbc3134a95`: default flags, one enable, Code 0 and a 600-second hold with no Xid, TDR or bugcheck. Native box gates and bare suite run first. The pre-lease H-FIX/falsifier is in `operator_plan_b1r.json`; after a falsifier, collect and stop without fix iteration.

Execution/evidence branch `codex/bw6-20261024` remains LOCAL. Publication is `codex/bw6b-20261024`, created from origin/master after teardown, with only `ROUND_B1r_bw6` and its report. Execution evidence head is local commit `4e990f69d4c0491e90b1c6b4b73ffb2edb8df51a`; publication base is commit `a9105acb656ec212daf5da7b8b916387f5da1671`. The external operator report records the final publication head after origin verification. No rebase, force-push, PR, integration merge or product-fix change.

2. Result (<=15 lines)

- DONE — MEASURED (2026-10-24, lease 57556167; `attempt1/arms/on_fix/bw6_result.json`): H-FIX did not hold. One enable reached Code 0, then Windows bugchecked 0x113 / parameter 1 0x2b. The formal hold did not start: 0 timed hold seconds, arm exit 255. No second product arm or fix iteration.
- MEASURED (2026-10-24, lease 57556167; `early_code0.txt`, `private_dump_metadata.json`): Code 0 at 20:11:10.2336308 UTC; fatal dump timestamp 20:11:25.529027 UTC. The roughly 15-second interval between samples does not prove a continuous Code 0 hold.
- MEASURED (2026-10-24, lease 57556167; `host_kernel.log`, `collect_on_fix.txt`, `final_remote_audit.out`): arm Xid count 0; System 1001=1, 6008=0, 4101=0, Kernel-Power 41=1. Five intentionally marked native Xid controls are separate from the arm; exact history matched.
- DONE — MEASURED (2026-10-24, lease 57556167; `attempt1/native_baseline/`, `native_backup_ack.json`): native gates 9/9 + gate10 PASS, bare suite 29/29, CUDA 4/4 and GR PASS; 3 valid pinned fuzz repeats plus the unpinned control had no finding. Native benchmark/product baseline revision is `6e3fd3fd6`.
- DONE — MEASURED (2026-10-24, lease 57556167; `observed_fix_binary.json`): exact fix source/parents verified; 2114 tests passed, 4 ignored; QEMU sha256 `05d3d46f4e8429f5ffebacbd7ca9f5156f7d6f1141b4b10ce84732b174347448`, identical to BW5 GB205.
- MEASURED (2026-10-24, lease 57556167; `route_census.json`): 1 route-declined line for Blackwell class 0xca6f, 0 private-read-only-ring births, 17 USERD-relay log lines. This verifies the selected route, not the cause of the bugcheck.
- PARTIAL — CODE READ (`/root/codex_overnight_20261018/reports/BW3.md`, Round REF): the prior native Windows reference is GB205/RTX 5070. No matching GB202 native Windows reference passed here, so this Windows failure is not yet attributed to kayfabe under the lane rule.
- DONE — MEASURED (2026-10-24, lease 57556167; capture and teardown receipts): all 176 private files were sha256-verified and rechecked before exact-id destroy. API absence verified 20:18:00.956538 UTC; timers cancelled after absence. No live lease remains.

3. Hardware, arm and dump evidence

MEASURED (2026-10-24, lease 57556167; `owned_instances.json`, `lease_metadata/bar1.json`, native metadata): offer 56509993, machine 152098, Chile CL; GeForce RTX 5090 / probed GB202, host open driver 595.91.07, Windows 580.88 (`32.0.15.8088`). Offer rate $0.736111/hour; observed $0.833333/hour. BAR1 total 32768 MiB; no resize attempted. Initial disk free 167 GiB; host preflight SOUND with 15 visible cores and 47 GiB available RAM. Hardware n=1; spread unavailable.

MEASURED (2026-10-24, lease 57556167; `attempt1/arms/on_fix/private_dump_metadata.json`): parsed public DUMP_HEADER64 metadata from the privately captured files gives this sequence; raw dumps and header hex stay private.

| Item | UTC on 2026-10-24 | Result |
| --- | --- | --- |
| Early device state | 20:11:10.2336308 | Code 0, driver 32.0.15.8088 |
| Display dump 0x1A8 | 20:11:10.642426 | Present, 330560 bytes |
| Display dump 0x1B8 | 20:11:10.879169 | Present, 1296245 bytes |
| Live dump 0x141 | 20:11:15.673239 | Present, 1182012512 bytes |
| Fatal dump 0x113, P1=0x2b | 20:11:25.529027 | Present, 3356081 bytes; duplicate WER copy has the same sha256 |
| Display dump 0x193 | — | Not found in this captured set |

MEASURED (2026-10-24, lease 57556167; `attempt1/capture/verification_before_destroy.json`): box 4/4 and on_fix 172/172 files verified, failed=0, not_pulled=0; every local file hash was rechecked at 20:17:57.680247 UTC before destroy. Private root: `/data/bw4_private_capture/57556167/`. Public evidence contains bounded text, parsed metadata, hashes and receipts; `publication_omissions.json` records the raw header-hex omission.

MEASURED (2026-10-24, lease 57556167; `owned_instances.json`): created 19:41:28.935107 UTC, destroyed with absence at 20:18:00.956538 UTC. Watchdog ARMED 19:41:25.843768 UTC before create, deadline 22:11:25.843768 UTC; settled pin matched twice before login. One lease created and one destroyed: 57556167. Estimated runtime cost $0.507412; observed account credit 24.273008 -> 23.805006, delta $0.468002. The account delta is not a settled exclusive invoice. Stop $3.00 / cap $3.50 and $8 reserve preserved; no pnputil/reload arm.

MEASURED (`rehearsal/expected_red.txt`, commit 23cff5302): RED 18 tests, 9 failures, 1 error, 8 passes against inherited B1q behavior. MEASURED (`rehearsal/b1r_green.txt`): adapted suite 143 passed. MEASURED (`rehearsal/resume_dry_loop_pass.json`): fresh pre-lease dry loop 574 passed, static 0; pin replay rehearsal 3 passed. MEASURED (`rehearsal/mutation_kills.json`): all five mutants below had at least two killing tests and were restored.

| Mutant | Killing tests |
| --- | ---: |
| SKU reverted to GB205 / RTX 5070 | 3 |
| Price ceiling raised | 8 |
| Stop reverted to $2.00 | 5 |
| Extra arm added | 5 |
| Label reverted to BW5 | 3 |

4. Decisions for the owner

1. Run native windows-on-vast on matching RTX 5090 / GB202 before assigning this failure to kayfabe; recommendation: make that the next separately authorized experiment, with driver 580.88 and the same hold.
2. Keep GB202 confirmation and the display/init dump item open; recommendation: do not promote this result as a GB202 pass, and preserve the private 0x113/0x141/0x1A8/0x1B8 evidence for the owner.

5. Limits and provenance

The matching native Windows reference, bugcheck root cause, continuous Code 0 survival, application behavior, second-life pnputil and Linux reload were not verified. No claim is made that the Linux-native pass proves Windows-native success. No fix was iterated. Dump parsing reads only the documented public header fields; no closed-code analysis or RE-derived implementation values were introduced.

CODE READ (`operator_plan_b1r.json`): the controller allowed rental with local BW6 commits despite inherited claims findings, then required publication of BW6-authored files only on a new master-based BW6b branch. The pre-run publication stops and their exact diagnostic output remain historical evidence in `rehearsal/`; no claims/clippy baseline or lease guard was edited to pass a gate. MEASURED (2026-10-24; `publication_gates.json`): BW6b claims new=0, frozen changed=0, dependency violations=0, GPU-UUID new=0/stale=0, secret/IP/binary and whitespace checks PASS. All changed code is under the COMMON.md traces/ line-length exemption. The root report records BW6b's final origin head. CODE READ (`publication_normalization.json`): publication removes only extra EOF blank lines from five text files; original execution receipts and their hashes are preserved, with both copy hashes recorded. No guard behavior changes.

6. Cleanup

MEASURED (2026-10-24, lease 57556167; `attempt1/lease_metadata/closure.json`, `final_remote_audit.out`): API absence independently rechecked; both owned timers exited; QEMU 0, controller exited, GPU lock released, host cuInit/device-count health OK, all pinned source trees clean. Nothing owned remains running. No dev target directory was created and `/workspace/bw6-target` does not exist. Raw private captures remain under the required private root. Local BW6 history is retained and never pushed.

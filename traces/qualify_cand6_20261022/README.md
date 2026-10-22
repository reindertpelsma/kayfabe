# Q1 candidate-6 qualification

STATUS: RESUMED 07:40 CEST; STOPPED at native reader bound; bounded-reader approval pending, 2026-10-22. Original process/history stops remain resolved.

MEASURED: candidate f7cf3ed81200077e55ee7769c635887661f3f990 fetched from origin. Q1 evidence branch: codex/qualify-cand6-20261022. Start 05:01 UTC; hard host-restoration deadline 10:00 UTC. No product behavior change was made by Q1.

INFERRED H1: the candidate is the stated three-fix integration. FALSIFIER (before checking): different remote head, absent fix ancestry, or failed required CI job. MEASURED: remote head agrees; CI run 39725599154 was queued at first check. MEASURED: stated ancestry verified; CI remains queued at final check (05:24 UTC).

INFERRED H2: all candidate GPU-free suites remain green and clippy adds zero diagnostics versus e64b8e5f6. FALSIFIER (before execution): any failed test, rustfmt failure, or added source-bound clippy diagnostic. MEASURED PARTIAL: all-kf test command exited 0 with 3787 passed / 0 failed / 20 ignored; clippy and rustfmt UNRUN after safety stop.

INFERRED H3: merged regression tests discriminate each fix. FALSIFIER (before mutation): restoring the old descriptor budget, handle privilege authorization, or >64-space refusal kills fewer than two tests. Result UNRUN after mandatory safety stop.

INFERRED H4: candidate passes native 9+10 and Linux 30/30 under the approved exact-match intentional-fault policy. FALSIFIER (before execution): failed/missing verdict, non-USER birth, missing completion, or any new unmatched host Xid. Result UNRUN after mandatory safety stop.

INFERRED H5: all nine SPECviewperf workloads and available production regression smokes complete on the candidate. FALSIFIER (before execution): benchmark failure/kill, guest nvlddmkm 153 or Display 4101, new unmatched host Xid, or descriptor/scan budget refusal. Require exercised large-Pointer counters. Result UNRUN after mandatory safety stop.

CODE READ: lane requires native-before-guest, candidate-built binary, independent Windows CoW, production flags, pinned K1 runner, and no master/candidate push. Raw logs stay outside git. Measurements and limitations are recorded below.

CODE READ — hardware stop, before any GPU workload or VM boot: candidate `scripts/fastguest/run_fast_guest.sh:79` executes `pkill -9 -f '[k]f-fastguest'`. A process name supplies no ownership proof. This conflicts with the lane's own-PIDs-only instruction. The sweep branch's launcher instead refuses an existing process, but substituting its different suite source silently would defeat exact-source qualification. No launcher was executed or changed.

MEASURED — trusted host whole-boot kernel journal contains 533 historical Xid records at preflight. CODE READ — `codex/sweep-ada-20261022:scripts/bench/native_xid_guard.py` requires exactly five source-matched native faults (`approve_native`, lines 82–96), and later compares against the whole-boot kernel snapshot (`kernel_snapshot`, lines 399–407; `supervise_guest`, lines 341–346). Its unchanged path cannot accept this host history as a new five-fault baseline. This is a preflight compatibility finding, not an executed guard refusal or a newly observed candidate Xid. No records were removed, no allowlist broadened, and no cursor workaround was implemented.

CODE READ — binding stop instruction: “If something contradicts the code or these instructions, STOP and write down options; never improvise a design change.” Also: “If a step is blocked or refused by your own safety checks, STOP and report; do not reword or work around it.” Q1 stopped hardware preparation and launched no further qualification commands. The previously started local full-suite command completed without launching another qualification command; final counts are recorded below.

INFERRED options for owner: (1, recommended) authorize a separately reviewed, tests-first harness change that proves process ownership and preserves an exact pre-run journal prefix while admitting only the five fresh native and exact guest control faults; re-pin its source and resume Q1; (2) authorize an explicit approved-sweep-runner qualification profile with candidate product provenance kept separate, plus a defined journal epoch; (3) retain candidate on its integration branch, unqualified. A host reboot alone addresses journal history but not launcher ownership. None of these options has been executed.

MEASURED — candidate source was staged on the trusted host, exact head verified, but `build_kf3.sh` was never launched. Native gates, Linux suite, Windows CoW/benchmarks, production-log comparison, clippy/rustfmt and mutation checks are NOT DONE. No product behavior change was made.

MEASURED: candidate ancestry contains e64b8e5f6 and the descriptor, SA-02 and SA-03/VA-2 merges f717a3422 / 7cbacd778 / f7cf3ed81. Candidate source staging and the first evidence push were verified; only Q1 evidence branch was pushed. H1 remains incomplete because CI is queued. H3/H4/H5 were not run after the safety stop.

MEASURED: initial locked host-health check at 07:06 CEST returned cuInit 0 before/after, no reload, NVIDIA bound, GDM active and successful nvidia-smi. The GPU lock was released and immediately reacquirable; no host target directory exists. See restoration.json.

MEASURED — final GPU-free result: 22 kf-* packages, 3787 passed / 0 failed / 20 ignored across 216 test summaries, exit 0. Command: `KAYFABE_NO_KVM=1 cargo test --release --locked -j2 -p 'kf-*' --no-fail-fast`. Started 07:02:52 CEST, completed 07:22:29 CEST; global cargo flock, nice 19, two jobs, lane-owned target. Product/source tree has no diff from f7cf3ed81. Before/master count N/A; separate baseline run not performed. See tests.json for raw-log SHA-256 and command provenance. Built-in mutant test cases do not substitute for the requested hand-mutation spot check, which was not run.

MEASURED — cleanup: all Q1 target executables exited; local target deleted; no host cargo target was created. Both locks released and reacquired successfully. No Q1 host process/VM/broker/monitor remains. Final health at 07:23:47 CEST: cuInit 0, NVIDIA bound, GDM active, nvidia-smi successful; no reload. Hosted candidate CI remains queued at 07:24:11 CEST (three required jobs queued; scheduled slow job skipped). See cleanup.json, restoration.json and ci-status.json.

INFERRED historical pre-resume verdict: **FAIL merge qualification — incomplete after mandatory harness safety stop.** Keep candidate off master pending the reviewed harness option and a complete resumed qualification. The GPU-free test result supplies no hardware/application qualification. The nine-row comparison and all unavailable candidate ratios are explicit in comparison.md.

CODE READ — explicit resume authorization, 07:40 CEST: use pinned K1 Windows/SPECviewperf/Heaven arms with candidate-built QEMU. Freeze pre-arm Xid history (dmesg count + hash); judge only fresh deltas, zero tolerated on Windows, Linux intentional faults only with exact source/PID/channel/address/type and recovery proof from the approved sweep guard. Six-hour resumed time box begins now. Preserve the original 12:00 CEST host restoration deadline. The earlier stop is historical, not a current blocker.

INFERRED H6, before resumed hardware: candidate native/Linux and all nine Windows workloads plus available production smokes pass under that approved epoch contract. FALSIFIER: missing/failed gate or arm verdict, non-USER birth, mismatched intentional fault or recovery proof, any fresh Windows Xid, new guest 153/4101, benchmark kill/reboot, or Budget/Scan-budget refusal. Existing H3 mutation falsifiers remain in effect.

MEASURED resumed build: exact candidate QEMU f7cf3ed8 built with build_kf3.sh; binary SHA-256 07b7450c0d3e4eefd2fe16646d7957625cb6cd91b0b3c3fc44fdcef351585088. Host raw build log remains in the Q1 host directory.

MEASURED resumed local checks: kf-* all-target release clippy has 354 source-bound sites on master e64b8e5f6 and candidate f7cf3ed81, added 0 / removed 0. Rustfmt --all --check FAILS in three merged files: kf-mem's secaudit_all_pdb_over_64_spaces.rs, kf-rm's chanlink.rs and secaudit_internal_client_range.rs. Product source was not formatted or altered. H2's format falsifier fired; this is an exact-candidate merge blocker. See local-checks.json and rustfmt.txt.

MEASURED resumed native preflight: the unchanged sweep guard's kernel_snapshot refused the 58,766,851-byte full kernel journal at its 16 MiB bound, before any GPU workload/VM. Native attempt exited 1; no gate verdict exists. Host restoration probe returned cuInit 0, nvidia bound and GDM active; lock released. Q1 stopped hardware and requested approval for the specific bounded Xid-only snapshot, retaining the same byte bound, exact historical prefix and unchanged intentional-fault validators. No filter change or hardware retry has been executed pending that answer.

MEASURED: resumed merged-area hand mutations killed 3 tests (descriptor budget), 3 (SA-02 handle privilege) and 12 (old >64 VA refusal). Before/restored green focused suites are 20/20, 6/6 and 13/13. All product files restored. See spot-mutations.json.

MEASURED: old production run3439 qemu.log was read completely (228633391 bytes, 1401869 lines): GP_FORMAT Budget 0, USER_RING_SCAN_BUDGET 0, USER_RING_REFUSED 0. These before-evidence counts do not substitute for candidate Heaven/Minecraft runs. See production-before.json.

CODE READ: the bounded reader proposal is scripts/bench/qualification/epoch.py::read_xids. It requests all NVRM Xid records only, retains the original 16 MiB allocation/read bound and strict command/UTF-8 refusal, and preserves exact records/duplicates for the existing delta and sweep validators. MEASURED: EXPECTED RED tests-only commits precede the implementation; all 20 tests green; three reader mutants each >=2 kills. Hardware activation remains pending the explicit reader question; this proposal has not been deployed.

MEASURED resumed final state at 07:55 CEST: all independent CPU checks complete; no candidate GPU workload or VM boot occurred. Local and host cargo targets deleted, master source archive deleted, own processes exited, both locks released and acquirable. Final host cuInit 0, nvidia bound, GDM active. Candidate binary and staged K1 scripts/installer remain available; no CoW or Windows arm was launched. See build.json, resume-cleanup.json and resume-final-restoration.json.

INFERRED current verdict: FAIL merge qualification. Confirmed blocker: rustfmt failure in three merged Rust files. Hardware remains unqualified after the byte-bound preflight stop, with a concrete tested Xid-only reader proposal awaiting the explicit question's answer. The earlier process/history authorization is accepted; Q1 has not requested it again.

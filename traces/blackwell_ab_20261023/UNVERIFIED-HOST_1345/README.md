# UNVERIFIED-HOST BW3 observation, 2026-10-23

1. Lane and scope

BW3, branch `codex/bw-native-edge-probe-20261022`. Operator/rehearsal commit bc75ca0e69a2e261a942bf9051e8dc8af53a201e follows matching-rule fix a3284002089cd7b2f95e6f4949d0cadfeb4f5848. The owner13:45 instruction authorized one RTX50 attempt after final-path console replay, using the13:20 non-verified-host grant, the existing observation protocol and a150-minute destroy timer armed first. All data artifacts carry UNVERIFIED-HOST; provider inventory called this host `deverified`, which was treated as non-verified under the grant. The earlier released GB202 evidence remains at origin160dc031ad51cda74ac3848f33386237fcd8c2d4.

2. Result

- DONE — MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, `rehearsal/receipt.json`: all3 final-operator receipt cases passed before rent: matching receipt accepted, recorded absence plus later-publication control accepted after20s, derived nonmatching control refused before SSH. Equality-preserving fixture keys substitute for unavailable raw key bytes; the mismatch is a derived control, not an independent live capture.
- DONE — MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, `market.json`/`owned_instances.json`: one snapshot selected offer57143771/machine141173, RTX5060Ti/GB206, reliability0.9979332, KVM, inet1634.6Mbit/s. No bad-listed machine was used; one lease, zero provider retries. Quote0.373851852USD/h; actual0.438666667USD/h.
- DONE — MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, timer receipts: create11:44:01.156442UTC, timers1368870/1368882 ready first, fixeddeadline14:14:01.156442UTC. SSH identity passed11:47:18.019971UTC. `closure.json` proves all71 frozen rule-source hashes unchanged after identity; no rule was added during the run.
- DONE — MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, `native_baseline/`: unprivileged die probeGB206, host595.91.07, gates9/9 plus gate10, raw30/30, CUDA4/4, GR15 checks/PASS, exact5 intentional native Xid31 controls. Text backup was hashed and acknowledged before Windows arms.
- DONE — MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, `lease_metadata/bwr_binding.json`, `bwr_cold_ready.txt`, `bwr_shutdown_census.json`: fresh LTSC install, signed580.88 actual32.0.15.8088 bound/Code22, both GSP settings1, clean shutdown and sealed parent. B used the same19 flags and one enable.
- DONE — MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, `arms/on_observe_b/keylines.jsonl`: exact B48b027267 served `source=T3 coverage=measured target=GB206 donor=GB206 host=595.91.07 words=48`. Four CA7A replies succeeded, fourth posted sequence277; enable output reports actual Code0/Problem0 at guest12:53:23.2395174UTC. This is GB206 own-page initialization evidence (n=1), with no new GB202 or inheritance result.
- STOPPED — MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, `lease_metadata/kernel_xids.json`/`XID_STOP`: owned QEMU154806 produced Xid32 at12:53:25.234267/12:53:25.249250UTC, channels0x14/0x1000000b, intr00200000. The second channel is outside frozen0x0e0000xx scope. Guard pidfd-stop fired; zero records admitted. No rule was changed or expanded.
- PARTIAL — MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, `arms/on_observe_b/rpc_timing.receipt.json`:3738 canonical rows through EOF, ordinal0..3737, zero unparsed lines. The cleanup summary snapshot has3735 RPCs;3 later rows are in the final canonical stream. Both inclusive40s pre-Xid windows retain ordinals0..3724. `fatal_saved_text.json` retains the QMP stream with no guest-panic event; host termination ended observation, not a guest-survival verdict.
- PARTIAL — INFERRED UNVERIFIED-HOST lease57366202, `channel_projection.json`:13 explicit Translated births and allocation/engine-class ordinals retained; neither target0x14 nor0x1000000b has a candidate token join. The requested channel-to-guest-engine mapping remains unresolved; no semantic match is asserted.
- NOT DONE — MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, `outcome.json`: no10-minute hold, guest event153/4101/41/1001 collection, minidump header, TDR verdict or B1 ON arm. B1cb80cbd29 built cleanly (1959 tests,4 ignored), but the B guard stop prohibited further boots. After fatal, collection read saved text only, with zero guest boots or GPU queries.
- DONE — MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, `closure.json`/`COST_FINAL.json`: exact-id APIabsence12:54:33.075924UTC preceded timer cancellation. All19 ownids absent, both timerPIDs gone. Runtime estimate0.515667230USD; phase5.759571019USD<stop7.5/cap8; observed shared credit28.902164714USD>reserve5. Shared-credit changes are not exclusive lane billing.

3. Tests and limits

MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, `rehearsal/tests.txt`:3 final-operator cases passed before rent. Prior correction had3 expected-red regressions at commit71d938771, then252 green helper tests with24 pin/operator cases at fixa32840020;12 current readiness mutants each had>=2 test killers (`../UNVERIFIED-HOST_1320/lease_57363603/pin_mutations.json`). The unchanged observer/projection/protocol set has26 additional killed mutants in `../observation_run/`. No behavior changed after SSH. B1's affected-crate tests passed1959 with4 ignored; B's test verification receipt is retained. One hardware B arm, no performance series or spread claim. The exact native test counts above are correctness checks, not performance scores.

FALSIFIERS UNVERIFIED-HOST recorded before rent in `run_kit/manifest.json`: B0x50 after fourthCA7A would refute page sufficiency for the actual die; inherited-die refusalCode0/0x50 would test inheritance for that die, notGB202. Post-admitted-Xid32 guest0x116/TDR/exit would refute10-minute survival. This run passed fourthCA7A/Code0 on own-measuredGB206, but its out-of-scope Xid ended the observation. Recovery and inheritance remain untested.

4. Decisions

1. Resolve the0x1000000b host-channel identity from public source and saved evidence before considering a separately scoped follow-up. Recommendation: retain the existing guard until that class has an explicit owner decision; no further lease is authorized by this report.

5. Unverified items

INFERRED UNVERIFIED-HOST lease57366202: absence of a QMP panic before host termination does not establish survival or exclude a later0x116. The final host health sample passed just before the Xids; health was not queried after fatal. Guest events/dump headers were not collected because another guest boot was prohibited. Neither target matched an explicit traced Translated birth. No cause, fault-containment, rendering, same-die A/B comparison or page-inheritance claim follows from this run.

6. Cleanup and publication

MEASURED UNVERIFIED-HOST lease57366202 on2026-10-23, `closure.json`: exact-id destroy and APIabsence complete, timers cancelled afterwards and gone, all19 lane ids absent. The rental's QEMU/controller/builds ended with destruction; no further resource work is running. No local Cargo target was created, no local GPU lock was taken, and no foreign process/lease/worktree was changed. Publication checks run only after live collection and destruction. See `claims.json` for the unchanged inherited gate blocker; debt/checker files were not changed and the gate was not waived.

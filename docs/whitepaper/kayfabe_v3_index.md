# Kayfabe v3 companion index

**STATUS: RESEARCH, 2026-10-27 — companion to [kayfabe_v3.pdf](kayfabe_v3.pdf).** The implementation pin remains `12c4008705f17033a803e56d4d479bf1275cdb94`. Source keys and historical campaign labels live here so the argument can use plain words.

This file is an index, not a hardware qualification or a current defect list. The publication task ran no hardware. [Source identities](sources_v3.json), [result distances](result_distances_v3.json), [PR metadata](pr_snapshot_v3.json) and [historical gap topics](gap_topics_v3.json) are machine-readable companions.

## Evidence qualification

- **REPORTED:** public repository records can be inspected, but were not reproduced here.
- **UNVERIFIED, analysis not in the repository:** the supplied Windows mapping analysis attributes 297,495 of 318,747 walks (93.3%) to one channel in boot 7321 and 461,389 of 490,686 (94.0%) in boot 7391. These are harness boot identifiers, not product versions. The memo and producing command are unavailable in the public tree; arithmetic does not validate the counts. The [experimental note](https://github.com/reindertpelsma/kayfabe/blob/2196a9ad1/docs/design/ENS11_SPLIT_LATENCY_20261027.md) reports the attribution.
- **UNVERIFIED, analysis not in the repository:** supplied boot 7391 analysis at `64c50ad4` gives a 2.43 s apply and a 2.529 s completion gap. The [scope note](https://github.com/reindertpelsma/kayfabe/blob/2196a9ad1/docs/design/V3_WALK_GATE_NARROWING.md) calls it supplied evidence. No shared-clock alignment, packet identity or causal conclusion is established here.
- **Runtime unknown:** the Heaven FPS summaries from the research build that preceded the port of lazy observation and Pointer forwarding are committed, but their stated source runtime cannot be resolved. The paper removes that hash as a reproducible runtime citation. Dataset revisions are not substituted for the missing runtime. This is separate from merge status: both mechanisms are on master and default on in Pointer mode, and the research-build records are not a measurement of master.
- **UNMEASURED:** the host-plus-guest coexistence screenshot supplies no concurrent throughput, host responsiveness or fault-containment measurement.

## Result distance and rerun status

Paths count the endpoint diff under `crates/`, `qemu/` and `cuda/`. Commit counts are pin-only/result-only reachability counts over the whole graph. A nonzero result-only count denotes divergence, not a merged baseline result. All rows were **not rerun at the pin for this edition**. A zero count is an ancestry fact, not a passing test.

| Result | Runtime | Product paths | Pin-only commits | Result-only commits |
|---|---|---:|---:|---:|
| TU116 merge bar | `1915bd719bb686dfd0b1aaca8540668b626c250a` | 442 | 1987 | 0 |
| TU116 ladder | `a295e2cedd4dd1ac4f06ab4da51b7ab603e0c18e` | 442 | 1988 | 0 |
| AD104 merge bar | `fc2dadc4e07ffa5c0d930972afb8a4d002595855` | 442 | 1982 | 0 |
| GB205 bar / both ladders | `d40dafbfa9e984e2e79608697b10f86201675015` | 442 | 1985 | 0 |
| GA104 candidate | `714ec29d3fdca0a013d9d478755346b0f1af6fa7` | 64 | 629 | 52 |
| Windows matrix | `eb9f2aa33f2217355bb5a43ee54ebd2f3e09ea26` | 135 | 864 | 0 |
| Mixed Windows session | `89fc8a22291dd7bda69c3077ba829b1e934b11db` | 84 | 660 | 0 |
| Linux 61/65 apps | `2830988f183c2475a89acc989523a977c5671ab4` | 441 | 1984 | 0 |
| Linux 100 processes | `4c48ca0cdebff4fbb5caa7d7383206ef1f4495e9` | 639 | 2114 | 0 |
| Linux headless graphics | `d06833f0db5e8534837767efe17ff7ae7715dd95` | 641 | 2197 | 0 |
| VFIO native control (QEMU) | `666ea966f5ae57be54bf9a84ad77f42897f3f41e` | 132 | 822 | 0 |
| Earlier-code FPS subset (before the port) | `0a74611734dc91c78c86ff39ed9f06d866072ac4` | 130 | 821 | 0 |
| Linux matmul | `b5c9f71759b3818dac90b7ea056c1b1ad256987e` | 307 | 1518 | 0 |
| Linux llama.cpp | `9d82f2598d267732c87476da27f501aaabe2c4af` | 382 | 1754 | 0 |
| Linux eager decoding | `e5ff45ff85ea77e5d10837ac9bf9d33c3dd7ece0` | 801 | 2485 | 0 |
| Linux suspend/resume | `559db30cc6694fc1d4e571f1dc08e5abd2bf7bd5` | 169 | 938 | 0 |
| Host-call cost study | `6d95870c7c2c26cca07f0382152dd480715fd172` | 150 | 629 | 162 |
| Supplied long-wait analysis | `64c50ad472d4402a5bd7e7a3b07a628254a6565a` | 135 | 35 | 182 |
| Research-build Heaven (before the port) | Unknown | — | — | — |

## Open PR snapshot

Captured **2026-10-27T21:59:59.518305+00:00**: **87 open PRs**; 6 target `next`, 26 target `master`, 55 target other branches. The latter feature stacks predate the October 27 routing policy. A known parent is an exact branch-name match in the captured all-PR catalogue, including merged parents. Unknown does not imply independent. `next` and `master` are treated as long-lived targets. This document’s own PR head is necessarily the pre-publication head at capture.

| PR | Title | Base | Head branch / full SHA | Draft | Known parent PR(s) |
|---|---|---|---|---|---|
| [#5](https://github.com/reindertpelsma/kayfabe/pull/5) | archive: move the pre-v3 code the raw client does not need (17 -> 9 kayfabe-* crates, -117k lines) | `master` | `claude/archive-prev3-20261015` / `c9f00664b439a27b7e52c1157c7262a4c50fa444` | Yes | Long-lived target |
| [#7](https://github.com/reindertpelsma/kayfabe/pull/7) | abi/display: carry host GID and model C57E shared semaphores | `codex/sweep-linux-ampere` | `codex/hardware-regression-20261015` / `320b3317033b3755836d59abd69e6b75b2394953` | Yes | [#6](https://github.com/reindertpelsma/kayfabe/pull/6) (merged) |
| [#8](https://github.com/reindertpelsma/kayfabe/pull/8) | display: compose address-named Blackwell windows within the guest store | `codex/sweep-linux-blackwell` | `codex/fix-blackwell-scanout` / `598ca60f8b76231e023e2152cc220e07d683c1e3` | Yes | Unknown |
| [#9](https://github.com/reindertpelsma/kayfabe/pull/9) | display/rm: measure and wire exact AD102 Windows580 initialization facts | `codex/fix-blackwell-scanout` | `codex/sweep-ada-20261015` / `c3f9c13d557607e40ef95fdaf022268dde875721` | Yes | [#8](https://github.com/reindertpelsma/kayfabe/pull/8) (open) |
| [#10](https://github.com/reindertpelsma/kayfabe/pull/10) | Derive TU106 Windows channel admission and exact 590/595 display layouts | `codex/sweep-ada-20261015` | `codex/sweep-turing-20261015` / `728e6febde5e42d53c8085675e2bf1209ab38375` | Yes | [#9](https://github.com/reindertpelsma/kayfabe/pull/9) (open) |
| [#12](https://github.com/reindertpelsma/kayfabe/pull/12) | disp: add exact GB206580 caps from native Windows; validate both guest OSes | `codex/sweep-turing-20261015` | `codex/sweep-blackwell-20261015` / `24b6a4256ebdd4d806854696d417b93eddf34df4` | Yes | [#10](https://github.com/reindertpelsma/kayfabe/pull/10) (open) |
| [#13](https://github.com/reindertpelsma/kayfabe/pull/13) | Recover Windows guest command results after agent disconnects | `claude/gpu-types-20261015` | `codex/fix-qga-result-recovery` / `8aa16cd645572dadeaca065091c6623e7f418540` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#14](https://github.com/reindertpelsma/kayfabe/pull/14) | Capture early GSP queues when the validated table location moves | `claude/gpu-types-20261015` | `codex/windows-capture-revisit` / `631f087e34e2e575f303835da9f11fedd3a97df6` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#15](https://github.com/reindertpelsma/kayfabe/pull/15) | Check complete derived display caps against native test oracles | `claude/gpu-types-20261015` | `codex/caps-oracle-check` / `886f885c5ffaa9cb49df6058b1b05af61c32371c` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#16](https://github.com/reindertpelsma/kayfabe/pull/16) | Export bounded ordered GSP metadata for native trace comparison | `claude/gpu-types-20261015` | `codex/census-sequence` / `9251e45c45410e62935a01b6e82b0c11a92b58a6` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#17](https://github.com/reindertpelsma/kayfabe/pull/17) | Share CPU store views (blocked by Linux fast gate) | `claude/perf-fix-20261015` | `codex/perf-parity-20261016` / `75b247c07e65cfa48a50141b048170e3827c645f` | Yes | Unknown |
| [#18](https://github.com/reindertpelsma/kayfabe/pull/18) | observer: follow public FMC mailbox indirection | `claude/gpu-types-20261015` | `codex/observer-fmc` / `20e0ddc4b0634febb3ed7e97297dc9e31c14705b` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#19](https://github.com/reindertpelsma/kayfabe/pull/19) | cuda: require fresh GPU events for walk completion | `claude/gpu-types-20261015` | `codex/walk-completion-events` / `2bf43d609ff5126d70a1e194c5cb79366a91960a` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#20](https://github.com/reindertpelsma/kayfabe/pull/20) | bench: stop fast guests without forced kills | `claude/gpu-types-20261015` | `codex/fastguest-no-kill` / `21b72582ccdbadc771181f6efe0bf05baccffe25` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#21](https://github.com/reindertpelsma/kayfabe/pull/21) | Trace alignment: retain the complete startup and teardown sequences | `claude/gpu-types-20261015` | `codex/rpc-alignment-20261016` / `3008bc8346bdfa45162c62c400bffe3f8e7c25e6` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#22](https://github.com/reindertpelsma/kayfabe/pull/22) | GSP: bounded diagnostics for replies actually posted to the guest | `claude/gpu-types-20261015` | `codex/bounded-gsp-reply-trace-20261016` / `4d83e661fcc60f48235381308fd1091dea4351a5` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#23](https://github.com/reindertpelsma/kayfabe/pull/23) | Convert GSP heartbeat using the guest system timer offset | `claude/gpu-types-20261015` | `codex/gsp-heartbeat-clock-20261016` / `ac01b7bd306dc00202f523cf1545b3cdf9ae026b` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#24](https://github.com/reindertpelsma/kayfabe/pull/24) | PR title, not measured here: Admit independently measured GB205580 and595 capability words | `claude/gpu-types-20261015` | `codex/gb205-caps-word-20261016` / `5b8dd3a5ff43c73d8434b7cae52e96ab5bc1d9b0` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#25](https://github.com/reindertpelsma/kayfabe/pull/25) | Bound metadata RPC trace across guest policy lifetimes | `claude/gpu-types-20261015` | `codex/rpc-trace-budget-20261016` / `fee2ac9336eed69444b9edbb94a6d126d95f80e3` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#26](https://github.com/reindertpelsma/kayfabe/pull/26) | bench: select the broker guest image and driver ABI explicitly | `claude/gpu-types-20261015` | `codex/broker-guest-driver-20261016` / `c07f8e8ad4704a0a4060d499493f7ffe3bf3f2e3` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#27](https://github.com/reindertpelsma/kayfabe/pull/27) | bench: allow owned interactive QEMU to shut down safely | `claude/gpu-types-20261015` | `codex/interactive-graceful-stop-20261016` / `d9aec37d5afe4f208f79945909a5aefc159ca14d` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#28](https://github.com/reindertpelsma/kayfabe/pull/28) | bench: synchronize bounded QGA file staging without replaying writes | `claude/gpu-types-20261015` | `codex/qga-file-staging-20261016` / `5a300e75d3b1b99fa17b8d4f07a5eea136f4118f` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#29](https://github.com/reindertpelsma/kayfabe/pull/29) | display: derive Blackwell virtual tiles and required IMP assignments | `claude/gpu-types-20261015` | `codex/blackwell-virtual-tiles-20261016` / `712a0a4755c48fa40eec72724da7d9a09927845f` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#30](https://github.com/reindertpelsma/kayfabe/pull/30) | probe: isolate Blackwell CMC log-buffer refusal sensitivity | `claude/gpu-types-20261015` | `codex/cmc-log-buffer-20261016` / `73af410b683c48b5419651073fb41867f4b21312` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#31](https://github.com/reindertpelsma/kayfabe/pull/31) | disp: derive exact public580.94.02 display tables | `claude/gpu-types-20261015` | `codex/display-abi-5809402-20261016` / `98b49a1a4fb6f13bd8bad8214f63039dffa81d42` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#38](https://github.com/reindertpelsma/kayfabe/pull/38) | setobj: pre-registration (predictions and falsifiers) before any run | `master` | `claude/setobj-20261016` / `ed81e56f37f01f9a78d897b9bf5426d528223d34` | No | Long-lived target |
| [#46](https://github.com/reindertpelsma/kayfabe/pull/46) | rm: scope Windows 581 allocation diagnostic to native AD102 evidence | `claude/gpu-types-20261015` | `codex/runlist-581-allocation-20261017` / `90364d9d86a634f6319352f42549cacaa77ed9e7` | Yes | [#3](https://github.com/reindertpelsma/kayfabe/pull/3) (closed) |
| [#50](https://github.com/reindertpelsma/kayfabe/pull/50) | cursor: A1R5G5B5 cursors (the Windows I-beam): expand the GPU copy to 32 bits and compose it, host cursor converts blend-mode ones; bounded KF3_DEBUG_CURSOR dump of refused cursors; format vocabulary checked against the headers | `master` | `claude/tsg-subctx-feasibility-20261018` / `61dc81b73aa2ed25fb07d5b922f88eb450c410e7` | No | Long-lived target |
| [#51](https://github.com/reindertpelsma/kayfabe/pull/51) | disp: derive display DMA bounds and address masks per class | `codex/sweep-integrate-20261018` | `codex/sweep-display-dma-20261019` / `70afcac481a861def36a4368595524f3a41d8a57` | Yes | Unknown |
| [#52](https://github.com/reindertpelsma/kayfabe/pull/52) | Support public FMC bootstrap in Windows GSP tracer | `codex/sweep-integrate-20261018` | `codex/sweep-tracer-fmc-20261020` / `b9e2462811ef0310b72fa029c7af2a7325d06289` | Yes | Unknown |
| [#53](https://github.com/reindertpelsma/kayfabe/pull/53) | Admit generated public Blackwell NVDEC constructors | `codex/sweep-integrate-20261018` | `codex/sweep-nvdec-abi-20261020` / `aa8fdea9cb0e00fd4e5ec148461d1e805d308e0c` | Yes | Unknown |
| [#55](https://github.com/reindertpelsma/kayfabe/pull/55) | Admit the public Blackwell OFA constructor | `codex/sweep-nvdec-abi-20261020` | `codex/sweep-blackwell-ofa-20261020` / `844eb167f3a4a5b86e6b001e62ba3f6f7078402b` | Yes | [#53](https://github.com/reindertpelsma/kayfabe/pull/53) (open) |
| [#56](https://github.com/reindertpelsma/kayfabe/pull/56) | Review silent arming of Blackwell GC6 threshold notifier196 | `codex/sweep-blackwell-ofa-20261020` | `codex/sweep-blackwell-gc6-20261020` / `c58989dab89e8e033efc9ab807a29771b3d0006c` | Yes | [#55](https://github.com/reindertpelsma/kayfabe/pull/55) (open) |
| [#57](https://github.com/reindertpelsma/kayfabe/pull/57) | Add the independently measured GB202 branch580 caps scalar | `codex/sweep-blackwell-gc6-20261020` | `codex/sweep-blackwell-caps-word-20261020` / `1bba760c4c6f0d32cb48f9e11a13ccc663786300` | Yes | [#56](https://github.com/reindertpelsma/kayfabe/pull/56) (open) |
| [#58](https://github.com/reindertpelsma/kayfabe/pull/58) | Admit public Blackwell kernel GR report-semaphore releases | `codex/sweep-blackwell-caps-word-20261020` | `codex/sweep-blackwell-grsem-20261020` / `d04f8ae9ad9a6169da7bb2de68bbf0f63d4c6778` | Yes | [#57](https://github.com/reindertpelsma/kayfabe/pull/57) (open) |
| [#59](https://github.com/reindertpelsma/kayfabe/pull/59) | Derive Blackwell POSTCOMP tile capabilities from public layouts | `codex/sweep-blackwell-grsem-20261020` | `codex/sweep-blackwell-postcomp-20261020` / `680efbb32c7742026812303604cd60e37ede53e1` | Yes | [#58](https://github.com/reindertpelsma/kayfabe/pull/58) (open) |
| [#60](https://github.com/reindertpelsma/kayfabe/pull/60) | disp: derive Blackwell SDR output LUT capacity from the decoder | `codex/sweep-blackwell-postcomp-20261020` | `codex/sweep-ca73-olut-20261021` / `f62ec368f68a204a23b3707a135178cbeb6ba9f9` | Yes | [#59](https://github.com/reindertpelsma/kayfabe/pull/59) (open) |
| [#61](https://github.com/reindertpelsma/kayfabe/pull/61) | harness: select native GR fixture classes from allocated family objects | `codex/sweep-blackwell-postcomp-20261020` | `codex/sweep-native-gr-fixture-20261021` / `97dc65cc8ec9a1317bb2dcf9b828c2fb8780688c` | Yes | [#59](https://github.com/reindertpelsma/kayfabe/pull/59) (open) |
| [#62](https://github.com/reindertpelsma/kayfabe/pull/62) | bench: preserve process ownership and GPU lease during baton qualification | `claude/baton-pointer-20261020` | `codex/relay-hw-20261020` / `fd23ab1c5933d196847eba4c91eae6b97559ed51` | Yes | Unknown |
| [#65](https://github.com/reindertpelsma/kayfabe/pull/65) | fix(disp): derive the public POSTCOMP unit-width contract | `codex/sweep-win-20261021` | `codex/sweep-postcomp-units-20261021` / `296636c66264b94b591f77fb38dae71d4b13ef1a` | Yes | Unknown |
| [#68](https://github.com/reindertpelsma/kayfabe/pull/68) | Vast GPU broker: secret endpoints, prefix gate and scoped callbacks | `master` | `codex/vast-broker-20261021` / `ffd1d1d518e8550e9b44e070f070ca78e5d37fad` | Yes | Long-lived target |
| [#69](https://github.com/reindertpelsma/kayfabe/pull/69) | disp: declare PRECOMP CSC precision from public NVKMS | `claude/blackwell-resume-20261021` | `codex/bw-resume-cont-20261022-capc` / `6e7cb6ff1befc2c8920a80e615c266901a7044fc` | Yes | Unknown |
| [#70](https://github.com/reindertpelsma/kayfabe/pull/70) | docs: G1 descriptor continuation design stop and options | `codex/tdr-analysis-20261022` | `codex/descriptor-bound-20261022` / `03715b7d32554995215b477ca24ede10240c5b21` | Yes | Unknown |
| [#71](https://github.com/reindertpelsma/kayfabe/pull/71) | Propagate Translated host RC errors to guest channels | `codex/bw-resume-cont-20261022` | `codex/translated-rc-20261022` / `1c6452a276d7014bece3adf42d68cbd4edffbc18` | Yes | Unknown |
| [#72](https://github.com/reindertpelsma/kayfabe/pull/72) | Document installation and default LUT decoding with opt-in SDR and diagnostics | `master` | `codex/install-flags-20261022` / `295419c84a542ff2ac1eb428ad5d4ab7cd0800ac` | Yes | Long-lived target |
| [#73](https://github.com/reindertpelsma/kayfabe/pull/73) | Require RM privilege stamps for guest-kernel channel authorization | `master` | `codex/audit-fixes-sa02-20261022` / `69ef5973d0440dca5333b13a667557b4c7b1c57b` | Yes | Long-lived target |
| [#74](https://github.com/reindertpelsma/kayfabe/pull/74) | Chunk ALL_PDB walks at the 64-object ABI boundary (SA-03 / VA-2) | `master` | `codex/audit-fixes-va-batch-20261022` / `e6a0d2b8185dd14e137dd2a06e74d387ee636912` | Yes | Long-lived target |
| [#75](https://github.com/reindertpelsma/kayfabe/pull/75) | EXPECTED RED: VA-4 root-move invalidates and withdrawal policy question | `master` | `codex/audit-fixes-va-root-retry-20261022` / `01de2fed491aca938da2b862344dafbc56c27d85` | Yes | Long-lived target |
| [#76](https://github.com/reindertpelsma/kayfabe/pull/76) | fix(user-ring): admit Pointer descriptors by hardware format | `codex/descriptor-bound-20261022` | `codex/descriptor-bound-impl-20261022` / `468a35c9d845afc1938683540acc251bdef90e5a` | Yes | [#70](https://github.com/reindertpelsma/kayfabe/pull/70) (open) |
| [#77](https://github.com/reindertpelsma/kayfabe/pull/77) | fix: preserve scan fairness, rescan wakes and bounded dbfast service | `master` | `codex/audit-fixes-scan-service-20261022` / `1c510cb68e94ee85a717c50c93258bcb6012b1e7` | Yes | Long-lived target |
| [#79](https://github.com/reindertpelsma/kayfabe/pull/79) | fix(user-ring): bound streamed classification on workers | `codex/descriptor-bound-impl-20261022` | `codex/descriptor-bound-phase2-20261022` / `e018e784cecba94f8a6c43ab0ebc5388271ed5fb` | Yes | [#76](https://github.com/reindertpelsma/kayfabe/pull/76) (open) |
| [#80](https://github.com/reindertpelsma/kayfabe/pull/80) | perf: index quadratic RM free paths (SA-04 partial) | `next` | `codex/audit-fixes-sa04-20261022` / `eb57a341aea6b8e777fe0bf8b0216f2be55920c7` | No | Long-lived target |
| [#81](https://github.com/reindertpelsma/kayfabe/pull/81) | fix: cap live CPU window placements before host mapping | `next` | `codex/audit-fixes-sa06-20261022` / `9e8c6fe129eeffcda5807dc530c80f494a9089be` | No | Long-lived target |
| [#82](https://github.com/reindertpelsma/kayfabe/pull/82) | test: expose armed invalidate snapshot drift (SA-14 deferred) | `master` | `codex/audit-fixes-sa14-20261022` / `2363313c8c280fbd0d607542f834fa563bc0b16f` | Yes | Long-lived target |
| [#83](https://github.com/reindertpelsma/kayfabe/pull/83) | fix: share bounded admission for guest-driven diagnostics | `master` | `codex/audit-fixes-sa07-20261022` / `fa012c5b4d962e841e02b702907d4505409b3a94` | Yes | Long-lived target |
| [#84](https://github.com/reindertpelsma/kayfabe/pull/84) | fix: disable kf3 hotplug on the production PCI topology | `next` | `codex/audit-fixes-sa01-20261022` / `56bff64fa9f16615220d6cd9d063097b3dcf0232` | No | Long-lived target |
| [#85](https://github.com/reindertpelsma/kayfabe/pull/85) | Read own-die PRECOMP capabilities through bounded unprivileged RM requests | `codex/bw-resume-cont-20261022` | `codex/bw-host-precomp-20261022` / `124a37ce3dba62fc70eea47c41bf5a74bf849e1c` | Yes | Unknown |
| [#86](https://github.com/reindertpelsma/kayfabe/pull/86) | fix: candidate-6 review followups F1–F7 | `master` | `codex/review-followups-20261022` / `ee80c5eb95891874704e33d7f30164bf8c47e26f` | Yes | Long-lived target |
| [#87](https://github.com/reindertpelsma/kayfabe/pull/87) | C1: check design statuses and document dead-code cleanup candidates | `master` | `codex/cleanup-sweep-20261022` / `e50cf4da55d129e985f6c18dc6093d096ada56d1` | Yes | Long-lived target |
| [#88](https://github.com/reindertpelsma/kayfabe/pull/88) | Bound doorbell batches and remove two trap blocking capabilities | `master` | `codex/worst-trap-20261022` / `c63f66cdbc8633dece1b27d3bfc962f358154fbf` | Yes | Long-lived target |
| [#90](https://github.com/reindertpelsma/kayfabe/pull/90) | Seed dedicated-copy decoder class from the host object at birth | `integration/prod-candidate-6-20261022` | `codex/ce-class-at-birth-20261022` / `9490d7227e26dc0f5c9c618530ece0c39437cb77` | Yes | Unknown |
| [#91](https://github.com/reindertpelsma/kayfabe/pull/91) | bench: add Linux Translated RC error and isolation control | `codex/translated-rc-20261022` | `codex/translated-rc-reset-20261022` / `67df34aff3c7d24e8ba7c2cb9228d63105972b65` | Yes | [#71](https://github.com/reindertpelsma/kayfabe/pull/71) (open) |
| [#92](https://github.com/reindertpelsma/kayfabe/pull/92) | display: render mirrored OLUT behind signed matrix (Hue/Digital Vibrance freeze) | `master` | `claude/color-negative-20261022` / `e636236cf34621b0c9e53170638c54a55a22f979` | Yes | Long-lived target |
| [#93](https://github.com/reindertpelsma/kayfabe/pull/93) | Derive PRECOMP fields from virtual support before considering host facts | `codex/bw-host-precomp-20261022` | `codex/bw-precomp-fields-20261022` / `e4cec12b1b31864dfa94b0fca0c1aebbf1726e61` | Yes | [#85](https://github.com/reindertpelsma/kayfabe/pull/85) (open) |
| [#95](https://github.com/reindertpelsma/kayfabe/pull/95) | Record actual Windows Code0 before the enable command returns | `codex/bw-host-precomp-20261022` | `codex/bw-init-observer-20261022` / `1ac2b51d56ca5fc20bcc08b30c496e20d920635b` | Yes | [#85](https://github.com/reindertpelsma/kayfabe/pull/85) (open) |
| [#97](https://github.com/reindertpelsma/kayfabe/pull/97) | bench: register native GR/CE completion listeners before submission | `codex/bw-precomp-fields-20261022` | `codex/bw-native-edge-probe-20261022` / `f4879bda249946da0ce237edd84cfce442ed6480` | Yes | [#93](https://github.com/reindertpelsma/kayfabe/pull/93) (open) |
| [#98](https://github.com/reindertpelsma/kayfabe/pull/98) | ci: ratchet in-use source lines at 100 columns | `master` | `codex/line-length-20261023` / `820db0c11d16e150d256a522495f895e1da1bba6` | Yes | Long-lived target |
| [#99](https://github.com/reindertpelsma/kayfabe/pull/99) | Candidate 6d: descriptor Phase 1 + SA-02/SA-03 + CE-class fix + colour fix (hardware qualification pending) | `master` | `integration/prod-candidate-6d-20261023` / `714ec29d3fdca0a013d9d478755346b0f1af6fa7` | Yes | Long-lived target |
| [#101](https://github.com/reindertpelsma/kayfabe/pull/101) | DR1: ratchet documentation freshness and publish a weekly inventory | `next` | `codex/doc-rot-20261023` / `60f74631cea2744ec09cd1c7428e4a4afe1b8bff` | No | Long-lived target |
| [#103](https://github.com/reindertpelsma/kayfabe/pull/103) | DR1: label two dated checkpoints as historical | `codex/doc-rot-20261023` | `codex/doc-rot-mechanical-20261023` / `19dcdd9b3d50d6453a933921f96f60667002d7f3` | No | [#101](https://github.com/reindertpelsma/kayfabe/pull/101) (open) |
| [#107](https://github.com/reindertpelsma/kayfabe/pull/107) | bench: adapt Windows readiness to measured nested-host latency | `codex/bw-init-observer-20261022` | `codex/bw-init-query-20261023` / `95dac5d5eacb3d2f7b9b8763c4f9df556056599f` | Yes | [#95](https://github.com/reindertpelsma/kayfabe/pull/95) (open) |
| [#108](https://github.com/reindertpelsma/kayfabe/pull/108) | bench: add privileged read-only PRECOMP sweep measurement | `codex/bw-precomp-fields-20261022` | `codex/bw-tier3-precomp-20261023` / `2ace3d219ad4490af41c5eccff5b48ea7009a228` | Yes | [#93](https://github.com/reindertpelsma/kayfabe/pull/93) (open) |
| [#110](https://github.com/reindertpelsma/kayfabe/pull/110) | bench: retire failed provider machines and bound startup retries | `codex/bw-init-query-20261023` | `codex/bw-provider-retry-20261023` / `d53cfd9f44d7bdbc87887c2e19fb3431545d84b5` | Yes | [#107](https://github.com/reindertpelsma/kayfabe/pull/107) (open) |
| [#112](https://github.com/reindertpelsma/kayfabe/pull/112) | disp: resolve Blackwell PRECOMP through measured consttab rows | `codex/bw-native-edge-probe-20261022` | `codex/bw-precomp-consttab-20261023` / `008c97705a2c1cd043f407dc1b5c9e8eb446ad1f` | Yes | [#97](https://github.com/reindertpelsma/kayfabe/pull/97) (open) |
| [#127](https://github.com/reindertpelsma/kayfabe/pull/127) | Record Blackwell Linux driver-readiness refusal and bounded operator | `master` | `codex/blackwell-linux-sweep-20261025` / `f0fa7f4953b9b6c1d651cbf33a8ddef657919ad3` | No | Long-lived target |
| [#137](https://github.com/reindertpelsma/kayfabe/pull/137) | ci: simple Vast runner prototype (one script instead of the Worker) | `master` | `claude/vast-simple-runner-20261025` / `794d4a700d752018aedf45a28a0df77792effd2a` | Yes | Long-lived target |
| [#140](https://github.com/reindertpelsma/kayfabe/pull/140) | fix(mem): defer retired backing frees until TLB invalidate completes | `claude/cand6d-mirror-20261024` | `codex/fx1-20261026` / `edfd0271f38111b6ba32b14d7fd6cf78d8deaeb9` | Yes | Unknown |
| [#146](https://github.com/reindertpelsma/kayfabe/pull/146) | Selective sparse twin mappings (KF3_SPARSE_TWIN, default OFF) | `claude/cand6d-mirror-20261024` | `claude/spr2-20261026` / `3e52e0161c24547bf2e68d90e79b07c26a30df6d` | Yes | Unknown |
| [#160](https://github.com/reindertpelsma/kayfabe/pull/160) | TUR1: kayfabe on Turing (TU106): display vocabulary, Windows user-ring class, evidence | `master` | `claude/tur1-20261027` / `0b44315ef7f33dd19f919f2937b07c230e73e37a` | Yes | Long-lived target |
| [#163](https://github.com/reindertpelsma/kayfabe/pull/163) | bench: WINVM_KF3_DISPLAY=off headless device in win_vm.sh | `master` | `claude/tur1-headless-winvm` / `e5b9ee2fe647834f9dc141540c64c9792f884b89` | Yes | Long-lived target |
| [#164](https://github.com/reindertpelsma/kayfabe/pull/164) | kf-disp: Turing NVC57E window semaphores (display=on realizes on Turing; acquire not modelled) | `master` | `claude/tur1-disp-turing-semaphores` / `c7e085b3d82a6ff01561887c3530eb47d5133bfc` | Yes | Long-lived target |
| [#165](https://github.com/reindertpelsma/kayfabe/pull/165) | bench: windows_lane.sh and WINVM_INSTALL_CACHE for a shorter Windows lease | `master` | `claude/tur1-winlane` / `a1cd0e6283e6261d56164089ce5dac2a6251f4fd` | Yes | Long-lived target |
| [#169](https://github.com/reindertpelsma/kayfabe/pull/169) | kf-chan,kf-qemu: completion probe names a lost wake (verdict, cause, token word) | `master` | `claude/tur1-probe-wake` / `6adec88f9a5f0b847084fbcc8f3d0365b49faa0a` | Yes | Long-lived target |
| [#172](https://github.com/reindertpelsma/kayfabe/pull/172) | simplify(ci): use one action-free GPU trigger job | `master` | `codex/wk1-onepolicy-20261027` / `5ced95e19e8dc6df4a996470703e996d642da0d3` | No | Long-lived target |
| [#176](https://github.com/reindertpelsma/kayfabe/pull/176) | batch: site copy, FAQ, goals page, AGENTS branch flow (next -> master) | `master` | `next` / `858ca1e8ad4a2b264656b8e499d820856bcc7310` | No | Long-lived target |
| [#177](https://github.com/reindertpelsma/kayfabe/pull/177) | site: frontpage and README rewrite | `next` | `claude/frontpage-20261027` / `52e2c9d07315150ab9364cc5b0aca70687e63b2e` | No | Long-lived target |
| [#178](https://github.com/reindertpelsma/kayfabe/pull/178) | docs: revive the architecture paper for v3 | `next` | `codex/whitepaper-v3-20261027` / `514451723a7eaf3d3d980748ec950ac3a38fb5b9` | Yes | Long-lived target |

The main paper selects ten proposals that affect its argument. The following research prompts retain the earlier inventory’s goal/falsifier pairs without treating PR titles as code review or tested results. They are proposals by the paper.

| PR | Goal | Proposed falsifier, not a result |
|---|---|---|
| #5 | Archive unused predecessor crates without breaking the grader | The raw-client dependency closure no longer builds or a product crate acquires a legacy dependency. |
| #7 | Model shared Turing display semaphores | Native acquire/release ordering differs or one window consumes another window's event. |
| #8 | Compose address-named Blackwell windows inside the store | A window escapes the store or a native pixel/ordering reference disagrees. |
| #9 | Wire exact AD102 Windows initialisation facts | Native same-die replies or Windows acceptance disagree with the supplied fields. |
| #10 | Admit measured Turing channel and driver layouts | A same-die driver cell disagrees or an unrelated branch inherits the layout. |
| #12 | Wire measured GB206 display capabilities across both guests | Exact-head Linux or Windows still fails its application and display acceptance. |
| #13 | Recover guest command results after agent disconnect | Recovery reexecutes a side-effecting command or associates output with another request. |
| #14 | Follow early queue movement in the observer | The observer misses a real queue relocation or reads an unvalidated candidate. |
| #15 | Compare derived display capabilities to native oracles | A mutated wrong capability still passes the claimed behaviour-level oracle. |
| #16 | Export bounded ordered firmware metadata | An overflow looks like complete coverage or ordering changes during export. |
| #17 | Share CPU views of the backing store | View sharing breaks the Linux fast gate or permits stale topology/lifetime access. |
| #18 | Follow validated public firmware mailbox indirection | An unvalidated pointer becomes a source or a legitimate relocation is missed. |
| #19 | Require a fresh event for GPU-walk completion | An old completion can acknowledge a newly submitted walk or a fresh result is lost. |
| #20 | Stop fast guests without forced process kills | The normal stop leaves an owned process alive or kills a process outside the run. |
| #21 | Retain full startup and teardown trace alignment | A truncated trace looks complete or the aligner pairs different device lives. |
| #22 | Trace replies actually posted to the guest | A queued but unposted reply is counted as delivered or a posted reply is omitted. |
| #23 | Align firmware heartbeat time to the guest timer | A changed offset breaks monotonicity or produces a heartbeat in another clock domain. |
| #24 | Add measured GB205 driver-specific capability words | Repeat measurement or another branch differs, yet the consumer silently reuses the row. |
| #25 | Bound metadata traces across guest lifetimes | Reset evades the cap or old-life records are attributed to a new guest. |
| #26 | Select broker guest image and ABI explicitly | An implicit image or driver choice overrides the requested test cell. |
| #27 | Stop only the owned interactive QEMU safely | Shutdown targets a foreign PID or returns while the owned guest is still running. |
| #28 | Synchronise bounded guest-agent file staging | A lost acknowledgement replays a write or a partial transfer is used as a complete file. |
| #29 | Derive virtual Blackwell tiles and display assignments | Native acceptance and virtual assignment bounds disagree or an unimplemented feature is enabled. |
| #30 | Isolate sensitivity to one Blackwell log-buffer refusal | Matched refusal and served arms leave the outcome unchanged while reachability is proven. |
| #31 | Derive exact public display layouts for a driver tag | The native layout differs or the table is selected for a different driver version. |
| #38 | Preregister object-binding experiments | An arm is interpreted without its positive control or an outcome contradicts its recorded prediction. |
| #46 | Scope a Windows allocation diagnostic to its measured die | An unsupported die or driver inherits the diagnostic answer as a product capability. |
| #50 | Handle a legacy cursor format and bounded diagnostics | Cursor pixels or inversion differ from the native reference, or composition violates broker ownership. |
| #51 | Derive display DMA bounds from the class | A boundary operand reaches outside the owned store or a valid last element is refused. |
| #52 | Observe public Blackwell firmware bootstrap indirection | A moved queue is missed or an unvalidated location becomes a capture source. |
| #53 | Admit generated Blackwell video-decode constructors | Decode content or errors differ from the native reference despite successful construction. |
| #55 | Admit the public Blackwell optical-flow constructor | The native constructor or an actual flow workload fails despite an accepted class. |
| #56 | Review silent registration of a power-management notifier | The guest later requires a hardware action that the accepted inert registration did not perform. |
| #57 | Add one independently measured GB202 capability scalar | A same-die repeat disagrees or the row is applied to an unmeasured driver branch. |
| #58 | Admit public Blackwell kernel-graphics report releases | The report value or completion order disagrees with the native oracle. |
| #59 | Derive Blackwell post-composition tile capabilities | A native supported tile is refused or the virtual device advertises a tile it cannot execute. |
| #60 | Derive Blackwell output-table capacity | The advertised table size exceeds the emulated decoder or rejects a valid native boundary. |
| #61 | Use allocated family classes in native graphics fixtures | The fixture fails because it uses a foreign class, or false success comes from no engine work. |
| #62 | Keep process ownership during relay qualification | A harness stop kills a foreign process or loses the GPU lease while its owned process is live. |
| #65 | Derive post-composition unit widths | A boundary-value native transform disagrees or an impossible width is advertised. |
| #68 | Prototype scoped GPU CI callbacks and endpoint routing | Wrong-run or expired data is admitted, or a URL locator is mistaken for content approval. |
| #69 | Derive colour-space precision from the open display interface | A native boundary-value transform disagrees with the derived precision or field width. |
| #70 | Record unresolved command-descriptor continuation choices | A packet spanning descriptors is accepted without a consistent continuation and lifetime rule. |
| #71 | Propagate real translated-channel errors to the guest | A host error becomes success, loses its notifier, or contaminates another live channel. |
| #72 | Document normal colour decoding and optional diagnostics | Following the documented default requires a diagnostic flag or produces a different transform. |
| #73 | Require native privilege stamps for kernel-channel authority | Guest-controlled identity bits grant privileged execution without a trusted allocation stamp. |
| #74 | Split all-space walks at the ABI batch boundary | The first space beyond the batch limit is omitted, duplicated or prematurely acknowledged. |
| #75 | Expose root-move invalidation and withdrawal questions | A moved or removed root can acknowledge success while the wrong host translation remains live. |
| #76 | Admit pointer descriptors using their hardware format | Valid hardware descriptor bits are refused or reserved bits create extra address authority. |
| #77 | Preserve scan fairness and trailing rescan wakes | One hot token starves another or the final publication is stranded without a future wake. |
| #79 | Bound streamed command classification on workers | A hostile packet stream consumes unbounded work or a bounded continuation loses parser state. |
| #80 | Index expensive resource-manager free paths | A growing object graph still produces quadratic cost in the targeted free path. |
| #81 | Cap live CPU mapping windows before native placement | Concurrent admissions exceed the cap or failure rollback leaks ownership or a slot. |
| #82 | Expose invalidate snapshot drift with failing contracts | A root change can settle against a stale snapshot; ignored contracts are not a runtime fix. |
| #83 | Bound guest-driven diagnostics at one admission point | A flood bypasses the cap through another path or a refusal hides unrelated progress. |
| #84 | Prevent unsupported device unplug on the current topology | The guest can still unplug it or legitimate reset/shutdown loses the required lifecycle event. |
| #85 | Query display capabilities through unprivileged host requests | The query needs privilege or a whole physical answer enables an unsupported virtual field. |
| #86 | Repair combined-candidate review findings | A hostile-input or ownership regression remains reachable in the exact integrated candidate. |
| #87 | Check design status and identify dead code | A live proposal is deleted as dead or a source-status correction claims implementation exists. |
| #88 | Bound trap-side doorbell batches | An admitted large delta blocks a vCPU or the worker fallback loses or duplicates entries. |
| #90 | Seed the copy decoder from the allocated host class | A family-specific object is decoded with the wrong method set or privilege checks weaken. |
| #91 | Test translated errors against a live Linux victim | An induced attacker fault silently succeeds, corrupts the victim, or poisons later guest work. |
| #92 | Handle mirrored output lookup tables in the colour path | Native colour controls and captured output disagree or the target UI operation still freezes. |
| #93 | Derive virtual display fields before consulting host facts | A physical host capability enables a virtual feature that the emulated display cannot perform. |
| #95 | Observe actual Windows adapter readiness independently | An enable-command return is confused with adapter Code 0 or the observer reads a stale instance. |
| #97 | Register native completion listeners before submission | A completion arriving immediately after submission is missed or attributed to another client. |
| #98 | Ratchet source width at one hundred columns | A newly long non-exempt source line passes, or an unchanged allowed line creates false debt. |
| #99 | Qualify a combined descriptor and mapping candidate | The exact combined tree fails full-profile Windows or other-die regression, despite parent passes. |
| #101 | Inventory stale documentation with a ratchet | A planted stale active document escapes detection or a historical archive is treated as live. |
| #103 | Correct two historical status checkpoints | The new status still implies the old checkpoint is the current integrated implementation. |
| #107 | Calibrate the Windows readiness observer | A slower observer merely hides a product failure or accepts readiness without the required state. |
| #108 | Measure display fields with a privileged read-only tool | The tool alters state or its independently decoded replies disagree with the native oracle. |
| #110 | Retire failed provider machines before retrying | Retry starts before absence is verified or chooses the same failed machine or host. |
| #112 | Consume measured Blackwell display capabilities | Same-die native answers disagree, or the exact-head Windows A/B still fails the target path. |
| #127 | Record a Blackwell driver-readiness refusal | The operator labels a failed provider boot as a tested product cell or fails owned cleanup. |
| #137 | Prototype a private controller without the Worker | A real owned-instance lifecycle violates approval, spend, exact-SHA or teardown constraints. |
| #140 | Keep retired backing until invalidation acknowledges | Old backing is freed while an old translation can execute, or failed invalidate loses ownership. |
| #146 | Represent guest sparse pages with selective host reservations | Sparse-ON repeats the failure under the full production profile or breaks another die. |
| #160 | Bring up the TU106 Windows path | An exact-head guest fails the full workload despite a successful adapter initialisation. |
| #163 | Permit an explicit headless Windows device experiment | Display-disabled setup still creates a display device or is mistaken for full desktop acceptance. |
| #164 | Model Turing display-window semaphore releases | The native release oracle and guest notification disagree; acquire semantics remain unmodelled. |
| #165 | Reduce repeated Windows lease setup | A cached guest differs from the pinned clean image, leaks private state, or fails a cold control. |
| #169 | Explain a consumed completion wake | An induced lost wake remains invisible, or a harmless pending channel is called stranded. |
| #172 | Simplify the GPU trigger workflow | Removing the action dependencies changes which approved SHA is tested or skips required work. |
| #176 | Batch the documentation and site changes | The final batch tree differs from the approved tree or a publication check fails. |

## Numeric citations and immutable source identities

Every repository link names its own revision. Experimental sources outside the cover pin remain separate; a source document is not automatically an implementation or raw data. Some cited sources contain addresses or paths of the project's own test machines. Their lookup keys are `rulings`, `handoff`, `translated`, `display`, `pacing`, `driverdesign`, `constants`, `flags`, `reset`, `linuxperf`, `rmcost`, `latency`, `windowsperfmethod`, `llamahost` and `llamaguest`. Each points at an immutable pinned revision, so the text cannot be edited without changing the pin, and this paper does not repeat the values. In the current tree, `docs/OWNER_RULINGS.md` has them replaced by generic wording; the follow-up for the other documents is tracked by the owner.

| Citation | Lookup key | Source / revision | Purpose |
|---|---|---|---|
| [1] | `architecture` | [ARCHITECTURE.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/ARCHITECTURE.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Functional map and stated invariants |
| [2] | `rules` | [AGENTS.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/AGENTS.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Contributor rules and next/master merge policy |
| [3] | `debt` | [scripts/claim_ledger.py](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/scripts/claim_ledger.py) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Attribution scanner and its stated limitations |
| [4] | `readme` | [README.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/README.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Dated product and workload summary |
| [5] | `faq` | [docs/FAQ.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/FAQ.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Comparisons, sharing limits and interrupt boundary |
| [6] | `credits` | [docs/howto/credits.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/howto/credits.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Verified positioning and licence notices |
| [7] | `notices` | [docs/NOTICES.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/NOTICES.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Vendor software and trademark notices |
| [8] | `license` | [LICENSE](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/LICENSE) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Repository licensing alternatives and exceptions |
| [9] | `security` | [docs/howto/security.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/howto/security.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Threat model, deployment assumptions and dated gaps |
| [10] | `windows` | [docs/WINDOWS_GUEST_SETUP.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/WINDOWS_GUEST_SETUP.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Stock Windows setup and registry configuration |
| [11] | `requirements` | [docs/howto/requirements.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/howto/requirements.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Recorded hardware and driver cells; explicit gaps |
| [12] | `design` | [docs/design/THE_ARCHITECTURE_v3.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/THE_ARCHITECTURE_v3.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Architecture argument and seven compatibility axes |
| [13] | `rulings` | [docs/OWNER_RULINGS.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/OWNER_RULINGS.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Dated owner decisions and corrected interpretations |
| [14] | `handoff` | [docs/STATUS_AND_HANDOFF.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/STATUS_AND_HANDOFF.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Historical checkpoints and later updates |
| [15] | `status` | [docs/STATUS_DETAIL.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/STATUS_DETAIL.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Scoped evidence catalogue, with staleness warning |
| [16] | `old` | [docs/whitepaper/kayfabe_architecture.tex](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/whitepaper/kayfabe_architecture.tex) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Predecessor: method and correction history |
| [17] | `translated` | [docs/design/THE_TRANSLATED_PLANE.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/THE_TRANSLATED_PLANE.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Address rewriting and real GPU execution |
| [18] | `kinds` | [docs/design/the_three_channel_kinds.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/the_three_channel_kinds.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Privilege, address and completion by channel kind |
| [19] | `gspcode` | [crates/kf-gsp/src/lib.rs](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/crates/kf-gsp/src/lib.rs) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Firmware-facing protocol vocabulary |
| [20] | `rmcode` | [crates/kf-rm/src/lib.rs](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/crates/kf-rm/src/lib.rs) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Guest resource-manager model |
| [21] | `corecode` | [crates/kf-core/src/lib.rs](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/crates/kf-core/src/lib.rs) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Composition of trap and worker planes |
| [22] | `trapcode` | [crates/kf-trap/src/lib.rs](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/crates/kf-trap/src/lib.rs) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Bounded trap and token interface |
| [23] | `hostcode` | [crates/kf-host/src/lib.rs](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/crates/kf-host/src/lib.rs) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Authored host interface |
| [24] | `memcode` | [crates/kf-mem/src/vasmgr.rs](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/crates/kf-mem/src/vasmgr.rs) / `12c4008705f17033a803e56d4d479bf1275cdb94` | VA manager request and result protocol |
| [25] | `diffcode` | [crates/kf-cuda/src/diffmodel.rs](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/crates/kf-cuda/src/diffmodel.rs) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Diff and acknowledgement model |
| [26] | `applycode` | [crates/kf-mem/src/apply.rs](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/crates/kf-mem/src/apply.rs) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Applying changed runs and invalidation |
| [27] | `tsgcode` | [crates/kf-host/src/channel.rs](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/crates/kf-host/src/channel.rs) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Host group and member construction |
| [28] | `latejoin` | [docs/design/V3_LATE_TSG_JOINER.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_LATE_TSG_JOINER.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Late-member state and scheduling dependency |
| [29] | `relaydesign` | [docs/design/V3_WINDOWS_USER_RING_RELAY.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_WINDOWS_USER_RING_RELAY.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Windows user-work relay design |
| [30] | `lifecycle` | [docs/design/V3_USER_RING_LIFECYCLE_20261016.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_USER_RING_LIFECYCLE_20261016.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Corrected lifecycle and lazy-observation rules |
| [31] | `eventloop` | [docs/design/V3_USER_RING_EVENT_LOOP_20261014.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_USER_RING_EVENT_LOOP_20261014.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | GET versus processed marker correction |
| [32] | `lazycode` | [crates/kf-chan/src/user_lazy.rs](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/crates/kf-chan/src/user_lazy.rs) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Cache and forced-observation predicates |
| [33] | `turncode` | [crates/kf-chan/src/user_turn.rs](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/crates/kf-chan/src/user_turn.rs) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Actual relay turn and mode transitions |
| [34] | `display` | [docs/design/V3_DISPLAY.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_DISPLAY.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Emulated display and boot-layer policy |
| [35] | `planes` | [docs/design/V3_DISPLAY_PLANES_ORACLE.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_DISPLAY_PLANES_ORACLE.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Pixel and completion oracle |
| [36] | `pacing` | [docs/design/V3_DISPLAY_PACING_20261017.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_DISPLAY_PACING_20261017.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Broker credits, clock fallback and test limits |
| [37] | `video` | [docs/design/V3_VIDEO_ENGINES.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_VIDEO_ENGINES.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Engine grouping and recorded video correctness |
| [38] | `uvmdesign` | [docs/design/V3_UVM_DEMAND_PAGING.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_UVM_DEMAND_PAGING.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Fault-delivery research and stock limitations |
| [39] | `driverdesign` | [docs/design/V3_DRIVER_MATRIX.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_DRIVER_MATRIX.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Independent host and guest driver matrix |
| [40] | `constants` | [docs/design/V3_CONSTANT_STRATEGY.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_CONSTANT_STRATEGY.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Tier ladder, source separation and coverage gaps |
| [41] | `provenance` | [docs/design/V3_PROVENANCE.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_PROVENANCE.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Public provenance policy only; no analysis findings |
| [42] | `flags` | [docs/design/V3_FLAG_INVENTORY.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_FLAG_INVENTORY.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Diagnostic versus product-mode debt |
| [43] | `cooperation` | [docs/design/V3_COOPERATIVE_TIERS.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_COOPERATIVE_TIERS.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Stock path versus optional cooperation proposals |
| [44] | `gaps` | [docs/design/V3_FEATURE_GAPS.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_FEATURE_GAPS.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Dated priority and application-gap catalogue |
| [45] | `reset` | [docs/design/V3_DEVICE_RESET.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_DEVICE_RESET.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Reset stages and lifetime contract |
| [46] | `sleep` | [docs/design/V3_S3_RESUME.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_S3_RESUME.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Linux evidence and Windows resume limits |
| [47] | `migrationdecision` | [docs/design/V3_WINDOWS_USER_FIFO_MIGRATION_DECISION_20261011.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_WINDOWS_USER_FIFO_MIGRATION_DECISION_20261011.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Mediation, native relocation correction and cooperation alternatives |
| [48] | `owed` | [docs/design/V3_ENS8_OWED_RING_TESTS.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_ENS8_OWED_RING_TESTS.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Consumed-wake witness, model limits and later fix |
| [49] | `broker` | [ci/vast-broker/README.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/ci/vast-broker/README.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Broker lifecycle, current policy and unattested verdicts |
| [50] | `threat` | [ci/vast-broker/docs/THREAT_MODEL.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/ci/vast-broker/docs/THREAT_MODEL.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Owner approval and test-infrastructure trust boundary |
| [51] | `apps` | [docs/design/V3_APP_MATRIX.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_APP_MATRIX.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Application outputs, samples and failures |
| [52] | `winmatrix` | [traces/win11_userring_20261014/matrix-eb9f2aa3-20261014-2321.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/traces/win11_userring_20261014/matrix-eb9f2aa3-20261014-2321.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Windows application receipts at a named revision |
| [53] | `physevidence` | [traces/phys_operand_oracle_20261008/README.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/traces/phys_operand_oracle_20261008/README.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Physical/virtual operand hardware boundary |
| [54] | `sweep` | [traces/sw3_die_sweep_20261024/README.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/traces/sw3_die_sweep_20261024/README.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Exact GA104 driver/gate cell |
| [55] | `turing` | [traces/v3_turing_master/README.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/traces/v3_turing_master/README.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | TU116 Linux campaign and its exact checkouts |
| [56] | `families` | [traces/v3_families_master/README.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/traces/v3_families_master/README.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | AD104 workstation and GB205 Linux campaigns |
| [57] | `linuxperf` | [docs/design/V3_LINUX_PERF_BASELINE_6692e621.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_LINUX_PERF_BASELINE_6692e621.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Baseline selection, timing methods and limits |
| [58] | `winperf` | [traces/perf_status_20261017/README.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/traces/perf_status_20261017/README.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Windows experimental and production timing scopes |
| [59] | `llmmethod` | [docs/design/V3_BUILD.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/docs/design/V3_BUILD.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Language-model campaign revisions and test method |
| [60] | `llmdata` | [traces/llm_parity/vh_amd/summary.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/traces/llm_parity/vh_amd/summary.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Three-run rates, ranges and text-equality records |
| [61] | `alternativerelay` | [docs/design/V3_RELAY_SIMPLE_20261020.md](https://github.com/reindertpelsma/kayfabe/blob/c452ca7cb412852a3e6ca68d18e94f92ac625599/docs/design/V3_RELAY_SIMPLE_20261020.md) / `c452ca7cb412852a3e6ca68d18e94f92ac625599` | Retry-set relay proposal, semantic changes and unrun hardware obligations |
| [62] | `admission` | [docs/design/V3_ENS5_SPARSE_ADMISSION.md](https://github.com/reindertpelsma/kayfabe/blob/2196a9ad1103052dcc4cd1ce2bf53529b574f030/docs/design/V3_ENS5_SPARSE_ADMISSION.md) / `2196a9ad1103052dcc4cd1ce2bf53529b574f030` | Local undo, occupancy and software churn evidence |
| [63] | `rmcost` | [docs/design/V3_ENS6_RM_COST.md](https://github.com/reindertpelsma/kayfabe/blob/2196a9ad1103052dcc4cd1ce2bf53529b574f030/docs/design/V3_ENS6_RM_COST.md) / `2196a9ad1103052dcc4cd1ce2bf53529b574f030` | Native reservation churn and first-reset counterexample |
| [64] | `budget` | [docs/design/V3_ENS7_SPARSE_BUDGET.md](https://github.com/reindertpelsma/kayfabe/blob/2196a9ad1103052dcc4cd1ce2bf53529b574f030/docs/design/V3_ENS7_SPARSE_BUDGET.md) / `2196a9ad1103052dcc4cd1ce2bf53529b574f030` | Finite budgets, missing-coverage accounting and exclusions |
| [65] | `sparse` | [docs/design/V3_SPARSE_TWIN.md](https://github.com/reindertpelsma/kayfabe/blob/2196a9ad1103052dcc4cd1ce2bf53529b574f030/docs/design/V3_SPARSE_TWIN.md) / `2196a9ad1103052dcc4cd1ce2bf53529b574f030` | Default-OFF sparse representation and refresh contract |
| [66] | `walkscope` | [docs/design/V3_WALK_GATE_SCOPE_20261026.md](https://github.com/reindertpelsma/kayfabe/blob/2196a9ad1103052dcc4cd1ce2bf53529b574f030/docs/design/V3_WALK_GATE_SCOPE_20261026.md) / `2196a9ad1103052dcc4cd1ce2bf53529b574f030` | Earlier scope proposal and explicit unresolved contracts |
| [67] | `walknarrow` | [docs/design/V3_WALK_GATE_NARROWING.md](https://github.com/reindertpelsma/kayfabe/blob/2196a9ad1103052dcc4cd1ce2bf53529b574f030/docs/design/V3_WALK_GATE_NARROWING.md) / `2196a9ad1103052dcc4cd1ce2bf53529b574f030` | Target-scoping proof limits and supplied boot timing |
| [68] | `latency` | [docs/design/ENS11_SPLIT_LATENCY_20261027.md](https://github.com/reindertpelsma/kayfabe/blob/2196a9ad1103052dcc4cd1ce2bf53529b574f030/docs/design/ENS11_SPLIT_LATENCY_20261027.md) / `2196a9ad1103052dcc4cd1ce2bf53529b574f030` | Split freshness, optional continuation and native timing limits |
| [69] | `windowsperfmethod` | [docs/design/V3_WINDOWS_PERF_20261015.md](https://github.com/reindertpelsma/kayfabe/blob/9b2d7fe1bf713e25c33f3b44af5d22088909e494/docs/design/V3_WINDOWS_PERF_20261015.md) / `9b2d7fe1bf713e25c33f3b44af5d22088909e494` | Native and production Heaven protocol |
| [70] | `windowsperfdata` | [traces/perf_win_20261015/heaven_table.txt](https://github.com/reindertpelsma/kayfabe/blob/9b2d7fe1bf713e25c33f3b44af5d22088909e494/traces/perf_win_20261015/heaven_table.txt) / `9b2d7fe1bf713e25c33f3b44af5d22088909e494` | Three native and three fixed-path FPS rows |
| [71] | `experimentdata` | [traces/d3d11_gap_20261016/heaven/z16a_d3d11.sum](https://github.com/reindertpelsma/kayfabe/blob/890c6643c6022df22a5bc02804ab1dad5449e9a7/traces/d3d11_gap_20261016/heaven/z16a_d3d11.sum) / `890c6643c6022df22a5bc02804ab1dad5449e9a7` | Experimental FPS summary A |
| [72] | `experimentb` | [traces/d3d11_gap_20261016/heaven/z16b_d3d11.sum](https://github.com/reindertpelsma/kayfabe/blob/890c6643c6022df22a5bc02804ab1dad5449e9a7/traces/d3d11_gap_20261016/heaven/z16b_d3d11.sum) / `890c6643c6022df22a5bc02804ab1dad5449e9a7` | Experimental FPS summary B |
| [73] | `experimentc` | [traces/d3d11_gap_20261016/heaven/z16c_d3d11.sum](https://github.com/reindertpelsma/kayfabe/blob/890c6643c6022df22a5bc02804ab1dad5449e9a7/traces/d3d11_gap_20261016/heaven/z16c_d3d11.sum) / `890c6643c6022df22a5bc02804ab1dad5449e9a7` | Experimental FPS summary C |
| [74] | `linuxmerge` | [traces/merge_bar_20261008/README.md](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/traces/merge_bar_20261008/README.md) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Large-matmul ladder record |
| [75] | `llamahost` | [traces/v3_candidates/cand2_20261004/apps_host/llama_bench.host.log](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/traces/v3_candidates/cand2_20261004/apps_host/llama_bench.host.log) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Host llama.cpp prompt and generation benchmark, two internal repetitions |
| [76] | `llamaguest` | [traces/v3_candidates/cand2_20261004/apps_guest/workspace/apps/results/cand2/llama_bench.guest.log](https://github.com/reindertpelsma/kayfabe/blob/12c4008705f17033a803e56d4d479bf1275cdb94/traces/v3_candidates/cand2_20261004/apps_guest/workspace/apps/results/cand2/llama_bench.guest.log) / `12c4008705f17033a803e56d4d479bf1275cdb94` | Guest llama.cpp prompt and generation benchmark |
| [83] | `rootlessaudit` | [traces/rootless_audit_20261016/REPORT.txt](https://github.com/reindertpelsma/kayfabe/blob/32d7319b23e7bd51d11ff1de3f99d4df86801400/traces/rootless_audit_20261016/REPORT.txt) / `32d7319b23e7bd51d11ff1de3f99d4df86801400` | Scoped non-root run on one host; report on the unmerged branch `claude/rootless-audit-20261016` |
| [77] | `mskeys` | [Microsoft: TDR registry keys](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/tdr-registry-keys) / checked 2026-10-27 | TdrDelay is a scheduler preemption wait, default two seconds. |
| [78] | `msrecovery` | [Microsoft: Timeout detection and recovery](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/timeout-detection-and-recovery) / checked 2026-10-27 | Scheduling, preemption and engine/adapter recovery; application effects. |
| [79] | `nvidialicense` | [NVIDIA: Driver License Agreement](https://www.nvidia.com/en-us/drivers/geforce-license/) / checked 2026-10-27 | Deployment-specific permissions and restrictions; not a legal conclusion. |


Proposal citations point to mutable PR pages; the head at capture is frozen below.

| Citation | Key | Proposal | Head at snapshot |
|---|---|---|---|
| [80] | `retirementpr` | [#140](https://github.com/reindertpelsma/kayfabe/pull/140) | `edfd0271f38111b6ba32b14d7fd6cf78d8deaeb9` |
| [81] | `sparsepr` | [#146](https://github.com/reindertpelsma/kayfabe/pull/146) | `3e52e0161c24547bf2e68d90e79b07c26a30df6d` |
| [82] | `probepr` | [#169](https://github.com/reindertpelsma/kayfabe/pull/169) | `6adec88f9a5f0b847084fbcc8f3d0365b49faa0a` |

## Historical gap topics

**101 historical rows, not 101 current defects.** `G` denotes application/product blocking topics, `APP` named workload rows, `H` capability/coverage debt and `L` postponed features. Corrections in the pinned catalogue govern present interpretation. The two long history pastes have been reduced to topic names; their histories stay in the source. Numeric control names in historical topics are labels, not copied closed-binary findings.

| Historical ID | Topic |
|---|---|
| `G01` | Overlay-plane (MPO) flip completion |
| `G02` | MPO policy: advertise no overlay planes |
| `G03` | Video playback TDR in the hold (about every 5 min) |
| `G04` | HW decode must work or fall back cleanly |
| `G05` | Display halt on a refused GPU-side colour program |
| `G06` | Publish-then-raise: window event bits not owed |
| `G07` | Vidmem display writes may return before the DMA lands |
| `G08` | CE notifiers 12/23/24/26 never delivered |
| `G09` | Invalidate latency (section AD, 12 ms) |
| `G10` | Completion-audit findings not individually closed |
| `G11` | Batched-map decisions: hardware verdict pending |
| `G12` | Pending: zero-flag 15-minute run and 60-minute soak |
| `G13` | Pending: Linux regression on the combined line |
| `G14` | Translated-CE / kernel work refusals that apps hit |
| `G15` | Windows app matrix in progress; full success pending |
| `APP-kf_glgears` | Windows OpenGL gears crash; candidate passes |
| `APP-blackscholes_t` | Windows BlackScholes failure / startup timeout |
| `APP-clpeak_cuda` | Windows CUDA clpeak context-switch timeout |
| `APP-glgears_wgl` | Gears timeout after CUDA timeout on slicing candidate |
| `APP-vectorAdd_demo` | Short CUDA sample missed by periodic GPU-activity proof |
| `APP-stream_perthread_t` | Cold unrelated Torch startup consumes CuPy stream budget |
| `APP-cup8_ladder` | Short exact CUDA matmul missed by activity sampling |
| `APP-cupy_sha256` | CuPy SHA256 header discovery failure |
| `APP-cupy_cg` | CuPy cooperative-groups header discovery failure |
| `APP-managed_t` | Windows managed-memory allocation stops at translated SYNC_WAIT |
| `APP-cdp_quicksort` | New dynamic-parallelism test cannot load NVRTC builtins |
| `APP-nvenc_av1` | NVENC AV1 encodes but triggers recovery |
| `APP-nvenc_h264` | NVENC H264 encodes but triggers recovery |
| `APP-nvenc_hevc` | NVENC HEVC encodes but triggers recovery |
| `APP-cupy_asynccopy` | CuPy asynchronous-copy header discovery failure |
| `APP-minecraft_manual` | Historical manual Minecraft startup freeze; not a matrix verdict |
| `APP-gravitymark_vk` | Vulkan fails on kayfabe; same-GPU VFIO PASS |
| `APP-furmark_gl_bench` | FurMark OpenGL intermittent PBDMA DEVICE failure |
| `APP-oceanFFT_demo` | OceanFFT GUI misses per-process GPU proof |
| `APP-randomFog_demo` | RandomFog GUI misses per-process GPU proof |
| `APP-cupy_check` | CuPy shared correctness script missing CUDA headers |
| `APP-nvdec_h264` | Decode completes with guest recovery events |
| `APP-d3d11va_h264` | Decode completes with guest recovery events |
| `APP-nbody_demo_gl` | N-body GUI fails on kayfabe despite stock GUI PASS |
| `APP-scale_cuda_nvenc` | CUDA scaling/encode repeats NVENC session refusal |
| `APP-vulkan_video_decode` | Video workload passes with GPU recovery |
| `APP-edge_video_h264` | Video workload passes with GPU recovery |
| `APP-edge_video_vp9` | Video workload passes with GPU recovery |
| `APP-heaven_d3d11` | Heaven window survives but GPU proof is absent |
| `APP-heaven_opengl` | Heaven OpenGL exits early with GPU recovery |
| `APP-valley_d3d11` | Valley D3D11 wedges the guest supervisor |
| `APP-valley_opengl` | Valley OpenGL times out with GPU fault |
| `APP-vkcube` | Vulkan cube renders but fails activity proof |
| `APP-furmark_gl_stress` | CLOSED: fresh renderer log supplies existing proof |
| `APP-gravitymark_d3d12_raster` | Raster run exits without GPU activity proof |
| `APP-gravitymark_d3d12_rt` | Ray tracing run fails with driver recovery |
| `APP-torch_ai_bench` | PyTorch AI benchmark reboots Windows after a walk capacity refusal |
| `APP-gravitymark_d3d12` | D3D12 benchmark fails with recovery |
| `APP-gravitymark_gl` | GravityMark OpenGL exits with GPU fault and recovery |
| `APP-gravitymark_d3d11` | D3D11 exits without GPU activity proof |
| `APP-clpeak_ocl` | OpenCL peak crashes with recovery and temporary-file error |
| `G16` | USER-ring scan timing, late command bytes and consumed-ring wake |
| `G17` | Channel-reset refusal policy on a shared physical GPU |
| `H01` | Captured display caps page |
| `H02` | KF3_GFX_POOL_PROBE invented sizes |
| `H03` | KF3_DISPLAY_CTRL_PROBE 0x7302a3 echo |
| `H04` | Class-B flags still to hardwire (D4) |
| `H05` | Refused/unserviced controls that guests repeat |
| `H06` | Refusal audit coverage |
| `H07` | Class-B refusals left open |
| `H08` | Per-VM RUSD and utilization.gpu |
| `H09` | EVICT_CTX inference |
| `H10` | 0x2081010d stub |
| `H11` | Per-VM GPU UUID |
| `H12` | Guest driver version coverage |
| `H13` | Interrupt: GSP queue Release fence |
| `H14` | Interrupt: GP_GET refresh before non-stall raises |
| `H15` | Reboot, FLR, GSP life cycle |
| `H16` | Driver unload/reload in the guest |
| `H17` | Multi-head and hot-plug on Windows |
| `H18` | Windows Secure Boot and boot-ROM signing at product level |
| `H19` | Security audit before the first public binary |
| `H20` | Multi-GPU in one VM on Windows |
| `H21` | Shared BAR1 budget across VMs |
| `H22` | Family breadth |
| `H23` | Video lane: RUN_CAP wall without persistence mode |
| `H24` | Windows video engines NVENC/NVDEC/OFA |
| `H25` | Managed memory (UVM) |
| `H26` | Perf regression gate |
| `H27` | Host-only tooling not in git |
| `H28` | Colour: chroma/TMO decisions |
| `H29` | Colour pipeline as default, not a flag |
| `L01` | Console YUV composition fidelity |
| `L02` | Overlay depth, blend and clip on the console |
| `L03` | Scaling filter quality |
| `L04` | HDR and 10-bit video on the console |
| `L05` | GPU-resident console path |
| `L06` | Console-only things Linux already simplifies |
| `L07` | Power/thermal/P-state queries |
| `L08` | Suspend/resume, hibernate, Modern Standby |
| `L09` | Host patch tier |
| `L10` | Peer-to-peer inside a guest |
| `L11` | Mixed-family in one process |
| `L12` | NVJPG, SEC2 |
| `L13` | NVENC session cap accounting, 0x20808165 |
| `L14` | Perf work on the display and VA paths |

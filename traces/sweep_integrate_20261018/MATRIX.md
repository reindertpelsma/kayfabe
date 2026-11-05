# Sweep matrix

STATUS: LIVE, 2026-10-20 continuation task5. New stack starts048c28b9c on codex/sweep-blackwell-20261020. Current task rows and source limits are in ../sweep_continue_20261020/MATRIX.md. The rows below preserve the prior sweep, including its approved Ada broker follow-up; original evidence is on codex/sweep-integrate-20261018@0bbf3c55f. They are not a hardware acceptance of048c. Prior eight own leases remain absent; new task has no own rental yet.


Phase1:14release crates3338/0/21,49mutants with>=2killers; C4/C5 tests retained, candidate5f8f9d1a5f merged. The accepted product24906bf33 and rental0c87acda8 have identical Rust/QEMU/Cargo trees. Subsequent main commits record evidence; draftPR51/52/53 remain unmerged. Every pre-candidate4 PASS is STALE. The trusted host remains off-limits and untouched.

Budget amendment for task5: NEW stretch15USD cap/12USD stop/5USD reserve, one box/max180min/timerfirst, hardteardown2026-10-21 04:30UTC. New own spend0 so far; start APIcredit29.987922USD (shared account, not exclusive attribution). Previous task's7.676USD observed charges and eight closures are retained in codex/sweep-integrate-20261018@0bbf3c55f and SWEEP.md; predecessor Claude charges are separate. Current ledger ../sweep_continue_20261020/owned_instances.json and COST.md.

The first table preserves earlier measurements and original source revisions. Latest refreshed results follow below. A later pass on another box does not erase an earlier measured failure or establish a universal timing result. AD104 CPU oracles pass, but strict native caps equality remains RED100words; see AD104_PATH_AUDIT.md.

| Die | Guest OS / actual driver build | Current source | Current result | Historical result, now STALE | Historical source / evidence |
|---|---|---|---|---|---|
| GB202 | Linux580.159.04 | `24906bf33`, host595 | PARTIAL: raw30/CUDA4/gfx5steps+10hashesPASS; desktopFAIL black broker frame,1flip timeout/0guestXid | No current acceptance | phase2_blackwell_56788212/B2_RESULT.md |
| GB202 | Linux580.159.04 codec candidate | `45a4d91b5`, unmerged | FAIL: same black frame/1flip timeout; native9+10PASS; final B2 attempt | Separate public-format topic, PR51 | phase2_blackwell_56788212/b2_codec_candidate/ |
| GB202 | Linux595.91.07 | `24906bf33`, host595 | PARTIAL: raw30PASS (0HBwarnings), concurrency2PASS; 31zero-HB startup warnings + CUP3nonzeroHBbursts retained; CUDA4/gfx5steps+10hashesPASS; brokerUNRUN | No current acceptance | phase2_blackwell_56788212/B3_RESULT.md |
| GB202 | Windows580.88 | `24906bf33`, P19 | FAIL init/CodeTDRunverified: ONlate386RPCs/unload385; OFFcold0RPCs; suiteUNRUN. Initialguardrefusal retained | No current acceptance | phase2_blackwell_56788212/ |
| AD102 | Linux 580.159.04 | `24906bf33` | PARTIAL: CUDA4PASS; raw30/brokerUNRUN | Raw30/CUDA4/gfx10/broker PASS | `da0569a6`; E1 `ad102_4090/README.md` |
| AD102 | Linux 590.48.01 | `24906bf33` | UNRUN | Raw30/CUDA4/gfx10/broker PASS; 18 STALL/10 nvAssert | `9d6f210ba`; E1 `ad102_second_builds_20261016/linux590_apps/RESULT.md` |
| AD102 | Linux 595.91.07 | `24906bf33` | FAIL: raw29/30 (R19); CUDA incomplete (ext4, also C4); broker UNRUN | Raw30/CUDA4/gfx10/broker PASS across recorded revisions; warnings retained | Raw latest `9d6f210ba`, broker `6265b980f`; E1 `ad102_followup_20261016/RESULT.md` and `ad102_second_builds_20261016/linux595_regression/RESULT.md` |
| AD102 | Windows 580.88 / 32.0.15.8088 | `24906bf33`, exact P19 | PASS: Code0/desktop/CUDA3/D3D11+12;420.9s,0TDR | Code0, desktop, CUDA3, hardware DX11/12, 421 s zero-TDR hold; Blender/Edge short PASS | Hold/apps `77a72da9e`; E1 `ad102_followup_20261016/RESULT.md` |
| AD102 | Windows 581.51 / 32.0.15.8151 | `24906bf33`, P19 | FAIL Code43: alloc216/217OK;20801111refused219; unload254 | Guest FAIL Code43; native same-board suite/hold PASS | `9d6f210ba`; E1 `ad102_second_builds_20261016/{windows58151,native_windows58151}/RESULT.md` |
| AD103 | Linux 580 / 595 | `24906bf33` | UNRUN | No sweep acceptance claimed | Not re-audited |
| AD103 | Windows 580.88 | `24906bf33`, P19 planned | UNRUN | Older captured-page desktop result is STALE; derived-page cell unverified | E2; old exact revision not re-audited |
| AD104 | Linux / Windows | `24906bf33` | UNRUN | Owner-host history is STALE; host access forbidden to this lane | No hardware claim; CPU oracle PASS, strict stored-native-page equality FAIL100words; AD104_PATH_AUDIT.md |
| AD106 | Linux 580 / 595 | `24906bf33` | UNRUN | No current-product sweep acceptance claimed | Not re-audited |
| AD106 | Windows 580.88 | `24906bf33`, P19 planned | UNRUN | Older captured-page desktop result is STALE; derived-page cell unverified | E2; old exact revision not re-audited |
| GA104 | Linux 580.159.04 | `e8553e6ce`, host595 | FAIL: raw29/30, R34TIMEOUT180; C4controlACTIVE; CUDA/gfx/brokerUNRUN | Raw30/CUDA4/gfx10/broker PASS | `ba864a55`; E1 `README.md` / `ga104_3070/` |
| GA104 | Linux 595.91.07 | `e8553e6ce`, host595 | PARTIAL: raw30PASS/251zero-HB warnings,0guestXid/flip; CUDA/gfx/brokerUNRUN | Native gates9/9+10; guest raw30/CUDA4/gfx10/broker PASS; raw heartbeat warnings and 27 STALL/9 nvAssert | `9d6f210ba`; E1 `ga104_595_completion_20261017/FINAL.md` |
| GA104 | Windows 580.88 / intended 32.0.15.8088 | `24906bf33`, P19 | UNRUN | Binding UNVERIFIED; stored BasicDisplay 10.0.26100.1; applications/hold UNRUN | `9d6f210ba`; E1 same FINAL. Next prerequisite is verified native/VFIO install using unmodified windows-on-vast |
| TU106 | Linux 580.159.04 | `24906bf33` | UNRUN | Raw29/30, R34 exceeds frozen 180 s; CUDA4/broker PASS | `8e3a9d32` for recorded broker; E1 `README.md`; E2 for R34. No new R34 lease authorized |
| TU106 | Linux 595.91.07 | `24906bf33` | UNRUN | Full raw/CUDA/broker acceptance UNRUN | E2; later exact revision not re-audited |
| TU106 | Windows 580.88 / 32.0.15.8088 | `24906bf33`, P19 | UNRUN | Short suite and 421 s zero-TDR hold PASS | E1 `README.md`, E2; hold revision not re-audited |
| TU106 | Windows second actual build | `24906bf33` | UNRUN | UNRUN | No build chosen or support inferred |
| GB206 | Linux 580.159.04 | `24906bf33` | UNRUN | Raw30/CUDA4/headless gfx10 PASS; desktop FAIL (later modes validate but frame black/flip timeout) | Headless `c2092cf7b`, tiling diagnostic `36d134bc5`; E1 `blackwell_5060/README.md` |
| GB206 | Linux 595.91.07 | `24906bf33` | UNRUN | Raw30/CUDA4/headless gfx10 PASS; broker UNRUN | `e1bd2df4d`; E1 `README.md` / `blackwell_5060/` |
| GB206 | Windows 580.88 / 32.0.15.8088 | `24906bf33`, P19 | UNRUN | Guest FAIL Code43 after bounded display/CMC probes; native short suite PASS | `c2092cf7b` and later diagnostics; E1 `blackwell_5060/README.md`; no current unload ordinal measured |
| GB206 | Windows 610.88 / 32.0.16.1088 | `24906bf33` | UNRUN | Native WU binding observed historically; independent guest acceptance UNRUN | E1 `blackwell_5060/README.md`; no public ABI assumption |
| GB205 | Linux 595.91.07 | `24906bf33` | UNRUN | Raw30 PASS with startup warnings; broader acceptance unverified | E1 `README.md`; exact source not re-audited |
| GB205 | Windows 580.88 | `24906bf33`, P19 planned | UNRUN | Native reference PASS; kayfabe guest UNRUN | E1 `README.md`; exact source not re-audited |
| Hopper GH100 | Linux / Windows builds unchosen | `24906bf33` | UNRUN | CPU FSP model/header tests only | No hardware or driver-installation claim |
| GB203 / GA102 / GA106 and other unlisted dies | Linux / Windows builds | `24906bf33` | UNRUN | No current-revision coverage claimed | Other campaign or provisioning history cannot establish acceptance here |

E1: historical evidence remains on `codex/sweep-blackwell-20261015` at
`24b6a4256ebdd4d806854696d417b93eddf34df4`, under
`traces/die_driver_sweep_20261015/`. It was not imported into this branch.
E2: owner-provided historical matrix in the 2026-10-16 steering. This report
does not upgrade its claims or substitute that summary for a new bench run.

## Windows P19 flags

CODE READ of historical receipts: every historical P19 result above required
the following exact set. Current AD102 and both GB202 Windows arms use this set. No
result is described as flagless. Future receipts must assert the actual process
environment, build, ABI and source. Display/GOP on, VGA none, except the measured
GB202 OFF arm (2026-10-20, `phase2_windows_gb202_56816449/b1_off/`): display off, GOP off, empty GOP ROM, directly verified by QOM.

```text
KF3_WIN_USER_CHANNELS_PASSTHROUGH=1
KF3_WIN_TWIN_DEFAPI_OBJECT=1
KF3_KERNEL_GR_CE=1
KF3_KERNEL_GR_WORK=1
KF3_TRANSLATED_CE_RELAY=1
KF3_DEFERRED_API=1
KF3_PREEMPT_BIND_PROBE=1
KF3_DISPLAY_IMP_ENABLE=1
KF3_DISPLAY_CTRL_PROBE=1
KF3_ZCULL_BIND_PROBE=1
KF3_SW_RUNLIST_HOST_OWNED=1
KF3_ASYNC_PREEMPT=1
KF3_GFX_POOL_PROBE=1
KF3_SW_RUNLIST_PROBE=1
KF3_MEMORY_LIST_PROBE=1
KF3_DISPLAY_SDR_COLOR=1
KF3_DISPLAY_CAPS_PROBE=1
KF3_DISPLAY_LUT_MIRROR=1
KF3_DISPLAY_ILUT_OFFSET_256=1
```

Public guest ABI for historical 580.88 is 580.65.06; for 581.51 it is
580.94.02. A native firmware version is a separately recorded fact.

MEASURED (lease 56808180, 2026-10-19/20) GA104580 R34 matched candidate-4 control0452613b5: PASS159s versus integrated e8553e6c timeout180s, n=1 each, fixed180s budget. Evidence phase2_ga104_56808180/c4_ce_control/. Subsequent Linux arms deferred for owner Windows priority.

Latest paired refresh, completed2026-10-20. Native595 gates9/9+gate10/USER12/raw30/CUDA4 PASS on both boxes before guests. Windows LTSC2024 is nested under kayfabe on Linux. Blackwell was subsequently reimaged with unmodified windows-on-vast for the separate native reference; no native result is a kayfabe Windows PASS.

| Refresh die (2026-10-20) | Guest actual build | Measured source / configuration | Result | Evidence |
|---|---|---|---|---|
| AD10256816419 | Windows580.88/32.0.15.8088 | `0c87acda8`, P19+RPC diagnostics, display/GOPon | PASS desktop/CUDA3/hardwareDX11+12/426.09s zero4101hold | `phase2_windows_ad102_56816419/windows580_pass/` |
| GB20256816449 | SameWindowsbuild | `0c87acda8`, P19+RPC diagnostics, OFF/OFF/emptyROM | FAIL verifiedCode43;357RPCs/unload356 precleanup;noNV0073/CA70/CA7A;0System4101 | `phase2_windows_gb202_56816449/b1_off/` |
| GB20256816449 | SameWindowsbuild | Same source+flags, ON/ON/signedROM | FAIL verifiedCode43;386RPCs/unload385 precleanup;CA70/CA7A present;0System4101 | `phase2_windows_gb202_56816449/b1_on/` |
| AD10256816419 | Linux580.159.04 | `0c87acda8`, native595 | raw30/30 + CUDA4/4 + gfx5/5steps,10/10hashesPASS; brokerPASS at7f22d18df on new56829120 | `phase2_windows_ad102_56816419/linux/` |
| AD10256816419 | Linux595.91.07 | `0c87acda8`, native595 | raw30/30 + CUDA4/4 + gfx5/5steps,10/10hashesPASS;1562raw-zeroHBwarnings; brokerPASS at7f22d18df on new56829120 | Same |
| GB20256816449 native | Windows580.88 / GSP580.65.05 | reference e8883e231 + tracer8f9bd4e51, productN/A | Code0+forcedGSP CUDA3PASS; final3804cmd/3863status fromseq0,strictdecoders0/0loss; firstpartial retained | `phase2_windows_gb202_56816449/native_trace/` |

Candidate NVDEC admission: draftPR53 aa8fdea9c, expectedRED340c12909/fixf71c1ce5e,1908releasePASS,5mutants each2killers,clippy0new; hardwareUNRUN. NativeCFB0 success supports a missing public policy row, not a complete Code43 cause. ON reports fourCA7A trace_result0allocations then tears down beforeCFB0; effective posted-result equality unavailable. TracerPR52 b9e246281 signed/loaded, complete capture uses existing nearby method4; added FMC runtime path NOTPROVED.

Ada broker source note: raw/CUDA/headlessgfx above measured0c87acda8 on56816419; broker595/580 measured7f22d18df on56829120. Product trees are equal. Both actualdriver/Cinnamon/Xorg/NVIDIAgears and nonblank1920x1080 screenshots pass;0guestXid/flip,host5→5,n1each. Exact referencebadf2d707da47a625e0ce69172a84785d20f81b8 throughNVPV_DIR; actualruntimebroker current0017914, unchangedprep verified. Evidence phase2_ada_broker_followup_20261020/RESULT.md. GPU-free Blackwell first-order/status/size/quiet-event comparison and3falsifiers: blackwell_diff_20261020/RESULT.md.

## Final bounded Blackwell follow-up56830105

| Die / OS / driver (2026-10-20) | Product revision | Result | Evidence |
|---|---|---|---|
| AD102 Linux595.91.07 broker, host595 | `7f22d18df` | PASS actualNVIDIA/Cinnamon/gears/nonblank1920x1080; guestXid/flip0,host5→5,n1 | phase2_ada_broker_followup_20261020/broker595/ |
| AD102 Linux580.159.04 broker, host595 | `7f22d18df` | PASS same criteria,n1; public pin verified/currentbroker | phase2_ada_broker_followup_20261020/broker580/ |
| GB202 nativeLinux595.91.07 | `7f22d18df` | PASS SOUND/gates9+10/USER12/raw30/CUDA4 | phase2_blackwell_probe_56830105/native/ |
| GB202 Windows580.88 OFF baseline | `7f22d18df`, P19 planned | UNRUN: disabled parent not ready by fixed04:04 latest-start; no product enable | phase2_blackwell_probe_56830105/probe_verdict.json |
| GB202 Windows580.88 OFF PR53 candidate | `aa8fdea9c`, P19 planned, unmerged | UNRUN: no unmatched candidate after paired baseline missed cutoff | Same; preparation/candidate_scope.json |

Only setup was measured on this lastlease (2026-10-20): genericLTSC check exit0, both binaries built, actual disabled8088/oem7/GSP1 parent ready04:06:09; no product init/arm markers/RPC records. Original harness cleanstop, evidencepushed before destroy/APIabsence04:06:46. H-NVDEC-OFF falsifier remains untested. No deadline extension, extra lease, private reply or PR merge.

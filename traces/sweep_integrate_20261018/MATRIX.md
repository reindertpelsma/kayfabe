# Sweep matrix on candidate-5 integration

STATUS: LIVE, 2026-10-19. AD102595 raw is FAIL29/30 (R19); native gates/raw/CUDA pass. Windows580 and Linux580 CUDA pass; Windows581 fails Code43. No family is complete.

MEASURED Phase1 accepted:14release crates3338/0/21,49mutants>=2kills,
no C4/C5test removed. Candidate5 f8f9d1a5f is an ancestor. Runtime product is
frozen24906bf33; subsequent commits are evidence-only and accepted product trees
remain equal. Every historical PASS is STALE. Own56765201 destroyed after
preflight; own56766467 AD102 destroyed/API absence20:44:59UTC before20:53deadline.
Actual current host open595.91.07; guest580/595 are separate axes and no current
host580 result is implied. Native AD102 gates9/9+gate10/USER12,raw30,Cuda4 PASS.
AD104 CPU oracles pass; strict native page equality RED100words. Fresh AD102
native read has99nonzero words and strict equality is also RED100words; neither
establishes a cause. Evidence under phase2_ad102_56766467/. Host remains off-limits.

| Die | Guest OS / actual driver build | Current source | Current result | Historical result, now STALE | Historical source / evidence |
|---|---|---|---|---|---|
| GB202 | Linux580.159.04 | `24906bf33`, host595 | FAIL: valid modes/Cinnamon/NVIDIA, black broker frame,1flip timeout/0guestXid | No current acceptance | phase2_blackwell_56788212/B2_RESULT.md |
| GB202 | Linux580.159.04 codec candidate | `45a4d91b5`, unmerged | FAIL: same black frame/1flip timeout; native9+10PASS; final B2 attempt | Separate public-format topic, PR51 | phase2_blackwell_56788212/b2_codec_candidate/ |
| GB202 | Linux595.91.07 | `24906bf33`, host595 | PARTIAL: concurrency PASS41s;31zero-heartbeat startup warnings; fullsuiteUNRUN | No current acceptance | phase2_blackwell_56788212/B3_RESULT.md |
| GB202 | Windows580.88 | `24906bf33`, P19 | UNRUN: sealed LTSC ready; binding-only preparation active | No current acceptance | phase2_blackwell_56788212/ |
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
| GA104 | Linux 580.159.04 | `24906bf33` | UNRUN | Raw30/CUDA4/gfx10/broker PASS | `ba864a55`; E1 `README.md` / `ga104_3070/` |
| GA104 | Linux 595.91.07 | `24906bf33` | UNRUN | Native gates9/9+10; guest raw30/CUDA4/gfx10/broker PASS; raw heartbeat warnings and 27 STALL/9 nvAssert | `9d6f210ba`; E1 `ga104_595_completion_20261017/FINAL.md` |
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
the following exact set. Current AD102 Windows results use this set; GB202 preparation is active. No
result is described as flagless. Future receipts must assert the actual process
environment, build, ABI and source. Display/GOP on, VGA none.

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

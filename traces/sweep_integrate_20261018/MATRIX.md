# Sweep matrix on candidate-5 integration

STATUS: LIVE, 2026-10-19. Current hardware is UNRUN; no family is complete.

MEASURED GPU-free checkpoint: code/tests imported on candidate5 f8f9d1a5f;
current checkpoint7326d3184, with heartbeat held and the exact scrub-test
fixture decision pending. This is not Phase1 acceptance. Every old hardware
PASS remains STALE. No rental has been created in this stretch.
AD104 CPU oracles pass; strict page equality fails100words, documented in
AD104_PATH_AUDIT.md. The trusted host remains off-limits.

| Die | Guest OS / actual driver build | Current source | Current result | Historical result, now STALE | Historical source / evidence |
|---|---|---|---|---|---|
| AD102 | Linux 580.159.04 | `7326d3184` | UNRUN | Raw30/CUDA4/gfx10/broker PASS | `da0569a6`; E1 `ad102_4090/README.md` |
| AD102 | Linux 590.48.01 | `7326d3184` | UNRUN | Raw30/CUDA4/gfx10/broker PASS; 18 STALL/10 nvAssert | `9d6f210ba`; E1 `ad102_second_builds_20261016/linux590_apps/RESULT.md` |
| AD102 | Linux 595.91.07 | `7326d3184` | UNRUN | Raw30/CUDA4/gfx10/broker PASS across recorded revisions; warnings retained | Raw latest `9d6f210ba`, broker `6265b980f`; E1 `ad102_followup_20261016/RESULT.md` and `ad102_second_builds_20261016/linux595_regression/RESULT.md` |
| AD102 | Windows 580.88 / 32.0.15.8088 | `7326d3184`, P19 | UNRUN | Code0, desktop, CUDA3, hardware DX11/12, 421 s zero-TDR hold; Blender/Edge short PASS | Hold/apps `77a72da9e`; E1 `ad102_followup_20261016/RESULT.md` |
| AD102 | Windows 581.51 / 32.0.15.8151 | `7326d3184`, P19 | UNRUN | Guest FAIL Code43; native same-board suite/hold PASS | `9d6f210ba`; E1 `ad102_second_builds_20261016/{windows58151,native_windows58151}/RESULT.md` |
| AD103 | Linux 580 / 595 | `7326d3184` | UNRUN | No sweep acceptance claimed | Not re-audited |
| AD103 | Windows 580.88 | `7326d3184`, P19 planned | UNRUN | Older captured-page desktop result is STALE; derived-page cell unverified | E2; old exact revision not re-audited |
| AD104 | Linux / Windows | `7326d3184` | UNRUN | Owner-host history is STALE; host access forbidden to this lane | No hardware claim; CPU oracle PASS, strict stored-native-page equality FAIL100words; AD104_PATH_AUDIT.md |
| AD106 | Linux 580 / 595 | `7326d3184` | UNRUN | No current-product sweep acceptance claimed | Not re-audited |
| AD106 | Windows 580.88 | `7326d3184`, P19 planned | UNRUN | Older captured-page desktop result is STALE; derived-page cell unverified | E2; old exact revision not re-audited |
| GA104 | Linux 580.159.04 | `7326d3184` | UNRUN | Raw30/CUDA4/gfx10/broker PASS | `ba864a55`; E1 `README.md` / `ga104_3070/` |
| GA104 | Linux 595.91.07 | `7326d3184` | UNRUN | Native gates9/9+10; guest raw30/CUDA4/gfx10/broker PASS; raw heartbeat warnings and 27 STALL/9 nvAssert | `9d6f210ba`; E1 `ga104_595_completion_20261017/FINAL.md` |
| GA104 | Windows 580.88 / intended 32.0.15.8088 | `7326d3184`, P19 | UNRUN | Binding UNVERIFIED; stored BasicDisplay 10.0.26100.1; applications/hold UNRUN | `9d6f210ba`; E1 same FINAL. Next prerequisite is verified native/VFIO install using unmodified windows-on-vast |
| TU106 | Linux 580.159.04 | `7326d3184` | UNRUN | Raw29/30, R34 exceeds frozen 180 s; CUDA4/broker PASS | `8e3a9d32` for recorded broker; E1 `README.md`; E2 for R34. No new R34 lease authorized |
| TU106 | Linux 595.91.07 | `7326d3184` | UNRUN | Full raw/CUDA/broker acceptance UNRUN | E2; later exact revision not re-audited |
| TU106 | Windows 580.88 / 32.0.15.8088 | `7326d3184`, P19 | UNRUN | Short suite and 421 s zero-TDR hold PASS | E1 `README.md`, E2; hold revision not re-audited |
| TU106 | Windows second actual build | `7326d3184` | UNRUN | UNRUN | No build chosen or support inferred |
| GB206 | Linux 580.159.04 | `7326d3184` | UNRUN | Raw30/CUDA4/headless gfx10 PASS; desktop FAIL (later modes validate but frame black/flip timeout) | Headless `c2092cf7b`, tiling diagnostic `36d134bc5`; E1 `blackwell_5060/README.md` |
| GB206 | Linux 595.91.07 | `7326d3184` | UNRUN | Raw30/CUDA4/headless gfx10 PASS; broker UNRUN | `e1bd2df4d`; E1 `README.md` / `blackwell_5060/` |
| GB206 | Windows 580.88 / 32.0.15.8088 | `7326d3184`, P19 | UNRUN | Guest FAIL Code43 after bounded display/CMC probes; native short suite PASS | `c2092cf7b` and later diagnostics; E1 `blackwell_5060/README.md`; no current unload ordinal measured |
| GB206 | Windows 610.88 / 32.0.16.1088 | `7326d3184` | UNRUN | Native WU binding observed historically; independent guest acceptance UNRUN | E1 `blackwell_5060/README.md`; no public ABI assumption |
| GB205 | Linux 595.91.07 | `7326d3184` | UNRUN | Raw30 PASS with startup warnings; broader acceptance unverified | E1 `README.md`; exact source not re-audited |
| GB205 | Windows 580.88 | `7326d3184`, P19 planned | UNRUN | Native reference PASS; kayfabe guest UNRUN | E1 `README.md`; exact source not re-audited |
| Hopper GH100 | Linux / Windows builds unchosen | `7326d3184` | UNRUN | CPU FSP model/header tests only | No hardware or driver-installation claim |
| GB202 / GB203 / GA102 / GA106 and other unlisted dies | Linux / Windows builds | `7326d3184` | UNRUN | No current-revision coverage claimed | Other campaign or provisioning history cannot establish acceptance here |

E1: historical evidence remains on `codex/sweep-blackwell-20261015` at
`24b6a4256ebdd4d806854696d417b93eddf34df4`, under
`traces/die_driver_sweep_20261015/`. It was not imported into this branch.
E2: owner-provided historical matrix in the 2026-10-16 steering. This report
does not upgrade its claims or substitute that summary for a new bench run.

## Windows P19 flags

CODE READ of historical receipts: every historical P19 result above required
the following exact set. No Windows run occurred in the current lane and no
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

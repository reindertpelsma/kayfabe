# AD102 Windows580 reference and three single-row ablations

**STATUS: LIVE, 2026-10-16 23:00UTC. All three ablations complete: caps00c FAIL desktop; each LPWR removal PASS.**

Only own56302930, actualAD102/PCI2684, host595.91.07. All four intended tests are fresh
siblings of stopped bound580-verifiedpayload, with identical parent/VARS/UUID/media-free
profile. ActualWindowsEnterpriseSEval build26100 UBR1742 (LTSC evaluation),580.88/
32.0.15.8088, classEnableGpuFirmware1. Acceptance uses the19printed owner flags (P19),
no RPC/body tracing. Exact actualenvironment/argv/binarySHA/source are in per-phase receipts.

| phase | source | adapter/desktop | apps | pixels/TDR/hostXid | result |
|---|---|---|---|---|---|
| reference580-v2 |9d6f210ba1dc6ab0edbc49ff265101091e8b08f1|Code0/DWM1/Explorer1|CUDA3 and hardwareDX11/12 before+after PASS|1920x1080/78753colors,0TDR,host5→5|PASS short; suite14.656s, host invocation56s|
| ablate-caps00c-v2 |a52494d487e2ff3068cbae60e3ff1fe422ff281d|Code0/GSP1 but DWM0/Explorer0|UNRUN: desktop precondition failed|UNRUN; safe stop at22:54:18|FAIL acceptance|
| ablate-lpwr2801-v2 |a7509bf3d33a6986874beae0d03c95e6ef0efe19|Code0/GSP1/DWM1/Explorer1|CUDA3/DX11+12 beforeafter PASS|1920x1080/78756colors,0TDR,host5→5|PASS short/28.905s; stop22:56:37|
| ablate-lpwr2806-v2 |003ad0e749eede980723cc64b9107a1d747b9cc7|Code0/GSP1/DWM1/Explorer1|CUDA3/DX11+12 beforeafter PASS|1920x1080/78755colors,0TDR,host5→5|PASS short/25.145s; stop22:58:57|

Each diagnostic branch removes exactly one AD102580 row (0insertions/1deletion),
never a full caps page. Each private binary had exact native9+gate10/12USER gates
before testing. H-required falsifier, recorded before builds/runs: if removal still
passes Code0/desktop/CUDA/D3D, that individual row is not required for this exactbuild
and P19profile. A failed removal does not prove necessity on any otherdie/build;
one-at-time success does not establish that joint removal is safe.

Earlier CD and shell-filename staging attempts are explicitly excluded: no app could
run with missing probes. Corrected helper stages with Python argv/PureWindowsPath,
and the strict verifier and acceptance both check filepresence/hash. This baseline
contains verified files before its parent is frozen. Earlier Ada seven-minutehold/Blender/
Edge results remain separate; this reference does not repeat or claim a seven-minutehold.

Capture pixelSHA a4fa3d970da377af787d20e0f0434ea144c676d080d09fef553c7450c7e40df4;
raw frame remains on rental. Only screened text exported; CRLF/trailingwhitespace and
identifiers normalized, manifest hashes/sizes refer to raw files. Final ownedstop exit0
at22:49:00UTC. No familycomplete. SecondWindows581.51 and Linux590 stillUNRUN.

MEASURED caps00c: absent-row notice at realize, QGA answered after170s versus60s reference;
actual8088/GSP1/Code0 but DWM0/Explorer0 at census and again at acceptance. The suite
stopped on its desktop precondition before any CUDA/D3D probe. No blank-frame or TDR
claim is inferred from this early exit. Same source diff is one deletedrow; this bounded
result supports that row for this profile, but does not establish anotherdie/build's need.
Falsifier to required-row did not fire. Ownedstop used bounded shutdown/ACPI/TERM, noKILL.

MEASURED LPWR removals: both individually pass the same short suite and nonblankframe/
zero-TDR/zero-new-host-Xid checks. Each respective unsupported control is actually requested:
its QEMU cumulative refusal census contains fn76/0x20802801=0x56x1 or
fn76/0x20802806=0x56x1. Repeated census lines are not extra requests. The falsifier to
individual necessity FIRED for each LPWR row on this build/profile. These experiments
neither establish safe joint removal nor justify removing rows for other builds/dies.
No product row is removed; diagnostic branches remain unmergeable.

Exact P19 profile, every value1, actual process receipt verified for each run:

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
WIN_KF3_DISPLAY=display=on,gop=on
WIN_STDVGA=0
WIN_GUEST_DRIVER=580.65.06
```

The source receipt records the actual private binary and argv; there are no diagnostic
RPC flags in these runs. Screenshots remain on rental; exported pixel summaries contain
only dimensions, color count and SHA. The original175minute ownership timer remains armed.

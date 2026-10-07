# Status and handoff — where kayfabe v3 stands, and how to resume

**STATUS: LIVE, 2026-10-07 — §0.0 below is the resume point.** ⊘ *The 2026-09-30 paragraph that follows is history:* **(2026-09-30)** Master = the code of **`afb552ea`** (`v3-mc23`: CUDA dynamic parallelism +
the guest-RAM-object race fix + everything below), which passed the full merge bar (§0 first entry), plus
evidence and documentation. The single entry point for resuming work without any chat history. Decisions
live in `docs/OWNER_RULINGS.md` (doorbell refinements of 2026-09-30 in §D); per-topic detail in the design
docs named below. ⊘ When this file and a design doc disagree, the design doc's dated STATUS wins — then
fix this file. Entries below the first are dated history.

## 0. Current resumption — start here

### 0.0 ★ RESUME HERE — 2026-10-07

**StartDevice batch, 2026-10-07 (corrects the next paragraph).** The VFIO
GSP actually answered `rcEnable=ENABLED`, and Windows then sent
`SET_RC_RECOVERY(ENABLED)`. The earlier decode read the reserved word at offset 36,
not `params` at offset 40. The batch (`kf_rm::vfguest`, the class-0x78 edge,
NV0073 EVENT_SET_NOTIFICATION, PERF_GET_POWERSTATE) answers as the `_VF` vGPU-guest
HAL does: GET reports DISABLED, SET(DISABLED) is accepted and SET(ENABLED) is refused.
Run25 at 3b436488 (gates 9/9, host healthy, no Xid): Code43 and smi exit 9 are
verified again. GET is served, and the abort moved by one RPC to
`SET_RC_RECOVERY` (0x56). Windows sends ENABLED even after GET reports DISABLED.
**Owner decision pending:** accept ENABLED with no effect (`_VF` HAL), report
ENABLED as an inferred host fact, or implement per-VM RC recovery. The class-0x78
and NV0073 event entries that follow are already served. DIAGNOSTIC run26 at
8208effe (default-off `KF3_RC_RECOVERY_ENABLED_DIAG`) accepts ENABLED and aborts
at VFIO index 2518: the first `0x20800301` arming (notifier 44) is refused. Code43
persists. The next wall is the 23 notifier indices Windows arms, ranked in the
traces README.
Owner ruling §S (2026-10-07, `OWNER_RULINGS.md`) is applied at 4448be53: RC recovery is a
stub (ENABLED), the notifier family is classified per index, and hotplug is real for Windows.
Run27 still shows Code43. The abort is at VFIO index 2524, the arming of 120
HOTPLUG_PROCESSING_COMPLETE, which has no OGKM producer. That needs an owner decision or
semantics, followed by real-host-event notifiers (PSTATE, CE, GRAPHICS, runlist preempt).
VFIO event census (no boot): the real GSP posts only 33 PSTATE_CHANGE, 34/45 (HDCP/audio
for display 0x200) and 139 RUNLIST_PREEMPT_COMPLETE. Every other armed index, 120 and 122
included, is never posted and is now accepted silently. 33 and 139 (armed at 2528 and 2530)
are owner decision items; run28 is on hold. [Batch and run25](../traces/windows_code43_walls_20261007/README.md#sixth-repair-startdevice-batch-after-get_rc_recovery).

**Code43 cause analysis, 2026-10-07 (corrects the paragraph below).** Run24 at
a6f84d0d births and promotes OFA0. Code43/smi9 is verified and the 24 assertions
are unchanged. Reading the unchanged list as "the walls are not the cause" is
wrong. The RM assert journal is a 4 KiB buffer that holds exactly 24 records
(OGKM `journal.c`, `rmcd.h`), so it keeps only the first asserts of the boot.
In runs 13-24, the driver's Free-teardown begins right after the first
StartDevice refusal it does not tolerate. That RPC was C56F, then 5080, then
GPU_PROMOTE_CTX, and is now GET_RC_RECOVERY 0x2080220e. The livedump is 0x1B0
"Start device failed" with STATUS_INSUFFICIENT_RESOURCES, whatever the RM status.
Three Code43-free VFIO boots return rcEnable=DISABLED at the same RPC index
(2515), then continue. A VFIO-derived forecast lists 98 later controls/classes
that kayfabe has never served. Next: VM-scoped RC_RECOVERY, then run25 to test
the prediction. [Analysis](../traces/windows_code43_walls_20261007/README.md#abort-point-analysis-against-the-vfio-reference).

**Active Code43 iteration, 2026-10-07:** owner requests continued repair one wall
at a time. The first change resolves the declared Device-shared default VA for
COPY2; bounded same-GPU relationships and revocation tests pass (611 kf-rm tests).
Run16 at c30ee910 verifies COPY2 birth on the canonical shared VA with a real
USER host channel. The next allocation, class5080 deferred API, refuses; Windows
retires COPY2 with zero submissions. Code43/smi9 persist. FBMEM registration and
FIFO latency query also still refuse. No initialization success claim.
[Iteration record](../traces/windows_code43_walls_20261007/README.md).
The second change adds checked FBMEM registration for audited580.65.06 and
580.159.04 guest contracts, scoped to the VM's usable FB heap; focused18 tests
and full612 kf-rm tests pass. Run17 at5348fccf verifies the captured class82
FBMEM request returns0; COPY2 births but class5080 still refuses, no submissions.
Code43/smi9 persist. Next: Deferred API software-object constructor and its
queue-triggered control protocol; do not fake its execution or completion.
The source-backed constructor is now implemented with optional notification
policy and channel-parent lifetime checks;1172 ABI/RM tests pass. Run18 at
c34d8dac returns0 for both class5080 constructors, then engine allocation and
group scheduling succeed. Newly reached a06c0103 SET_TIMESLICE refuses because
the handler only resolves passthrough group members. Next: apply the authored
host control to Translated members off the GSP lock. This handler change is now
implemented;101 kf-qemu tests pass, zero new Clippy debt. Run19 at f6aa9d2e
applies4000us on the actual owned host group; a06c0103 returns0 and watchdog
hint e38498 disappears. Code43/smi9 persist. Next: kernel GR channels are
acknowledged without birth, then GPU_PROMOTE_CTX refuses. Capture exact inline
promotion entries before implementing a real owned unprivileged GR context.
The default-off observer is implemented with16 records of at most16 entries;
615 kf-rm tests pass, zero new Clippy debt. Run20 at bf909e1a captures both
nine-entry requests: main/patch/FECS/maps initialize, four entries only carry VA.
The payload decodes; the missing real kernel-GR twin refuses promotion. Next:
owned unprivileged GR context and actual Translated CE work on that runlist,
with bare-metal validation before another guest test. Native b7ef3e56 passes
all gate3 copy/split/GPU-completion checks on a real GR context. Kernel-GR
admission is now implemented behind KF3_KERNEL_GR_CE plus TSPACE, with promotion
requiring the actual owned context. Compatible CE-class native arm and Windows
run21 at3fd6fc39 births/promotes both kernel-GR channels and executes two real
RM-scrubber GPU submissions with GP_GET2. Code43 persists. Next missing birth:
kernel NVDEC0/engine13 channel ff040003; legacy Falcon-context promotion
(VA1203cd000 +4KiB, entryCount0) returns0x56. The newly reached0080170f channel-properties control
also refuses. Native7152d1a8 now constructs a real USER NVDEC0/C9B0 context,
completes its GPU fence and releases all resources. Guestb52da0c7 adds default-off
KF3_KERNEL_NVDEC_CTX with TSPACE and requires actual context ownership;101 QEMU
tests pass, zero new Clippy debt. Run22 atb52da0c7 births/promotes NVDEC0;
seven Translated births, two real GPU submissions, Code43/smi9 persist. Next:
NVENC1/engine1c clientc1d0001a/ff040005 missing birth and Falcon promotion
returns0x56. The property observer decodes0080170f as COPY2 engine timeslice
4000us; that independent setter still refuses. Native e465d356 NVENC1 USER context/fence/release passes; guest e462194d
adds default-off KF3_KERNEL_NVENC_CTX plus TSPACE.101 QEMU tests pass, zero new
Clippy debt. Run23 at e462194d births/promotes both NVENC1 channels, nine
Translated births, same two real GPU submissions. Code43/smi9 persist. Next:
OFA0/engine33 clientc1d0001c/ff040007 has no birth and Falcon promotion refuses.
Native ff12e6a7 OFA0 USER context/fence/release passes; default-off
KF3_KERNEL_OFA_CTX plus TSPACE implemented,101 QEMU tests pass, zero new Clippy
and claims debt. Windows run24 pending. A second4000us channel-timeslice setter also refuses. NVENC0 native bind refused, and is not
substituted for the requested NVENC1 instance. Arbitrary kernel GR/software/codec execution remains unsupported;
queued Deferred API controls/methods remain unsupported; no completion is forged.


**Windows allocation investigation, 2026-10-07:** bounded diagnostic run14 at
`b928ab4c` captures the complete single-inline memory descriptor: class82
FBMEM registration, flags48040200, direct PFN0xea6e0, 128KiB. Its shape is not
malformed; our SYSRAM-only decoder rejects class/location/GPU-cacheability.
Run15 at `b6336577` captures the failing COPY2 channel Device share explicitly:
client c1d00012 shares c1d00002's Device default, vaMode0; our channel resolver
drops that relation and returns INVALID_STATE before any host channel creation.
The adjacent 0080170e query is FIFO latency-buffer sizing, also unsupported.
Code43 persists; run14 retains the same24 saved assertions as run13. This is
source-backed diagnosis, not a functional repair or proof tying either RPC to
assertion1a103e6. [Exact decodings/evidence](../traces/windows_exact_decode_20261007/README.md).

**Latest checkpoint, 2026-10-06:** product `74cd590c` implements GPU TMO and
indexed CSC tables under the existing opt-in colour flag. The strict Linux
TMO run C passes: retained curve, accepted atomic requests, nonzero armed tone
buffer, black transformed output and exact restoration. SDR/TMO GPU fixtures,
9/9 GPU gates and 11/11 USER births pass; no promotion or full HDR parity claim.
[Source scope](design/V3_TMO_COLOR.md), [evidence](../traces/tmo_stage_20261006/README.md).
The owner clarified that broader table formats and chroma-correction controls
must also gain real processing; this checkpoint is not the intended endpoint.
Windows run13 at `74cd590c` still reports Code43, but reaches 8,330 display
methods (zero UPDATEs/scanouts) rather than zero methods. Its fresh watchdog
changes after the same first 17 assertions: `0x169d836` is replaced by
`0x1a103e6`; no interpretation as an NV_STATUS or completed initialization.
Product `8dde9b51` additionally executes variable tone segments and 65..1025
sample tables; GPU fixtures pass, including exact compact extents and hostile
header rejection. Product `00220cc6` adds real DIRECT8 input/output lookup; all eight SDR GPU
fixtures pass on the borrowed host; strict Linux run E passes again.
Broader segmented input/output and chroma policies
still need implementation and independent semantics/oracles.
The identified absent-TMO fallback lives in NVKMS,
whose build documents Unix sharing; its Windows use is not established.

The earlier SDR product `2aa8b92d` and missing-capability test follow:

**Latest owner-directed implementation:** branch `codex/sdr-lut-20261006`,
product `2aa8b92de6c6ab158f9bc8e788be792408a074a3`, real bounded GPU SDR
ILUT/OLUT processing under `KF3_DISPLAY_SDR_COLOR=1` (default off).
Linux open580.159.04 on borrowed RTX 4070 runs Weston and Sway. A 2501K
KMS curve changes output with **zero error in all 6,220,800 RGB components**;
restoration is byte-identical, with no compositor gamma failure. Six GPU colour
fixtures and 9/9 GPU gates (11/11 USER births) pass. Workspace tests, formatting,
zero-new-debt Clippy and product CI pass. Matched open580.159.04 fast guest
passes 30/30; the frozen native grader refuses host595.91.07 before running arms,
so this is not the full hardware merge bar. No master promotion/full app claim.
[Implementation and limitations](design/V3_SDR_COLOR.md),
[evidence](../traces/sdr_lut_20261006/README.md).

**Owner-directed strict TMO test (2026-10-06): FAIL.** The corrected Linux
harness `8316cb61` runs against unchanged product `2aa8b92d`. The active primary
plane exposes no `NV_PLANE_TMO_LUT`; the guest adapter rejects three TEST_ONLY
requests before kernel submission. Zero real TMO commits/bindings/control words;
all three console captures are identical despite a ready compositor callback.
The strict checker returns 1; VM exit is 0, restoration is byte-exact and host
GNOME/GPU remain healthy. An unprivileged read-only native control exposes the
property on host595.91.07, but no native TMO operation was performed and the
driver/kernel versions differ. [Evidence and limits](../traces/tmo_stage_20261006/README.md).
This catches the skipped stage the owner wanted ruled out; the broader positive
TMO objective remains unmet. Next, derive the actual ICtCp/TMO capability and
implement bounded GPU TMO before advertising it or retesting Windows. Ordinary
SDR rendering and working gamma do not establish a TMO buffer.

Windows run 12 with implemented DIRECT10 surface-loading declarations still
has Code 43 / smi exit 9 and zero display methods. Its fresh dump reaches
`0x169d836`: the display-buffer precondition path, statically pointing to a
zero-sized TMO descriptor with TMO absent (no live size captured). Do not
reintroduce a fake TMO capability. Next Windows work needs a supported path
avoiding TMO or real bounded TMO processing, with further walls possible.
Complete dump records remain usable; outer NVCD is one byte short.
Borrowed-host source/target/evidence: `/var/lib/kf-sdr-lut-20261006/`;
controller private evidence: `/data/kayfabe-runtime/sdr-lut-20261006/`.
All code/evidence checkpoints are committed and pushed. Master/v3 stay
`906a76a4`; all owned VMs exited, NBD disconnected, host GPU healthy, no rental created.

**Historical Linux diagnostic (superseded by real SDR implementation above):**

**Latest owner-directed test:** `codex/linux-color-oracle-20261006`, diagnostic
product `8bbcd7f3`, follows the Antigravity repair. Linux guest open580.159.04
on the borrowed AD104 runs Weston13 and Sway1.9 with NVIDIA rendering. The
fixed SDR image reaches the console pixel-exact and matches the native client's
RGBA hash. OGKM binds identity input/output LUTs during ordinary composition.
A 2501K KMS gamma blob is accepted but ignored by the virtual output; all three
screenshots are identical. A separate guest-only KMS refusal makes the gamma
client fail while Sway preserves the image, without a shader replacement.
Evidence: [Linux color audit](../traces/linux_color_20261006/README.md).

The next Windows investigation should identify its actual methods and the
smallest correct identity/disabled SDR subset; full arbitrary LUT support is
not yet established as a startup requirement. Unsupported actions must remain
absent/refused under §H, never successful no-ops. M/N/O remain construction-only
and off by default. No Windows success or master promotion. No VM remains
running, host GPU healthy, no rental created.

The earlier Antigravity repair and Windows constructor result follow:

The owner resumed work and requested review/repair of Antigravity's changes.
Branch `codex/antigravity-review-20261006`, product repair `e294fbe1`, source
`4cb9e609` after comment/CI corrections, removes fabricated Windows control
success, restores isolated default-off LUT probes and removes the incomplete
CPU-reference-only ILUT path. Both original histories are backed up on GitHub.
The generated method vocabulary is retained and reproduced from OGKM.

Validation so far: 2,262 v3 tests at `e294fbe1`, Clippy with zero new warnings,
format check, all 14 fast CI gates, and exact `4cb9e609` real GPU gates 9/9
(11/11 USER births) plus the Rust/C QEMU 10.2.4 rebuild on the borrowed RTX 4070.
No promotion or full merge-bar/application claim. Master/v3 remain `906a76a4`.
See [review and evidence](../traces/antigravity_review_20261006/README.md).

The controlled O Windows run `boundary-kayfabe-10` completed: Code 43 /
`nvidia-smi` exit 9 remains, but 455 RPCs (N: 365) now reach the probe's explicit
display-method refusals. The fresh dump's first changed assertion moved from
`0x16e9967` to `0x1a103e6`; the first 17 of 24 complete saved assertions match N.
Outer NVCD remains one byte short, so its checksum cannot be validated. The guest
shut down cleanly, supervisor exit zero, RTX 4070 healthy, no VM or NBD attachment
remains. Sanitized evidence is in the review directory; raw private controller
evidence is under `/data/kayfabe-runtime/windows-boundary-20261005/`.
Next (refined by the Linux result above): review the production LUT/color plan
and establish the supported subset with completion/lifetime tests. Constructor
declarations alone are insufficient.
No rentals were created. Preserve overlays until fresh evidence is durable.
The previous session's Windows priority and constructor analysis are recorded in
`codex/windows-boundary-comparison-20261005` at `fba14de6`, including
`docs/handoff/2026-10-05-WINDOWS-PAUSE.md`; the owner's resume revokes that pause.

### 0.0 Historical resumption — 2026-10-05 (superseded by the 2026-10-06 entry above)

**Experimental integration, not promoted:** `codex/p1p2-integration-2026-10-05`,
worktree `/tmp/kayfabe-p1p2-integration-20261005`, combines master `906a76a4`,
the complete P1/P2 branch `31b64802` (merge `a5a350a8`), and Windows branch
`c1d4e415`. **KF3 ABI 19** retains all ABI-18 display/broker fields and appends
the signed-GOP path to `kf3_realize`. P0's USER-channel birth guard is included.
`KF3_TSPACE` and `KF3_GFX_POOL_PROBE` both stay off by default. No hardware or
Windows success is claimed for this integration; master remains the verified
candidate-2 source described below.

Before Windows was added, the integrated P1/P2 source passed 919 GPU-free tests
(ABI/chip/host/mem/chan/qemu/harness), 18 Python instrument tests, and the full
unsafe-containment gate. The combined driver matrix was compared with the full
30-tag compiler sweep: 129,240 cells, zero differences, including 595.91.07 and
the display-SW allocation structure. These are source/logic checks, not a GPU
bar. Final combined validation is recorded in
`traces/windows_p1p2_integration_20261005/README.md`.

Next: incorporate the separately audited real timer mapping, rebuild the C and
Rust device together against QEMU 10.2.4, then run exact-revision GPU gates and
Linux regression lanes serially. P1/P2 additionally needs the window-reach
positive/negative control and Translated-ring/physical-CE probes (still unwritten
at `31b64802`), family coverage, unload/re-init, and A/B application checks before
changing its default. `V3_P1P2_TSPACE.md` has the exact requirements. No claim of
hostile-guest isolation may omit the remaining audit findings.

The narrow Windows A/B/C evidence remains on its original branch/source:
QUERY_SIZE success moved startup beyond the old barrier; timer allocation alone
did not clear Code 43. Those results do not validate this newer integration.
The borrowed PC and Vast rentals remain controlled by the root agent; this
integration task runs no hardware jobs and creates no rentals.

**Current verified integration (2026-10-04, candidate 2):**
`codex/candidate-2-2026-10-04`, local worktree `/data/kayfabe-candidate2-20261004`,
combines promoted candidate 1 with broker/maxfps, **KF3 ABI 18**. Product revision
`9d82f259` includes the HMP AioContext fix and asynchronous console cursor update.
All later commits are docs/evidence/recipes, with no product/build/test-source delta.

- Exact-source `cand2b`: **2131/0 tests, gates 9/9, bare and guest 30/30**, USER
  birth census. Product CI, including the full slow dispatch, passes.
- Apps: **host 71/71; guest 61/65 ordinary apps + 6/6 probes**, zero verdict
  changes from candidate 1, including four failures rerun alone. All three
  output digests match host and candidate 1. The same managed-memory limits
  remain (eight host Xid 31 records; conjugateGradientUM's wrong-answer success
  exit is correctly graded FAIL by the matrix).
- HMP/QMP fresh-frame checks pass, including the box where the old binary hung.
  At `9d82f259`, cap-30 KMS/Wayland/X11, forced-60-raster clamp, measured Vulkan FIFO 29.976857
  FPS, async-60 pacing, D2 idle/unwatched behavior, broker GPU-copy, cursor parity,
  reconnect and rendering with a stopped broker all have committed evidence.
- Refresh-only hints reach actual guest EDID at 30/50/60 Hz on userspace
  connector reprobe. No automatic DRM uevent/desktop mode switch or physical
  monitor transition is claimed. Baseline Xorg-start warnings and darker guest
  cursor edges remain documented; actual DDX import refusal was not provoked.
  `x11-dispsw` stays **off by default** pending §N's security conditions.

Evidence and full limitations: `traces/v3_candidates/cand2_20261004/README.md`.
Under §R, promote master/v3 only after CI passes on the final docs/evidence head;
check the remote refs for the exact promotion commit. The earlier candidate 1
promotion was `9b50d295`. Next integration work remains the P1/P2 and security
lanes in the table below; Windows is parked behind those priorities.

Owned display box **54137212** was destroyed after all its evidence was pushed,
and absence was verified. Existing **54049598** (vmb) remains; its candidate-2
jobs are finished. Do not destroy another session's box. nvkvm-pv dependency
`codex/broker-refresh-2026-10-04` at `9f2fd00` passes its own broker tests/CI but
was **not** promoted to that project's default branch or VMM hardware bar.
No code loss is known. The lost old-box display results were regenerated and
saved after each batch; retain that practice. The original dirty workspace on
`codex/ci-repair-2026-09-29` was preserved separately throughout.

**Candidate 1 recovery record (completed):**

**Verified integration = `0ac157b2` (+ docs/evidence).** Candidate 1's post-B5-fix hardware jobs
finished after the earlier handoff; their results were recovered on 2026-10-04 in
`traces/v3_candidates/cand1_20261004/`. Master was `529de32c` at recovery start.
This branch combines the tested candidate with those newer master docs; no product or test code
changed during recovery. Code reaches master only through a **candidate branch** tested as a
whole on a real GPU (rules below). Branch heads change; always
`git fetch` and read each lane's own design doc STATUS. Workflow results that lived only in a session
are committed in `docs/handoff/2026-10-04/` (README there).

**Rules that govern everything (read first):** `docs/OWNER_RULINGS.md`
- §Q, the address model: passthrough spaces hold only guest PTE/PDB rows; Translated spaces hold
  guest VRAM and/or guest RAM; a channel the guest made unprivileged never shares a space with a
  window or kayfabe memory.
- §R, memory safety: the `_unsafe` file is the audit perimeter, and validation sits at the boundary,
  never at call sites. The proposals and gates are adopted. The **standing merge approval**: merge
  without asking iff CI is green, the code is reviewed, and the EXACT commit passed on a real GPU
  box a real guest boot, the merge bar and the apps.
- §K: Secure Boot uses a self-signed ROM with a per-install key. The vTPM state is a persistent
  secret. Bench Windows runs without BitLocker.
- §M: display-max-fps decisions D1-D5. §N: x11-dispsw. §O: cursor. §P: security audit.

**Lanes (GitHub branches; each is the only source of truth for its work):**

| lane | branch @ last known head | state | next step |
|---|---|---|---|
| **Candidate 1** | `v3-cand-1` tested at `0ac157b2`; recovery `codex/resume-2026-10-04` adds master docs + evidence (KF3 ABI 13) | B5 fixed at `c1ca7945`; full hardware bar passed on unchanged retry: 1937/0, 9/9, bare 30/30, guest 30/30, USER births. Apps host 71/71, guest 61/65+6/6 = baseline; B0/B1, B5 **4×14/14**, X11 A/B pass. First-attempt churn-test failure and its fake-token collision diagnosis retained. | Promoted at `9b50d295`; candidate 2 now incorporates it. Evidence: `traces/v3_candidates/cand1_20261004/` |
| Display: broker | integrated in candidate 2 at `9d82f259` (ABI 18) | Post-review union passes GPU-copy/cursor/reconnect/stopped-broker checks; evidence in candidate 2 | Promote the candidate as a whole after final-head CI; companion nvkvm-pv default stays separate |
| Display: max fps | integrated in candidate 2 at `9d82f259` (ABI 18) | Hardware checks complete, including actual X11 FIFO present cadence and D4 correction folded into §M | Keep x11-dispsw default off until §N security conditions hold |
| nvkvm-pv broker | `codex/broker-refresh-2026-10-04` @ `9f2fd00`, based on block-linear extent fix `c386fec` | Broker CI/tests pass; GPU-copy/cursor/reconnect/hint checks pass with kayfabe candidate 2 | Not merged to nvkvm-pv main: needs its own HW test with nvkvm-pv's VMM |
| x11-dispsw default-on | (not started) | prep plan in `docs/handoff/2026-10-04/dispsw_default_on_prep.json` | **owner question below**; then the forced-release probe + flip |
| Security S1-21 (critical) | `v3-p1p2` @ `31b64802` | P1+P2 implemented behind `KF3_TSPACE` (default off; inc A count-only on the default path); two reviews + fix round done, CI green; design + box plan in the branch's docs; probe tools `tests/p1p2/window_reach_probe` and `kmod_tring` NOT yet written; owner decisions 8-12 (in the branch docs and `docs/handoff/2026-10-04/p1p2_result.json`) | box: A/B with the flag on, T-WINDOW-USER, T-PHYS-CE (S1-20), an app as an UNPRIVILEGED guest user; then default on |
| Security S1-03/04 | `v3-sec-rawaddr` @ `20a38e96` | done, CI green: addresses behind opaque handles, kf-cuda unsafe 75→64, gates G1/G1b/G1c/G1d + PERIMETER line ratchet (`docs/design/V3_RAWADDR_PERIMETER.md`, §9 = what v3-broker must convert) | HW rows H1-H4 in a security candidate after the display candidates |
| Security §R gates | `v3-sec-perimeter` | S1-01 gates, perimeter ratchet, export table, kf3.c in CI; implementing | candidate after review |
| Guest IOVA | `v3-viommu` @ `05009d07` (ABI 15) | detection + fail-closed + typed seam; CI green; the translator itself (~16-24 eng-days) not built | the 10 box tests in V3_VIOMMU.md §7.5; OD-1..6 recommendations in that doc |
| Windows | `v3-windows` @ `c50fad9a` (took ABI 12 → renumber at merge) | discovery complete: Windows 11 boots with Secure Boot, persistent TPM and the signed GOP; NVIDIA 580.88 boots GSP, then Code 43 at unserved `GR_GFX_POOL_QUERY_SIZE` after 141 RM-init commands. NVIDIA display takeover/CUDA not reached. | branch `docs/design/V3_WINDOWS_DISCOVERY.md` and `traces/v3_windows/discovery_20261004/`; park until display + S1-21 merge |
| managed-memory loudness | `v3-loud-uvm` | real work, never box-run | §R5.7 box run, then a candidate |
| guest UVM fault plane | `v3-uvm-guest` | BLOCKED by owner ruling §E | redesign first |

**KF3 ABI registry:** 11 historical GOP master · 12 v3-broker · 13 v3-dispsw-exp (cand-1) · 14 reserved for
broker-on-13 · 15 v3-viommu · 16 v3-maxfps · v3-windows also took 12 and is renumbered at its merge.
Candidate 2 uses **18** (17 was a discarded scratch ordering and is not reused). Numbers are never reused.

**Open owner questions:**
1. §N condition 3 for x11-dispsw default-on. Restate it as "no kayfabe mapping is ever kernel-mapped,
   and no address space running a display-SW object's channel holds kayfabe memory" (true by tests
   today + after S1-21's fix), and flip the default after `v3-p1p2` merges. The alternative is the
   literal client split (3-5 days). Recommendation: restate.
2. vIOMMU OD-1..OD-6 and S1-32 severity (recommendations in `V3_VIOMMU.md`).
3. §M D2 (non-flip copies at the emulated vblank, send-on-change, none while unwatched) is adopted
   unless the owner objects.

**Boxes (vast; the account is shared, so destroy only ids you created, by id, and verify):** 54032077
(display lane, KDE desktop template), 54049598 (merge bar, KVM template), 54071272 (Windows lane, KVM
template). Find addresses with `vastai show instances --raw`. Destroy any that are idle.

**How to run things:** merge bar = `scripts/bench/box/merge_check.sh` run FROM the box checkout with
`CARGO_BUILD_JOBS=$(nproc)` and a fresh `CARGO_TARGET_DIR` (README there). Apps = `scripts/apps/`
(V3_APP_MATRIX.md). Display = `scripts/bench/display/` (GOP lane, unload_hook B5, dispsw_run.sh,
broker_lane.sh). Candidate procedure = `traces/v3_candidates/cand1_20261003/README.md`. Traps:
- `pgrep -x qemu-system-x86` (never `_64`); put any pkill in its own ssh call.
- Detached box jobs write START/EXIT lines; a launcher's exit code says nothing about the job.
- Host Xids come from journald (the dmesg ring wraps).
- The dev host's `/` is nearly full: build in `/data`.


- ★ **DISPLAY STEP 1 — 2026-10-03 (late): ONE branch, `v3-gop`, supersedes the two below** — both halves
  squashed onto `master` with no compiled binary in any commit, plus the owner's §K (the GOP driver is
  built from source by `crates/kf-gop-image/build.rs`; the ROM is arch-neutral; CI's `aarch64` job builds
  the driver) and every finding of the halves' review fixed (`design/V3_DISPLAY.md` §4.11.12's ⊘ block).
  Merge `v3-gop` instead of `v3-gop-rom`/`v3-gop-kf3`, whose history carries the old `.efi`. Resume: B0a,
  as below; nothing has run on a GPU box. Local stand-in at `adbe6fcd`: 11 PASS, 3 OBSERVED, 0 FAIL,
  including test F1 (a ROM-verifying OVMF runs the signed ROM and refuses unsigned/tail-padded ones). The *Open* and *Merge order* lines below are superseded.
- (superseded the same day, kept as written) ★ **DISPLAY STEP 1 (the boot display) — 2026-10-03: both halves BUILT on branches, not merged, nothing
  run on a GPU box.** `v3-gop-rom` (the `kf-gop` UEFI GOP firmware, `crates/kf-oprom`, the local stand-in,
  11/11 at `3dd574e5`) and, on top of it, `v3-gop-kf3` (the kf3 integration behind property `gop`,
  default off: ROM BAR, KF3 ABI 11; BAR1 seed of store `[0, G)`; the display worker's boot layer; fn 72 kept
  → fn 65's console region). CI green at `37740a4b` (run 37132057723). Design and as-built:
  `design/V3_DISPLAY.md` §4.11 (§4.11.12 = what was built and what changed).
  - **Resume:** box test **B0a** first (today's bench, no build: `boot_vga` and Xorg with no `xorg.conf`),
    then B0, B1, B2 — exact commands in §4.11.9 (`KF_FIRMWARE=ovmf`, `DISPLAY_KF3_EXTRA=gop=on`).
  - **Open:** the owner's §K follow-up — build.rs compiles `firmware/kf-gop` instead of the committed
    `.efi`, and the ROM made arch-neutral (`OWNER_RULINGS.md` §K); owner questions 1–6 in §4.11.11.
  - **Merge order:** `v3-gop-rom`, then `v3-gop-kf3` (it contains the former and master `281a10a1`).

- ★ **RESUME HERE — 2026-10-03 (session wrap before compaction).** Master and v3 are equal and CI
  is green. No Vast box is running.
  - **Done this session, all on master:**
    - the dual licence (`LICENSE`, `OWNER_RULINGS.md` §G);
    - the section-number gate, wired into CI and made able to fail;
    - `design/V3_UVM_STATE_MACHINE.md`;
    - rulings §H (the stub rule; the vGPU guest stack crossed off) and §I (release scope);
    - `design/V3_COOPERATIVE_TIERS.md` (post-release);
    - the display plan of 2026-10-03, in `design/V3_DISPLAY.md` (NEXT block);
    - `reference/gsp_rm_size_and_rpc_surface_580.md`.
  - **Next, in order (owner, §I):**
    1. The display path:
       1. the GOP option ROM;
       2. a stock guest display with no tweaks;
       3. the broker;
       4. the unload tests.
    2. The install path and the sweep.
    3. Windows.
  - **Waiting on the owner** (facts and recommendations for all four: `OWNER_QUESTIONS_2026-10-03.md`):
    - the go-ahead and the seven open questions (Q2–Q8) in `design/V3_SWEEP_AND_INSTALL.md` §4;
    - the `GF100_DISP_SW` choice (A or B);
    - whether the archived C traces holding VBIOS-served PROM reads stay public (§G);
    - renting a GPU box before display testing.
  - **Release items found:**
    - managed-memory faults must fail loudly (`conjugateGradientUM` is silent);
    - add a CUDA virtual-memory-API sample to the sweep;
    - ship the doorbell helper only after the per-token breakdown and a non-nested baseline.
  - **Assumed, not contradicted:** the git identities `x <x>`, `r <r@r>` and `w330 <a@b>` are the
    owner's box sessions. They committed the w384, E0b/E1 and w330 work.
- **2026-10-03 — DESIGN, nothing built:** `design/V3_COOPERATIVE_TIERS.md` is the stock tier plus opt-in stages:
  - doorbell passthrough by token;
  - Linux dynamic memory with allocations that can fail;
  - host-owned UVM;
  - a host helper module.
  - The Windows balloon is dropped.
- **2026-10-02 — RESEARCH, docs only:** `design/V3_UVM_STATE_MACHINE.md` maps the managed-page state machine from the nvidia-uvm 580 source.
  - Owner's hypotheses (§7): H1, H2, H4 and H5 hold on x86 PCIe; H3 only partly; GSP-side placement is refuted.
  - The all-tenant GR hold is NVIDIA's designed "fault and stall" (`uvm_gpu_non_replayable_faults.c:78-81`).
  - Two kf3 gaps: ATOMIC_DISABLE is carried only under `KF3_CARRY_ATOMIC_DISABLE=1`; Ampere resumes a faulted CE channel through CHRAM plus the runlist doorbell, not the C076 method (§8).
- **2026-10-02 — LICENCE:** the repository is now `Apache-2.0 OR GPL-2.0-or-later`, by owner ruling (`OWNER_RULINGS.md` §G; `LICENSE` lists the exceptions). Still open:
  - I7: re-derive the nvproxy-derived `capability.rs`. Until then a QEMU binary built from this tree is not GPL-distributable.
  - Per-file SPDX headers.
  - Owner decision: whether the archived C traces, which hold PROM reads served from a real VBIOS dump, stay public.
- **2026-10-02 — DESIGN, nothing built (branch `v3-sweep`):** the driver × GPU-family sweep and the one-binary install artifact it runs on — `design/V3_SWEEP_AND_INSTALL.md`.
- ★ **WRAP-UP 2026-09-30 (evening) — the owner resumes Friday 2026-10-02. Where everything stands:**
  - **master = v3** = the code of **`afb552ea`** (+ docs/evidence/README only). Bar at that exact revision:
    tests 1754/0, gates 9/9, bare 30/30, thin 30/30, CDP smoke 4/4 shapes (`traces/v3_mc23/`). GitHub CI
    green, incl. the slow job. Families at master code: TU116, AD104, GB205 bars + CUDA ladder 4/4.
  - **Apps:** 61/65 + 6/6 at `2830988f` (CDP fixed). R4 on an RTX 3070 at `738c90e5` (pre-CDP code):
    guest 60/65 + 6/6 = R3, host 71/71 ⇒ no regression from display/ioeventfd/CI (`design/V3_APP_MATRIX.md`
    §R4). **Not run:** guest graphics set, the fast-path-ON app lane, PM and 100-process shapes.
  - **In flight, on a branch (not merged): `v3-uvm-guest` (`bf83d0ca`) — guest managed memory WORKS end to
    end (M2):** `um_probe` gpufirst/cpuinit/prefetch/advise/malloc all `ok bad=0` (3 499 faults delivered and
    replayed, 0 cancels, 0 Xid). M3 partial: `attach_verify`, `UnifiedMemoryStreams`, `UnifiedMemoryPerf`
    (79 713 faults) passed; `UnifiedMemoryPerf` failed ONCE (719 after 197 s: 16 faults timed out after 30 s —
    a guest-side stall, undiagnosed); `conjugateGradientUM` not run (`libnvJitLink` missing on that box).
    Stop note + exact next recipe: `design/V3_UVM_GUEST_FAULT_PLANE.md`; runs: `traces/v3_uvm_guest/`.
  - **Owner questions (open):**
    1. ⊘ **ANSWERED 2026-10-02 — both NO** (`OWNER_RULINGS.md` §E): no MMIO/BAR CPU page-table reads
       (a copy-engine snapshot if a CPU decode is unavoidable); the ~4 s all-tenant stall is not
       acceptable; UVM stays unmerged until the GR hold per fault is measured and bounded. Original text:
       **UVM (new, 2026-09-30):** a parked replayable fault blocks the host GPU's GR engine for EVERY
       tenant (host CUDA included) until serviced or ~4.3 s; kf3's GPU page-table walker needs that engine,
       so the design deadlocks (walk waits 4.3 s, twin killed, Xid 109). The branch's experiment reads the
       guest's page-table words on the CPU while faults are parked — which the v3 rule "no CPU read of a
       guest page table" forbids. (a) May the fault path do that? (b) Is a stall of every tenant's GPU work
       of up to ~4 s per parked fault acceptable? (c) Bug 1624521 ownership check: now or later?
    2. **Doorbells:** build the guest helper now, or first measure a non-nested host (none reachable)?
    3. **X11 desktops:** `GF100_DISP_SW` option A or B (recommendation B, §0 below).
  - **Next, in order (suggested):** (1) UVM after the owner's answers: diagnose the 30 s stall, run
    `conjugateGradientUM`, the Bug-1624521 negative control, the merge bar EFS off/on, merge. (2) A per-token
    LLM time breakdown on nested boxes: doorbells explain ~22 of the ~58 ms/token gap of PyTorch eager; the
    rest is unattributed (offered to the owner, not started). ★ llama.cpp is already 0.92× there (§1 LLM row). (2b) The owner's **doorbell pump** idea
    (`design/V3_DOORBELL_IOEVENTFD.md` §8b, design only): stage 1 ≈ the spin's +16–18 % without an
    always-busy core; stage 2 makes doorbells exitless while pumping, with a stock guest. (3) R4's remainder. (4) §4 items 10–12. (5) Driver
    matrix continuation. (6) Windows last.
  - **CI flake (2026-09-30):** GitHub CI failed once at `8644e477` (a README-only commit) in
    `kf-util`'s `a_blocking_section_outside_a_trap_is_not_a_violation_and_is_not_recorded`: it compared a
    process-global row count that a parallel sibling test writes to (run 36771653449: before 1, after 3).
    Test-only fix `101b1f7e` (`v3-flakefix`, inside `#[cfg(test)]`, CI green) — ★ MERGED the same evening
    after the flake hit master twice more (`baba8084`, `c51b43cc`; the same SHAs passed on v3). A second
    test of the same class then failed master at `2f4ad7c8` (`kayfabe-mmu`'s global epoch, 54 vs 53); it and
    `kayfabe-util`'s identical copy of the first are fixed by `fa4b29aa` (`v3-flakefix2`, test-only, CI
    green), merged. ⚠ **The class:** a test that compares a PROCESS-GLOBAL counter exactly while sibling
    tests change it in parallel. Unobserved but same-shaped, to audit: `kf-util` `lock.rs`
    `the_allowed_acquisitions_are_not_reported` (`in_trap_census() == before`) and `trapwitness.rs`
    `the_enumerated_exception_mints_inside_a_trap_and_counts_itself` (`== before + 1`), plus their
    `kayfabe-util` copies.
  - **Rust 1.99.0 (2026-10-02):** GitHub's runner moved from rustc 1.98.1 to 1.99.0 and CI went red on
    every commit: four `trybuild` compile-fail snapshots changed wording (every snippet still fails to
    compile — the guards hold) and clippy 1.99 reports six new sites (std `Atomic*::fetch_update`
    deprecated → `try_update`, four in kf-chan/kf-qemu; `clippy::double_must_use`, two in the frozen
    kayfabe-device). Fixed by `v3-rust199` (snapshots regenerated; the six sites recorded as migration
    debt in `scripts/ci/clippy-debt.json`), merged; no product code changed. ⚠ Owner decisions: pin the
    toolchain (`rust-toolchain.toml`, bumped deliberately) so a Rust release cannot break CI unannounced;
    and rename to `try_update` (needs ≥1.99 on every box) as a reviewed change.
  - **Durability:** everything is on GitHub. The dev host's local-only branches and two uncommitted Codex
    worktrees are backed up as `backup/host-2026-09-30/*` (13 branches, secret-scanned, not reviewed). Two
    unpushable research-repo corrections are `archive/nvkvm-unpushed-2026-09-30/`. **All vast boxes are
    destroyed** at wrap-up (the recipe re-provisions a box in 7–45 min: `scripts/bench/box/README.md`).

- **2026-09-30 — R4 matrices (partial, `v3-matrix-r4`):** kf3 `738c90e5` (= `3f67ed95`, pre-CDP-fix), RTX 3070,
  nested: apps bare metal 71/71; guest OFF **60/65 + 6/6, identical to R3** (UVM four + CDP); gfx bare metal
  38/38. Not run (owner pause): apps PM / 100-process / `doorbell-ioeventfd=on`, the guest gfx set, the §4.9
  kill cycles. `design/V3_APP_MATRIX.md` §R4, `V3_GFX_TESTSET.md` §0.
- **2026-09-30 — `v3-cdp` MERGED as `v3-mc23` (`afb552ea`): CUDA dynamic parallelism works in a kf3
  guest.** Bar at exactly `afb552ea`: tests 1754/0, gates 9/9, bare 30/30, thin 30/30; CDP smoke on the
  merged binary: all four launch shapes run their child, `cdpSimpleQuicksort` validates (default and 10 000)
  (`traces/v3_mc23/`). Branch detail follows.
  ⊘ *Branch-time text:* **`v3-cdp`: CUDA dynamic parallelism works in a kf3 guest.** The child grid never ran because the guest's SKED-reflected page (libcuda's `UVM_MAP_DYNAMIC_PARALLELISM_REGION`, kind `SMSKED_MESSAGE`) was mirrored as a memory row; fix `46509dce` places it as a message-kind host mapping. At kf3 `090b20d9` (RTX 3060) every CDP launch shape runs its child and `cdpSimpleQuicksort` passes (128/1 000/10 000); merge bar passed at `2830988f` (1752/0, gates 9/9, KF3_RC=0, bare 30/30, thin 30/30; later commits evidence/docs only); app matrix there 61/65 + 6/6. `design/V3_CDP.md`, `traces/v3_cdp/`.
- **2026-09-30 — landed on master earlier the same day** (first `b32aa046`, then the items below), since `5c639f54`:
  - **CI repair** (`v3-ci`, `bf6e7640`): first green GitHub CI; the hardware bar is now fail-closed and
    includes a bare-metal suite (`scripts/bench/box/merge_check.sh`, run from a repo checkout).
  - **Display M1–M3** (`v3-display2`): the emulated display (`display=on`, default **off**) drives a
    virtual monitor. Pixel-exact 1920x1080 scanout, 120/120 flips at 60 Hz, Mint's Cinnamon Wayland
    desktop with vkcube; weston; Xorg with the NVIDIA driver. X11 Cinnamon/Vulkan need
    `GF100_DISP_SW` (owner choice below). `design/V3_DISPLAY.md`, `traces/v3_display/`.
  - **Doorbell fast path** (`v3-ioeventfd`): one KVM ioeventfd per live token, drained off the vCPU,
    Passthrough rung inline, Translated handed on. Default **off** (`doorbell-ioeventfd=on`). Nested
    boxes only: LLM decode 0.29× → 0.32× of host. `design/V3_DOORBELL_IOEVENTFD.md`.
  - **UVM b3 host-only proof** (`v3-uvm-b3`, tools only): the opt-in nvidia-uvm patch delivers a real
    compute fault to the owning process, which maps the page and replays to correct data, alongside
    native host CUDA. No guest fault plane yet. `design/V3_UVM_B3_IMPLEMENTATION.md`, `traces/v3_uvm_b3/`.
  - **Bars:** `v3-mc22` (display + fast path, KF3 ABI 10) passed at `b84250b8`
    (`traces/v3_mc22/`). Then `3f67ed95` (lint fixes plus claim-ledger citations, no behaviour change;
    GitHub CI run 36744304349 green): tests **1742/0**, gates **9/9**, KF3_RC=0, bare **30/30**,
    FG_RC=0, and the display lane pixel-exact. The thin suite was **29/30** on the first run: `--timer`
    CeUtils timeout on the first boot after a stray QEMU was SIGKILLed on the same GPU (§4.9). It was
    **30/30** on the rerun at the same revision. `traces/v3_cifix/`.
  - **Open owner choices:** (1) build the guest doorbell helper now, or first measure a non-nested
    host? Needs a physical host: `172.22.1.20` was still `Network is unreachable` from this workspace
    on 2026-09-30, and the RTX 3050 kiosk PC's address is not recorded in the repo.
    (2) `GF100_DISP_SW` (X11 desktops): A = allocate a host object for it, which breaks the rule that
    kf-rm refuses unknown classes; B = keep refusing it, which leaves X11 partial. Recommendation: B
    now; try a kayfabe-serviced vblank next; A only with a headless guard. (3) b3 registration
    ownership (NVIDIA Bug 1624521) for mutually untrusting VMMs on one GPU: now or later.
  - **Queued:** the UVM guest fault plane (`design/V3_UVM_DEMAND_PAGING.md` §5, "What the guest must see"); CDP child launch (⊘ answered on `v3-cdp`, first line of §0);
    display leftovers (cursor, scaled windows, 16-bit/YUV, the §7 display apps); the rest of the driver
    matrix (570 UVM first-channel wall; hosts 535/590/595/610); re-running the app and graphics matrices
    at this master; Turing on hardware.
  - **CI at `fc2dadc4` (2026-09-30): all three jobs green**, including the **slow** job (the whole suite
    with the soaks, `KAYFABE_SLOW=1`), dispatched as run 36760547434. First green slow run since the CI
    repair; the scheduled runs of 09-28…09-30 had failed at pre-repair revisions.
  - **`v3-ramobj` merged (2026-09-30):** kf3 no longer caches a transient "guest RAM not registered
    yet" for the VM's life — the cause of the one `--timer` failure in the `3f67ed95` bar (§4.9). Bar at
    `f442980e` on TU116: 1744/0, 9/9, bare 30/30, thin 30/30 (`traces/v3_ramobj/`).
  - **Ada AD104 (RTX 4000 Ada) and Blackwell GB205 (RTX 5070) at this master (2026-09-30,
    `traces/v3_families_master/`):** merge bar 1744/0 resp. 1742/0, gates 9/9, bare 30/30, thin 30/30;
    CUDA ladder host 4/4 and guest 4/4 on both. Two dies not measured before.
  - **Turing at this master (2026-09-30, `traces/v3_turing_master/`):** GTX 1660 SUPER (TU116), merge
    bar at `1915bd71` (code `3f67ed95`): tests 1742/0, gates 9/9, bare 30/30, thin 30/30; CUDA ladder
    host 4/4 and guest 4/4.
  - **Boxes (2026-09-30):** `53004208` (RTX 3060, alias `v3060`, retained; the CDP agent's box);
    `53562843` (GTX 1660 SUPER, alias `vtu`, the `v3-ramobj` bar); plus one box per agent (UVM guest
    plane, app/graphics matrix R4), each registered with the reaper. Nothing on any box is the only
    copy of anything.

- **2026-09-29 CI repair in progress:** `codex/ci-repair-2026-09-29` contains the benchmark
  process-identity fix and v3/retained-grader CI repairs. See `CI_V3.md` for test and lint scope.
  Do not promote until CI and the exact-revision hardware bar finish; previous results below
  do not certify this candidate. The live ioeventfd accelerator remains a subsequent experiment.
- **2026-09-29 Paguro retirement:** the owner canceled the retention requirement. Source and
  unique artifacts were re-audited and backed up in Paguro commits `bb8cf91` and `17a51b9` on
  `recovery/resume-2026-09-28`; then instance `53076605` was destroyed. Vast inventory now contains
  only `53004208` (RTX 3060, about $0.1661/hour) for Kayfabe checks. The Windows VM is no longer
  running; its disposable images were not treated as irreplaceable work.
- **2026-09-29 integration verified:** `codex/recovered-integration-2026-09-29` combines the
  published allowlist baseline with Claude `826ef957` (merge `4e6e1fbf`) and recovered Turing
  code `576f5bb0` (merge `cd80712a`). Display allocation/free observation is moved to successful
  object application; rejected operations cannot replace/release live display channels.
  **Exact `d883d0eb`: 1683/0 crate tests, gates 9/9, KF3_RC=0, fresh FG_RC=0, thin 30/30**,
  terminal EXIT 2026-09-29 13:33:52 UTC on retained RTX 3060 `53004208`.
  All 96 evidence files are recovered and hash-checked in `traces/recovered_integration_20260929/`.
  **Promoted to master/v3 as `5c639f54`**, documentation/evidence only after the tested revision.
  Historical TU116 results do not certify the new candidate on Turing hardware.
- **Separate benchmark candidate:** `2676902f` on `codex/benchmark-pid-2026-09-29` binds LLM
  perf counters to the launched QEMU PID/starttime, with GPU-free identity tests. GitHub-backed,
  not included in this merge bar or promotion. No production doorbell fast path or b3 module yet.
- **Display-on smoke at the same code:** guest boot and `nvidia-smi` pass, and the display model
  accepts a core channel. NVKMS then stalls on `0xc67d:0` progress; no DRM node/connector.
  This is incomplete display, not an M1 pass. Evidence is hash-verified and preserved in
  `traces/recovered_display_20260929/`. Next display work remains engine/worker/notifiers.
- **Historical; superseded by retirement above. Paguro recovered again, 2026-09-29:** at about 18:19 UTC, `53076605` reported stopped/exited,
  `GPU error, unable to start instance`, and direct SSH refused. One start retry restored the host.
  The verified Windows launcher reused the existing overlay/firmware/TPM state; QEMU PID 1564
  started at 18:22 UTC and guest SSH returned Windows 10.0.26200.6584. Retained for Paguro; no reset.
- **2026-09-29 checkpoint:** terminal network access is restored; GitHub, Vast and both boxes
  are reachable. The final run at **`61c49f14` passed: 1653/0 tests, gates 9/9, KF3_RC=0,
  fresh FG_RC=0, thin 30/30**, terminal EXIT on 2026-09-28 at 18:44:21 UTC. Evidence is now
  recovered locally in `traces/capability_535_545_audit_20260928/final-run/`. Changes through the
  published `9c3d87fd` are documentation/evidence only. **Promoted to master/v3 as `7c5b2a13`**; the
  earlier sandbox-blocked checkpoint is historical, not a failed test result.

- **2026-09-28 owner decisions:** 535/545 capability extension approved, subject to the independent
  audit and exact-revision merge bar; **b3 patched host nvidia-uvm selected, full native host CUDA
  coexistence required**; doorbells: **non-nested baseline and host-side/ioeventfd work first, guest
  helper afterward**, optional modified guest driver deferred. See §3 and `OWNER_RULINGS.md`.
- **Recovery is preserved, not implicitly merged:** source/evidence at
  [`recovery/resume-2026-09-28`](https://github.com/reindertpelsma/kayfabe/blob/recovery/resume-2026-09-28/docs/RESUME_2026-09-28.md),
  recovered Turing code at `recovery/vast-tuwork-2026-09-28` (`2825c42f`, code tree `576f5bb0`).
  The Claude head `826ef957` has 30 commits beyond the previous baseline, integrated and verified
  in the exact combined candidate above.
  Do not use those historical results to certify a new candidate. The fresh allowlist candidate
  starts from the published baseline, so it does not silently promote the other pending changes.
- **Physical baseline access:** read-only SSH to `172.22.1.20` reports `Network is unreachable`
  from this workspace on 2026-09-28. Prepare the protocol locally; do not label a Vast KVM VM as a
  non-nested host. No physical-host session or display was changed. The owner subsequently
  requested the nested Vast ioeventfd experiment; label its results as nested, not physical-host.
- **Allowlist audit complete:** full 550–610 before/after policy comparison plus compiler-derived
  checks for all 16 admitted controls from the previously unchecked shared groups. Both the
  pre-change golden and the two header fixtures were independently regenerated on the trusted
  development host with byte-identical results. Final merge bar at `61c49f14` passed;
  see `traces/capability_535_545_audit_20260928/README.md` for exact revisions and limitations.
- **Doorbell work is mechanism evidence, not production acceleration:** the GPU-free KVM probe
  verifies token-matched ioeventfd and unmatched MMIO fallback on a read-only memslot. No timers
  or intentional batching delay; initially accelerate passthrough only, retaining the existing
  translated ordering and real-GPU completion paths. The generic Emulated route is not a live
  kf3 channel backend. See `design/V3_DOORBELL_BASELINE.md`.
- **UVM next:** finish the checked registration/lifetime proof described in
  `design/V3_UVM_B3_IMPLEMENTATION.md`, then build the bounded host-only experiment. No b3 patch
  has been loaded. In particular, a UVM channel-memory reference alone does not prevent hardware
  state from being detached and freed; delayed userspace replies need explicit invalidation.
- **Historical box inventory, superseded by retirement above (2026-09-29):** `53080587` (GTX 1660 SUPER) was retired after
  rechecking all 493 saved evidence hashes and its recovered source tree. Retained: `53004208`
  (RTX 3060, verification/next Kayfabe tests) and `53076605` (Paguro Windows). No new rentals.
  Both retained instances are running, and Paguro's Windows QEMU remains live. Combined listed
  rate is approximately $0.5203/hour before additional fees. Preserve evidence before retirement.
- **Historical pause notes below are superseded by this resumption.** mc21 was promoted to
  `8ab92bf4`; older "awaiting promotion", "all boxes being destroyed", and open-choice lines below
  describe earlier points in the campaign, not current actions.
- **2026-09-27 19:59 UTC — `v3-mc21` (= master `db038f5f` + `origin/v3-display`, merge `4c48ca0c`) PASSED THE
  FULL MERGE BAR** at exactly `4c48ca0c`: crate tests **1651 / 0**, gates **9/9**, `KF3_RC=0` (VNC+pixman build),
  `FG_RC=0`, fast suite **30/30** — same box. Evidence: `traces/v3_mc21/`. Display stays default-off.
  **Master and v3 fast-forwarded to `8ab92bf4`** (= `4c48ca0c` + evidence/docs; owner go-ahead in the session,
  2026-09-28). The `display=on` M0 lane on GA106 (`traces/v3_display/mc21m0/`) reproduces GA102's `m0a`
  exactly: KernelDisplay up, NVKMS stops at `0x730101`/`0x730102`/`0x730107`/`0x730151` ⇒ displayless.
- **Boxes from a cloud session:** the container has no SSH egress, so a box is driven through the **`vx`
  kit** (execd behind a cloudflared quick tunnel). `provision_full.sh` and `merge_check.sh` run unchanged
  that way; `traces/v3_mc20/README.md` records the v3-mc20 bar run like this.
- **`v3-uvm-e6pp` (`c6765f5c`): E6″ phase 0 ran** (a capture on a stock box — no fault numbers yet); §2, §3.2.
- ⊘ *Superseded 2026-09-28 (master is now `8ab92bf4`, above; v3-mc21 passed):* **2026-09-27 19:38 UTC — master
  and v3 = `db038f5f`** (= `c0ef7b75`, the revision that passed the bar below, + the commit that added its
  evidence, `traces/v3_mc20/` and this file only), fast-forwarded with the owner's go-ahead. `v3-drivers~1`
  (`8f9bdd14`) is therefore **ON MASTER** (via merge `5018bb57`); `v3-drivers` now holds only the held 535/545
  allowlist commit, **`a50265f8`** (§3.1). Merge candidate `v3-mc21` = `4c48ca0c` … bar in progress.
- **2026-09-27 18:27 UTC — `v3-mc20` PASSED THE FULL MERGE BAR** at `c0ef7b75` (= `5018bb57` + docs only), on
  branch `claude/kayfabe-gpu-testing-m0cv1q`: crate tests **1639 / 0**, gates **9/9**, `KF3_RC=0`, `FG_RC=0`,
  fast suite **30/30** — vast 53004208, RTX 3060 GA106, host 580.159.04 open. Evidence: `traces/v3_mc20/`.
  ⊘ *2026-09-27 19:38 UTC: done; see the first line.* Ready to fast-forward master and v3 (owner go-ahead
  asked in the session). Cloud sessions now reach boxes through execd behind a cloudflared quick tunnel
  (the `vx` kit), not SSH.
- ⊘ *Superseded by the line above:* 2026-09-27, resumed (cloud session): merge candidate `v3-mc20` = master `0d3ecde9` + `v3-drivers~1` (`8f9bdd14`,
  the held 535/545 allowlist commit left out) — merge commit `5018bb57`, clean, on branch `claude/relaxed-babbage-7zz89s`.
  GPU-free: all `kf-*` crate tests **1639 passed / 0 failed** at `5018bb57`. Hardware bar (gates, kf3 build, 30/30)
  NOT run: this session's container has no SSH egress, so `merge_check.sh` cannot be driven on a box from here.
  Do not promote to master until the bar passes at `5018bb57`.
- ⊘ *Superseded 2026-09-27 19:38 UTC — master is now `db038f5f` (v3-mc20), see the first line:*
  **Master = `v3-mc19` verified** (`08bf7f18`: 1637 tests / 0 failed, gates 9/9, 30/30) + docs. All vast boxes are being
  destroyed; nothing depends on a box or on local files. Every branch below is on GitHub.
- **`v3-initrace` is ON MASTER** (verified at `08bf7f18`). It contains: USERD cleared at Translated-channel birth (the re-init "flake": a reborn CeUtils channel
  inherited a leftover GP_PUT — 0/300 after, 20/20 injected failures before) and WPR2 served at the guest's
  own FWSEC-FRTS offset after a failed GSP boot (retry boots; 20/20 later opens pass).
- ⊘ *Merged 2026-09-27: `8f9bdd14` is on master via v3-mc20 (`5018bb57`; bar at `c0ef7b75`); the held
  allowlist commit is now `a50265f8`, the only commit on `v3-drivers` beyond master. The "Next for that
  branch" items below still stand:*
  **`v3-drivers` mergeable head `8f9bdd14`** (= `42b25354` + a default-off logging aid + its stop note; the bar was
  last fully green at `1837166d`, the thin suite at the newest head never ran) (rebased on `6ec7ec1a`; the 535/545 allowlist commit is LAST and held
  for owner review): guest 610 ladder 4/4, hosts 575.57.08 / 580.95.05 / 580.65.06 green on every row,
  (runlist, chid) token index, ≤545 GET_CHIP_INFO carry, 595+ GSP heartbeat. Merge it onto the new master
  after `v3-mc19`, then run the bar. Next for that branch: the 570 guest's UVM-first-channel wall (token
  0x803 reads its GPFIFO before the mirror maps it) and a 570 thin-guest NULL deref seen on master.
- ⊘ *2026-09-28: ON MASTER — merged as v3-mc21 `4c48ca0c`, bar passed (§0 first line), master `8ab92bf4`; step (1)
  is wired in code since (`docs/design/V3_DISPLAY.md` step-(1) note); superseded text follows:*
  **`v3-display` (`adbea28e`)**: Phase 1 decided — emulate the display hardware the stock guest driver expects
  (~3–5 weeks to a desktop on GA10x); M0 (behind `display=on`, default off) brings the guest's display layer
  up; next step and plan in the stop note at the top of `docs/design/V3_DISPLAY.md`. Merge bar not run.
- **Owner decisions resolved 2026-09-28**: §3. The cloud-only execd shared-key question remains
  separate; direct SSH without agent forwarding avoids it and does not block this work.

## 1. Master, and what it has been verified to do

Every promotion to master passed the merge bar (`scripts/bench/box/merge_check.sh`): all `kf-*` crate
tests, v3 gates 9/9, a kf3 build of that exact revision, and the 30-arm thin-guest suite 30/30.
Latest completed bar: **`3f67ed95`** (2026-09-30) — **1742 tests / 0 failed**, gates **9/9**, kf3 build,
bare-metal suite **30/30**, fresh fast guest, thin suite **30/30** on the rerun (29/30 first run, §4.9),
display lane pixel-exact; RTX 3060 (GA106), host 580.159.04; `traces/v3_cifix/`. Only evidence
follows that code on master. New code candidates require their own bar. The bar before it was the
recovered integration **`d883d0eb`** (1683 / 0, 9/9, 30/30; `traces/recovered_integration_20260929/`),
which included the recovered Turing code but was tested on GA106 only. ★ Turing since: the current
master's code passed the same bar on a TU116, plus the CUDA ladder (`traces/v3_turing_master/`).

| Area | State (hardware-measured unless marked) | Doc |
|---|---|---|
| Families | GA10x (GA106/GA104/GA102) 30/30; Ada AD106 30/30, **AD104 (RTX 4000 Ada) at master 2026-09-30: bar + ladder 4/4**; Blackwell GB203 (RTX 5080) 30/30 + CUDA ladder, **GB205 (RTX 5070) at master 2026-09-30: bar + ladder 4/4** (`traces/v3_families_master/`); floor-swept RTX 3060 Ti 30/30. TU116 (GTX 1660 SUPER) at master `1915bd71` (code `3f67ed95`, 2026-09-30): tests 1742/0, gates 9/9, bare 30/30, thin 30/30, CUDA ladder host 4/4 + guest 4/4 (`traces/v3_turing_master/`). GA100, Hopper, GB10x remain source-derived only; GA100/GB10B refused by name | [Turing recovery evidence](https://github.com/reindertpelsma/kayfabe/blob/recovery/resume-2026-09-28/docs/RESUME_2026-09-28.md), `design/V3_FAMILY_PORT_ADA.md`, `V3_FAMILY_PORT_BLACKWELL.md`, `V3_FLOORSWEPT_GR.md` |
| Multi-GPU | distinct host GPUs in one VM work (8×3060 box); per-card BAR1 budget refused at realize | `design/V3_MULTI_GPU_AUDIT.md` |
| CUDA apps | **61/65 + 6/6 probes at `2830988f`** (`v3-cdp`, host 71/71; CDP fixed, merged as `afb552ea`); the four remaining failures are UVM demand paging (guest fault plane in progress on `v3-uvm-guest`). Earlier: 60/65 at `4c48ca0c`, 100/100 processes; R4 (RTX 3070, `738c90e5` pre-CDP-fix, OFF only): 60/65 + 6/6, = R3 | `design/V3_APP_MATRIX.md` §R3, §R4 |
| Graphics / video | nvkvm-pv's headless graphics set + 15 more items: **38/38** on an RTX 3070 (31 byte-identical to bare metal; OFA optical flow advertised); NVENC/NVDEC byte-exact. Per-call GPU waits are slow on nested boxes (`glFinish` 62 vs 9 µs). R4 (`738c90e5`): bare metal 38/38 only, guest not re-run | `design/V3_GFX_TESTSET.md` (display-phase list §7), `V3_HEADLESS_GRAPHICS.md`, `V3_VIDEO_ENGINES.md` |
| Memory plane | pooled walker capacity (no per-space 16k-run wall); batched host maps; big-PTE slot ownership; guest PTE read-only/volatile carried, PRIV leaves withheld from user twins; **every host map snoops the CPU cache** (`NVOS46_FLAGS_CACHE_SNOOP_ENABLE` — without it a CE read stale DRAM on bare-metal hosts; nested VM boxes hid it) | `design/V3_BUILD.md`, `V3_BATCHED_MAP.md`, `traces/v3_adasys/FINDING.txt` |
| Refusals | audited host-vs-guest: forged completions removed (MC_SERVICE_INTERRUPTS, sysmembar flush); the rest classified | `design/V3_REFUSAL_AUDIT.md` |
| Driver matrix | ★ (2026-09-27, the `v3-drivers` code on master since `5018bb57`) CUDA ladder 4/4 for guests 580.159.04 / 580.105.08 / 590.48.01 / 595.84 / 575.57.08 / 610.57.04; hosts 575.57.08 / 580.95.05 / 580.65.06 gates 9/9, thin 30/30, ladder 4/4, mixed pairs 4/4 — measured on `v3-drivers` heads before the merge (grid §6.0, derived from `traces/driver_matrix/walk/`); 535/545 capability rows approved, independently audited and merged (§3.1); no 535/545 end-to-end application claim. Earlier: 29 ogkm tags measured into generated tables; guest 580.x works end to end; ≤575 guests pass RM init (fn 54/79 carried); host 575.57.08 gates 9/9 | `design/V3_DRIVER_MATRIX.md` |
| LLM | ★ (2026-09-30, found in the R3 evidence) **llama.cpp: 0.92× host decode, 0.96× prefill** (Qwen2.5-1.5B Q4_K_M, `llama-bench`, nested RTX 3060, trapped doorbells, kf3 `4c48ca0c`; ~4 600 doorbells for ~200 tokens + prefill). **PyTorch eager, Qwen2-0.5B: 0.29×** (0.32× fast path) — the worst case, ~1 084 doorbells per token. Per-token doorbell cost dominates only launch-per-op workloads | `design/V3_BUILD.md`, `V3_GUEST_DOORBELL_MODULE.md` |
| Hardware boundary | every hardware constant pinned to ogkm headers by 43 GPU-free tests; generator `tools/derive_hwref.sh` | `design/V3_HW_BOUNDARY_INVENTORY.md` |
| Pre-v3 tree | archived under `archive/`; 16 `kayfabe-*` crates kept only because the 30-arm grader uses them | `archive/README.md` |

## 2. Remaining work and historical branches

The recovery inventory linked in §0 is authoritative for the unmerged Claude/Turing work.
Do not restart completed investigations from the historical table below. The current scoped
candidate is `codex/allowlist-535-545-audit`; its policy extension is approved and independently
audited, not held for an unanswered owner decision. The 30-commit Claude line and recovered Turing
line remain separately preserved and need integration/testing; their old results are not a
combined-head certification. CDP child execution and display step 1 remain substantive follow-ups.

Historical branch table from the earlier pause:

| Branch | What it is | State / what it needs |
|---|---|---|
| `v3-drivers` | driver matrix (both axes) | everything except its last commit is merged up to `bcd55198`; **last commit `ee35ca4a` = 535/545 capability allowlist — HELD for owner review**; walk continues (575/590/595 guests pass the ladder; 570/565/550 blocked by §4.1) |
| `v3-initrace` | the adapter re-init race + failed-boot recovery | in progress; see §4.1 |
| `v3-adasys` | RTX 4070 sysmem copy failure, bare metal first | in progress; see §4.2 |
| `v3-uvm-n4` | UVM demand-paging research + N4 experiments E5/E6/E6′ | research; E6″ not run; see §3.2 |
| `v3-mgpu-audit` | original multi-GPU audit doc | **superseded** by the version merged with the multi-GPU fix |

## 3. Decisions resolved by the owner — 2026-09-28

### 3.1 The 535/545 capability allowlist (`a50265f8` on `v3-drivers`) — approved

The owner approved the scoped extension and the recommended audit/test work. The shared groups
not covered by the original header sweep have now been checked; the full resolved 550+ policies,
including names/IDs and rule/deny behavior, match the pre-change baseline. Merge only after
the exact candidate passes the normal bar. `ee35ca4a` in older references was a pre-rebase name.

Ports nvproxy's v535_104_05 / v545_23_06 capability rows so those guests get a capability surface
instead of a realize refusal. Existing tables are unchanged (pinned by test). Review points: every
withheld entry is also measured absent from that version's headers; `NVC36F_CTRL_GET_CLASS_ENGINEID` is
allowed at 535/545 (as at 550); the shared-floor rows (`NV00FD`, `NV9096`, `NV906F`, `NV208F`,
`NV90E6`, conf-compute, semaphore surface) now also have exact-tag NVIDIA compiler evidence. Full table:
`design/V3_DRIVER_MATRIX.md` §8.2 ruling 5.

### 3.2 UVM demand paging — route and next experiment

**Selected: b3**, a narrowly scoped opt-in patch to the host's open nvidia-uvm for guest fault
handling. **Full host CUDA coexistence is a requirement**, not a feature to trade away. The VMM
remains unprivileged and untrusted. Preserve ordinary UVM behavior for non-opted-in address spaces.

**N4 is not the next experiment.** Correction to the earlier "host CUDA on that GPU" claim:
580.159.04's `nvUvmInterfaceRegisterUvmCallbacks` has one global registrant, so the replacement
module excludes stock UVM throughout the same host kernel, including a second GPU. Historical N4
research through `v3-uvm-e6pp` (`c6765f5c`) remains useful source evidence. Phase 0 traced a stock-UVM
launch; it did not demonstrate replacement-module replay, cancellation or fault latency.

The next bounded milestone is a **host-only b3 proof**: authenticate ownership, deliver a real
replayable compute fault, map/repair and replay to correct data, cancel only the intended channel,
reject forged/stale handles, bound timeout/teardown, and run ordinary host CUDA concurrently.
The `DupAddressSpace` / `RetainChannel` helper calls must not be assumed to authenticate the caller;
the research's ownership correction applies to b3 too. Production guest fault injection follows
the privileged proof, not the other way around. No route-selection answer is pending from the owner.

### 3.3 The guest doorbell module — how to proceed

**Selected: non-nested baseline and cheaper host-side exits first; optional helper afterward.**
`ioeventfd` remains in the pipeline. Measure vCPU return latency independently of delayed GPU
notification, with idle/synchronous and deep-queue workloads plus content/progress checks. Faster
vCPU return can help throughput without improving single-launch latency. Coalescing is valid only
with preserved ordering and no lost wakeups. **Owner refinement: no batching timers or intentional
delay; act immediately and coalesce only already-pending notifications incidentally.** The optional modified guest NVIDIA driver remains open
for later; it is not selected now. The helper remains design-only with stock-guest fallback.

The owner's ~0.70x bare-metal expectation and possible Windows batching advantage are hypotheses.
Record nesting, same-host controls, doorbells per token, CPU use and correctness hashes; distinguish
a guest kernel entry from a hardware VM exit. Windows performance needs its own measurement.

### 3.4 The execd PSK on `vx` boxes and "no secrets on a box" (raised 2026-09-27)

Cloud sessions have no SSH egress, so they drive boxes through execd behind a cloudflared quick tunnel
(the `vx` kit; `scripts/bench/box/README.md`, last section). The v3-mc20 bar ran this way. Each such box
holds execd's HMAC pre-shared key (`/root/.execd_key`, written by the onstart). Root on a hostile box
can read that key, and the key runs commands as root on every box that holds the same key. With the
kit's per-container token, those are the boxes one session rented. With a shared `$VAST_EXEC_PSK` (set
so that boxes can pass between sessions), they are every box rented with that key. The key opens no
vast account and no GitHub. Ruling F says "no secrets on a box", and security-policy changes need the
owner. The question: is a box-scoped key like this allowed? If yes, may the key be shared across
sessions? If no, what should replace it (for example, a key per box derived from the session's key and
the instance id)? Until the owner rules, the README asks for the per-container token unless a box
must pass to another session.

## 4. Queued / in-progress investigations

1. ~~Adapter re-init race~~ **ANSWERED + FIXED, ON MASTER since `08bf7f18`** (v3-mc19; bar 1637 / 0,
   gates 9/9, 30/30 — see §0). Its evidence is on master too: `traces/v3_initrace/`, and
   `traces/driver_matrix/walk/` since `5018bb57`; `scripts/drivermatrix/initflake_evidence.sh` was on
   master before either, since `36ef0358` (2026-09-26), so "evidence on `v3-drivers`" below was only
   half right. The 570
   guest's next wall is a different one (the UVM first channel: `design/V3_DRIVER_MATRIX.md` stop note, step 1).
   **Adapter re-init race — ROOT-CAUSED + FIXED on `v3-mc19` (uncleared vidmem USERD; see §0).** Original note: Guest 570.148.08 reproduces it every boot (fat guest,
   no persistence mode): on re-init the CeUtils channel is reborn with the same token, host channel id
   and GPFIFO VA; the ring reader yields no words, nothing is pushed, yet the completion tail retires the
   entry (GP_GET authored 1) — the guest sees its work "done" and times out. Rare on 580.x (3/≈270
   boots). A variant: completion observed but the guest CPU read back the wrong data ⇒ suspect the view
   path. Fix rule: an entry that yields no words must wait or refuse — never retire. Also: after one failed
   GSP boot every retry fails on TU/GA10x/Ada (WPR-end margin not modelled). Branch `v3-initrace`;
   evidence on `v3-drivers` (`traces/driver_matrix/walk/`, `scripts/drivermatrix/initflake_evidence.sh`).
2. ~~RTX 4070 sysmem copies fail~~ **ANSWERED + FIXED, ON MASTER (merge `8b32178f`, v3-mc18; promoted as
   `6ec7ec1a`)**: kayfabe's missing CACHE_SNOOP bit, not the box (`traces/v3_adasys/FINDING.txt`).
   Lesson: run memory-plane changes on a non-VM host too.
3. **Hardware gaps** ranked in `design/V3_HW_BOUNDARY_INVENTORY.md` §5.2 (GB100/GB102 PCIe capability,
   VER3 unmapped-big-PTE and sparse-PDE encodings, GB10x 256 GiB leaf, 128 KiB pages, …).
4. **Host GPU wedge after QEMU exit** seen once on an RTX 3090 (GFW boot "progress 0xff", recovered by
   FLR). Watch for recurrence — if the VMM can leave the host GPU unusable, that is a host-side defect.
5. **Turing on hardware** (owner: later, if time).
6. ~~**`cuda/walk` tidy-up**~~ **DONE 2026-09-28, on master:** one `offset_of!` decoder per walk-report
   struct, `read_struct` gone (`17009a4f`); run-flag bit ranges named after `kf_walk.h`, no magic
   shifts at the readers (`30d0ee77`); the VER2/VER3 descriptors are checked field by field against
   each die group's `dev_mmu.h` (`kf-mem/tests/walk_format_vs_ogkm.rs`). What remains is VER3
   (Hopper/Blackwell) semantics, not tidiness: the PDE PCF sparse encodings and the unmapped-big-PTE
   case (`design/V3_HW_BOUNDARY_INVENTORY.md` rows for VER3; item 3 above).
7. Re-run the full CUDA app matrix at the current master (the 58/65 predates several fixes).
8. **Display** (started: M0 on `v3-display`, in the v3-mc21 candidate — §0, §2), then a desktop, then
   **Windows** (roadmap).
9. ⊘ **ROOT-CAUSED 2026-09-30 — not the killed VMM: a kf3 startup race.** That boot's QEMU log says
   `no fd-backed guest RAM block` at t=0.057 s, and then refuses a sysmem leaf of the CeUtils VA space
   by name (`map 0x120070000: guest-RAM row and no RAM object`). Its invalidate stayed unreconciled, so
   RM's `memmgrTestCeUtils` (vid→sys copy through CeUtils) never ran. It is the only one of the 60 boots
   of both runs with that refusal; all 59 others built the 2 GiB RAM object. Mechanism:
   `prewarm` checks for the fd, then `guest_ram_object` looked it up again inside a `OnceLock`.
   QEMU's memory listener re-renders guest RAM at reset (delete, then add), and the second lookup
   hit that gap. The `OnceLock` then cached "no RAM" for the VM's life; the two retries took 2 and
   0 µs, i.e. the cached error, where a real import takes ~1.8 s. Fix on branch **`v3-ramobj`**
   (`once_after`: the fd is read once, outside the cell; "not yet" is never cached; prewarm retries
   next tick) — ★ **MERGED 2026-09-30** after its bar at `f442980e` on TU116 (1744/0, 9/9, bare 30/30,
   thin 30/30; `traces/v3_ramobj/`). Residual: other `RamMap` readers can still observe a
   mid-transaction topology (a refusal by name, not a cached one); fixing that needs the
   listener's `begin`/`commit` staging in `kf3.c`. The text below is the original,
   superseded hypothesis.
   ⊘ *Superseded:* **CeUtils timeout on the first guest init after a killed VMM** (2026-09-30, `traces/v3_cifix/`, seen once).
   At 16:37:56Z a stray QEMU (a kf3 binary of `bf6e7640` left by an earlier mis-launched chain) was
   killed on the RTX 3060 (`kill`, then `kill -9`). The merge check started at 16:38; its bare-metal
   suite used the host GPU in between and passed 30/30. The thin suite's first arm, `--timer`, was the
   first kf3 guest boot after the kill, and it timed out: guest `memmgrMemSet` returned NV_ERR_TIMEOUT
   at 150.7 s, then `ce_utils.c:349` asserted (`lastCompletedPayload == lastSubmittedPayload`). The
   other 29 arms and a full 30-arm rerun passed. Open question: can state left by a killed VMM stall
   the next guest's first CeUtils completion, even though host-side clients work? If so, it is a
   host-side defect in the same class as §4.4. First step: kill a VMM mid-arm, then boot one arm.
10. **CDP on Hopper/Blackwell hardware** (2026-09-30): the SYSTEM_NON_COHERENT SKED-reflected page
    (`kf_chip::sked`) is covered by GPU-free tests only; run `scripts/cdp/cdp_guest.sh` on a GB20x box.
11. **A killed hung guest CUDA process poisons the boot** (`design/V3_CDP.md` §6): after a pre-fix CDP hang
    was SIGKILLed, every later process in that boot failed `cudaSetDeviceFlags` with 999. Not diagnosed;
    the bare-metal control (kill a hung process, then start another) has not been run. If bare metal
    recovers, it is a kayfabe teardown gap (bare-metal pass + guest fail ⇒ kayfabe bug).
12. **Instrument:** `boot_capture.sh`'s host-dmesg line-count watermark reads 0 once a long-lived box's
    kernel ring is full, so "host Xid 0" counts from such boxes were never measured. `scripts/cdp/`
    uses `journalctl -k --since` instead; port that to `boot_capture.sh`.

## 5. How work was run (so it can be run again)

- **Boxes:** `scripts/bench/box/README.md` — rent, register with the idle watchdog
  (`vast_reaper.sh`), provision (`provision_full.sh`), check (`merge_check.sh`), destroy. From a cloud
  session (no SSH egress) the same scripts are driven through the `vx` kit — execd behind a cloudflared
  quick tunnel (the v3-mc20 bar ran this way: `traces/v3_mc20/README.md`).
- **Merging:** agents push sub-branches; a merge branch `v3-mcN` combines verified work on the current
  master; the full bar runs on a box at that exact revision; then master and v3 are fast-forwarded. Only
  docs/traces may be added on top of a verified revision without re-running the bar.
- **Local host:** shared and small; builds go on boxes; local cargo only under
  `flock /tmp/claude-0/kf-local-cargo.lock`, 2 jobs, own target dir deleted after.

## 6. Lessons from this campaign worth keeping

- **A refusal on a wait/flush path is a forged completion.** Symptom: results faster than bare metal
  (clpeak "FP64 450 GFLOPS" vs 218). Audit non-OK statuses against a bare-metal trace (nvdiff) first.
- **An entry retired without its work executing is also a forged completion** (§4.1).
- **Merges combine branches that each passed alone** — two branches added a same-named field with
  different meanings; one branch left another's test red; two bumped the same ABI number. Resolve
  semantically and re-run the whole bar on the merged revision.
- **A test runner without `--no-fail-fast` under-reports** (it stopped at the first red crate).
- **A watchdog must not read "cannot probe" as "idle".**
- **Boxes vanish** (vast destroyed several mid-run); evidence must already be in git.
- **A coexistence test that tolerates a stall reads the stall as a pass** (2026-09-30): the b3 host proof
  logged host CUDA stalling for a whole 3 s fault park as coexistence PASSED; the guest plane then
  deadlocked on exactly that (`design/V3_UVM_GUEST_FAULT_PLANE.md`). Bound the latency, not just the outcome.
- **A cached "not yet" outlives its cause** (2026-09-30, §4.9): a `OnceLock` kept a transient "no guest RAM"
  for a VM's life. And the first plausible cause (a killed QEMU just before) was wrong: read the failed run's
  own log against a passing run of the same arm, and grep the signature's base rate across all archived logs.
- **`git stash` is shared by every worktree of a repo** — another agent's `stash pop` applied someone
  else's stash. Agents working in parallel worktrees save a patch file instead of stashing.

STATUS: DESIGN-ONLY, 2026-11-13. Unified code is GPU-free tested, not hardware-qualified.

# One proprietary-image check pipeline

CODE READ: `scripts/bench/windows/propimage_check.py` is the single check module and
`scripts/ci/test_propimage_check.py` its single test file. The former app module is
folded into it. Image packaging and identity derivation remain in the existing
`scripts/bench/windows/propimage.py` library; this is not a second check pipeline.

MEASURED predecessor evidence (2026-11-13, RTX 4090 host, revision `439ff38e7`):
`traces/hwrun_20261113/README.md`, section B, records a new display device instance
without EnableGpuFirmware, Code 43 before the key was set, then seed GPU UUID with
580.88 after restart. swtpm 0.6.3 could not read TPM state; swtpm/libtpms 0.10.1 worked.
This does not qualify the unified pipeline, which has not run on a real guest.

CODE READ: the pipeline checks swtpm >= 0.10 before unpack or VM startup. It preserves
SHA256 verification of both archive members and verifies those hashes on directory
reuse. The immutable backing alias is `image.base.qcow2`; only its new qcow2 overlay
is passed to QEMU. CI mode is fixed, with restrict=on, a copied stop-time override
that cannot move backwards, restored TPM/firmware, the exported KF3 flags, guest SSH
wait, per-boot GPU fix, blocking identity verification, checks, and shutdown.
A new empty work directory is required. Started processes are cleaned through an
ExitStack even on setup errors; no process-name kill sweeps or internet boots exist.

CODE READ: GPU fix examines all NVIDIA display class instances, sets missing
EnableGpuFirmware=1, restarts NVIDIA display devices when keys changed, and requires
one nvidia-smi UUID exactly equal to the seed UUID. A successful command alone or
an arbitrary GPU UUID cannot pass that step. Runtime SSH key material is used only
for the image's local guest transport; this lane read no live image state or keys.

INFERRED falsifiers, stated before qualification: old swtpm accepted; a second
instance left unfixed; wrong GPU UUID accepted; a stale/unfinished benchmark or
unchanged launcher screen passed; wlms shutdown blamed on an app; an existing
finished label treated as new work; or an error path leaving an owned VM running.
Offline regressions and mutations exercise these counterexamples. They cannot
establish real guest behavior, GPU rendering or the availability of UI controls.

CODE READ: per-run guest evidence lives in `C:\kfapps\<run-id>\<app>`. The host pulls
allowlisted JSON, STEP, benchmark HTML and PNG files to `<out>/<app>/<run-id>/`.
Metadata JSON is published atomically. Host verdicts bind app/run identity and
use guest-relative start/end/mtime, nonzero matching byte length and parseable
content. `result.json` and `result.md` record verdict, evidence kind and class;
app passes identify FILE or UIA. SCREEN remains a supported table label but no
screenshot-only automatic work verdict is implemented. A window is LAUNCH_ONLY,
never PASS. Aggregate success requires every requested row to pass.

| App | Path / evidence | Qualification boundary |
|---|---|---|
| SolidWorks | CODE READ: default LAUNCH_ONLY; optional `--solidworks-export` uses public COM OpenDoc6, ActivateDoc3 and SaveAs, then checks `model.step` entity data and API success (FILE). | MEASURED predecessor title: SOLIDWORKS Design Standard 2026 SP5.2 - Academic Use Only. INFERRED: installed samples/COM export are unverified; STEP export is application work, not proof of GPU rendering. |
| Resolve 21 free | CODE READ: default LAUNCH_ONLY; optional collected UIA Run/status plan requires a new progress-to-finished transition (UIA). | MEASURED predecessor reached Project Manager. DOC READ: external scripting is Studio-only. INFERRED: free-build console exposure, a local project, rendering controls and decodable media output remain unverified. |
| 3DMark | CODE READ: default LAUNCH_ONLY; optional `--experiment-3dmark-accessibility` adds the Chromium accessibility switch only to its launch. | MEASURED predecessor UIA exposed two panes. INFERRED: the switch is unproven and may do nothing; it is not an automation fix. DOC READ: documented CLI/XML export requires Professional; activation alone does not establish that edition. |
| Superposition Basic | CODE READ: default LAUNCH_ONLY; explicit `--superposition-run` derives the RUN click from the UIA window rectangle using the measured relative anchor. It removes the old log before launch and captures before/result screens. | MEASURED predecessor Qt launcher had no UIA children. `traces/hwrun_20261113/propimage/results/log.html` records running then stopped; `propimage/shots/sp_after.png` is its result screen. SAVE was missed after launcher closure; no saved score report is claimed. |
| eDrawings | CODE READ: default LAUNCH_ONLY and UIA dump; optional collected UIA plan. | DOC READ: public OLE/ActiveX OpenDoc/Save exist. INFERRED: installed control registration, successful-load event, sample and native saved-format parser remain unverified. |
| Wukong | CODE READ: launch existing desktop shortcut only; report fresh candidate game processes, never drive Steam or produce a work pass. | MEASURED predecessor showed Steam's launch dialog, not the benchmark. INFERRED: candidate process names and any native benchmark result-file schema remain unverified. Steam stays signed out. |
| Edge | CODE READ: default LAUNCH_ONLY and UIA dump; optional collected UIA plan. | DOC READ: Microsoft supports WebDriver automation. INFERRED: matching driver and a defined local workload with semantic completion assertions remain unverified; no runtime downloads. |

CODE READ: Superposition FILE evidence requires a fresh log with exactly one ordered
running/stopped interval, distinct before/result PNGs, and local tesseract output
with a positive named score plus ordered positive min/avg/max FPS. Log alone,
renderer identification, arbitrary changed pixels or a screenshot alone cannot
pass. Missing OCR is a harness prerequisite error, not an app failure. Native log
and screens remain saved. No score is guessed and no successful SAVE is claimed.
INFERRED: rectangle fractions and the conservative OCR layout parser need real-guest
qualification; resizing, DPI or another launcher version may invalidate them.

CODE READ: `--plans FILE` accepts collected selectors (AutomationId, optional Name,
ControlType) for one Run control and a stable status control, exact running/finished
text and a bounded timeout. The process-scoped selector must be unambiguous; old
running/finished states are rejected. The bounded UIA dump supports discovery.
INFERRED: use UIA fallback only after establishing that usable native saving is
unavailable and qualifying the selected controls' work semantics. The generic plan
does not prepare projects, configure workloads or drive Save dialogs.

MEASURED predecessor licence event: `traces/hwrun_20261113/spec/enscape_events.txt`
records System 1074 from wlms.exe about 62 minutes after boot (the evaluation image).
CODE READ: per-boot event queries record that event as class harness, never app FAIL.
Before each app, a monotonic boot-age guard warns and marks NOT_RUN when its maximum
budget would cross 55 minutes. Lost guest transport is harness/unknown rather than
an invented app failure. Events are queried again after each app and saved locally.
INFERRED limitation: if shutdown cuts SSH before event retrieval, the specific wlms
cause remains unverified; inspect the persisted Windows event log on a later boot.

INFERRED hardware handoff: use a fresh overlay with compatible swtpm/libtpms, first
verify exact seed GPU UUID after per-instance repair, then qualify each optional
work path separately. Collect UIA before/during/after, owned process/launcher links,
locale/DPI, native saved artifacts and freshness metadata. Never dump credentials,
authentication controls, proprietary binary internals, or the whole desktop UIA tree.
No disassembly/decompilation or output derived from it is used or stored here.

DOC READ sources (official; consulted in the app lane on 2026-11-13):

- [SolidWorks SaveAs API](https://help.solidworks.com/2026/English/api/sldworksapi/SOLIDWORKS.Interop.sldworks~SOLIDWORKS.Interop.sldworks.IModelDocExtension~SaveAs.html).
- [eDrawings API](https://help.solidworks.com/2026/english/eDrawings/t_Application_Programming_Interface.htm).
- [UL edition features](https://support.benchmarks.ul.com/support/solutions/articles/44002132667-3dmark-features-by-edition),
  [CLI guide](https://support.benchmarks.ul.com/support/solutions/articles/44002145411-run-3dmark-benchmarks-from-the-command-line).
- [Resolve reference manual](https://documents.blackmagicdesign.com/UserManuals/DaVinciResolveReferenceManual.pdf), External Scripting Using.
- [Superposition manual](https://benchmark.unigine.com/docs/superposition/Superposition_Benchmark_User_Manual.pdf), Automation (Pro only).
- [Vendor benchmark page](https://store.steampowered.com/app/3132990/).
- [Microsoft Edge automation](https://learn.microsoft.com/en-us/microsoft-edge/test-and-automation/test-and-automation),
  [UIA InvokePattern](https://learn.microsoft.com/en-us/dotnet/framework/ui-automation/invoke-a-control-using-ui-automation).

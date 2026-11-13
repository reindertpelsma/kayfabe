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
Fresh/CI runs require a new empty work directory; manual --resume reuses a fully
stopped run, as described under Clock below. Started processes are cleaned through an
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

## Clock

CODE READ (2026-11-13): a fresh overlay uses the frozen reference from the verified
image state: last_time_at_stop_utc + gap_s. CI behavior stays fresh by default.
Manual `--resume` reuses that work directory's existing overlay and TPM/firmware;
it refuses --zip and validates the backing image before startup. Fully stop the
previous run and its helpers first; this option does not attach to a running VM.
Stale owned socket/FIFO paths are removed after the overlay preflight.

CODE READ: `run/clone.qcow2.clock` is an atomically replaced JSON sidecar with
base_utc (last selected RTC base), last_guest_utc (recorded guest UTC high-water
mark, initially null), and boots (recorded start attempts). Startup uses
max(frozen reference, prior selected base, last_guest_utc + 60 seconds when known).
Before clean shutdown, SSH reads `(Get-Date).ToUniversalTime()` and updates the
sidecar without lowering any known timestamp. A failed read/write is visible as
clock-save WARN and does not prevent shutdown. No host wall clock is substituted.

CODE READ: only the working state copy's last_time_at_stop_utc is adjusted so its
normal gap produces the selected RTC base. Both QEMU argv and identity comparison
use that copy, retaining the existing base <= guest UTC <= base+1800 seconds
verification window for resumed overlays. The immutable image state is unchanged.
An in-process Windows reboot continues to use the running QEMU RTC (clock=vm);
sidecar advancement happens only when this runner starts/stops QEMU.

CODE READ: a corrupt sidecar or an unexpectedly missing sidecar on --resume emits
a warning and falls back to the frozen reference, as the owner requested. Normal
absence on a fresh clone is expected. INFERRED limitation: fallback cannot recover
lost disk history, and an abrupt stop cannot supply a clean-shutdown guest sample;
nondecreasing guarantees apply to valid recorded timestamps, not unknown history.
Resume/clock behavior has GPU-free fixtures and mutants, not a real guest proof.

## Deviations from a default install

DESIGN INTENT (owner, 2026-11-13): the image needs OpenSSH plus its public key,
the NVIDIA driver, and autologon baked in, alongside the licensed applications.
Other harness repairs belong at run time. CODE READ: the inherited #357 answer file
still carries the additional choices below; this inventory neither removes them
nor claims that a newly minimal image has been captured.

| Deviation / exact choices | Reason | Baked versus per-boot / evidence |
|---|---|---|
| OpenSSH Client/Server MSI; sshd Automatic; inbound TCP 22 rule; public key in administrators_authorized_keys with Administrators/SYSTEM ACL; DefaultShell=Windows PowerShell | Remote harness transport and consistent script execution | CODE READ: `propimage.py` render_setup_ssh. Required baked prerequisite; section 11 of the proprietary-image design records the predecessor DefaultShell change (MEASURED, 2026-11-12). No private key is sent by that setup script. |
| Local administrator account and AutoLogon enabled with LogonCount=1; hidden online/wireless/OEM/local-account OOBE screens; ProtectYourPC=3 | Offline account creation and an interactive desktop for app checks | CODE READ: answer-file oobeSystem. Autologon is required baked; the single initial logon is not proof that every restored boot will autologon. INFERRED: persistent autologon still needs image verification; power_fix never reads or modifies its credential values. |
| en-US input/system/UI/user locale, UTC, fixed computer name, Windows 11 Pro image index 6; wipe disk 0 and create EFI 300 MiB/MSR 16 MiB/remaining NTFS C: | Deterministic unattended setup and disk/edition selection | CODE READ: answer file plus render_unattend. Existing installation choices, not per-boot repairs; the host-derived VM ids and clock are separate host configuration. |
| WinPE viostor plus staged NetKVM/vioserial/viorng/pvpanic/fwcfg/smbus; no Balloon/viomem | Virtio storage/network/devices and no guest-directed RAM resizing | CODE READ: render_unattend and answer file. Existing baked driver prerequisites; no new installation in the check step. |
| BypassCPUCheck=1 only; Secure Boot/TPM/RAM checks are not bypassed | Permit the fixed CPU profile used by the harness | CODE READ: windowsPE. Existing setup-only deviation, not changed by this lane. |
| PreventDeviceEncryption=true plus BitLocker PreventDeviceEncryption=1 | Owner's VM encryption policy and reproducible test disk | CODE READ: specialize component and registry. Existing baked choice; power_fix does not disable/decrypt BitLocker. |
| DynamicUpdate disabled; NoAutoUpdate=1; ExcludeWUDriversInQualityUpdate=1; SearchOrderConfig=0 | Avoid setup/update traffic and unexpected driver replacement | CODE READ: windowsPE/specialize. Existing baked policy, not a per-boot rewrite. |
| EnableVirtualizationBasedSecurity=0; HypervisorEnforcedCodeIntegrity Enabled=0 | Avoid nested security virtualization in the harness | CODE READ: specialize. Existing baked policy; actual running security state is not measured by this lane. |
| CrashDumpEnabled=2 and DisplayParameters=1 | Kernel dumps and visible bugcheck diagnostics | CODE READ: specialize. Existing baked diagnostics. |
| VerifiedAndReputablePolicyState=0 | Allow the harness's unsigned CUDA probes | CODE READ: specialize Smart App Control policy. Existing baked deviation. |
| NVIDIA EnableGpuFirmware=1 in nvlddmkm service and each display class instance | GSP initialization; prevent the new-instance Code 43 finding | MEASURED predecessor: proprietary-image design section 11 (2026-11-12) and 4090 evidence README (2026-11-13). CODE READ: existing driver setup; gpu_fix repairs display instances per boot. |
| DWM OverlayTestMode=5 | Predecessor workaround for Start-menu display-driver resets | MEASURED predecessor: proprietary-image design sections 10–11, 2026-11-12. Existing manual image change; not changed by power_fix and not newly qualified here. |
| HiberbootEnabled=0; hibernation disabled; sleep/unattended-sleep/hybrid-sleep/hibernate/monitor/disk timeouts disabled on AC and DC | Avoid suspended/restored Windows state meeting a fresh kf3, and keep unattended checks awake/visible | CODE READ: Fast Startup is already baked by specialize; power_fix now repairs and independently reads back all listed power values every boot. No current hardware measurement. |
| Wake sign-in disabled on AC/DC; lock-screen policy disabled; machine inactivity limit 0; screensaver active/secure/timeout 0 in user settings and user policy, plus live interactive-session apply/readback | Keep the autologon desktop available for app checks | CODE READ: per-boot power_fix; no wake/timeout repair existed in the reviewed #357 generator. This does not set autologon credentials or defeat an already locked desktop. |
| Temporary per-boot InactivityRestartBoot registry marker; runtime scripts/results under C:\kfhw and C:\kfapps | Preserve a required-restart warning across repeated repairs within the same boot, and collect evidence in the disposable overlay | CODE READ: per-boot harness state only, not baked. A later actual boot clears the warning by boot-identity comparison; the check never restarts Windows itself. |

CODE READ sources for this inventory: `scripts/bench/windows/propimage.py`, its
`propimage_autounattend.xml.in` template, and
`docs/design/V3_WINDOWS_PROPRIETARY_IMAGE.md` sections 1, 10 and 11. The latter's
phrase “only deviation” is not treated as evidence that the other listed tweaks
are absent. The #357 sources remain unchanged apart from the previously authorised
single fixture-literal edit; image minimisation is for their owning lane.

CODE READ: after SSH and desktop readiness, power_fix applies the active scheme
without selecting a different performance plan. It uses powercfg /hibernate off,
sets both AC and DC indices, reactivates that same scheme, then reads powercfg /q
plus the active scheme's explicit AC/DC registry indices (also covering hidden
settings without depending on localised query labels).
HibernateEnabled is read, never forged by a direct registry write. The hibernation
file must be absent. Native failures, missing values, wrong user/session, mismatches
or a pending policy restart produce `power: NOT OK`, with values in the result table
and the complete queries/readback in `power.json`; app work is blocked. Success is
`power: OK`. The interactive screensaver setter/readback runs through a temporary
scheduled task for the same SSH user and is removed afterwards.

INFERRED falsifier for power qualification: any nonzero required AC/DC value,
remaining hibernation/Fast Startup, active/secure screensaver, wrong desktop user,
missing query, failed command or pending inactivity-policy restart earns OK. The
offline fixtures/mutants reject these; real power behavior remains unverified.
DOC READ: Microsoft documents that a changed machine inactivity limit can require
a restart. The per-boot marker therefore keeps NOT OK visible on repeat calls in
that boot, even after the registry has become zero. An already sleeping/locked
session can also reject the live screensaver change; no success is invented.

DOC READ references (official, consulted 2026-11-13):

- [Powercfg options and aliases](https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/powercfg-command-line-options).
- [Screensaver/lock-screen policy registry mappings](https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-admx-controlpaneldisplay).
- [Machine inactivity limit and restart requirement](https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/security-policy-settings/interactive-logon-machine-inactivity-limit).
- [Live screensaver setters/readback](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfow).
- [Microsoft lab unattended sleep timeout](https://hobl.microsoft.com/docs/HOBL_Prep.html).

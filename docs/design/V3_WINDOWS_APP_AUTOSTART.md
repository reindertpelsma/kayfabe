STATUS: DESIGN-ONLY, 2026-11-07. GPU-free tooling with a fake Windows provider;
real Windows APIs, reboot persistence and application compatibility are UNTESTED.

# Offline image application autostart policy

DESIGN INTENT (owner/coordinator, 2026-11-07): before final image capture,
disable background autostart so another application's helpers cannot contaminate
a test. The primary build is native Windows-on-Vast with Alpine ESP capture;
the QEMU session tooling remains an alternative. Packaging, machine-state,
manifest and upload contracts are unchanged. No hardware or storage operations
are part of this change.

DESIGN INTENT: take a footprint around each manual, official installer, including
the stock driver installer. Keep these scripts as build/run-time tools rather
than baking harness scripts into the image. Keep the resulting footprints and
lab report under `C:\ProgramData\KayfabeLab` and retain private sidecar copies
next to the image inventory. Commands, account names and task XML can reveal
installer data: use throwaway accounts, inspect the documents, never commit them
or publish them. The machine-state manifest's offline-material exception is not
permission to collect personal account tokens in task arguments.

## Footprint contract

CODE READ (`scripts/bench/windows/app_footprint.ps1` and
`app_autostart_common.ps1`): `snapshot` writes a version-1
`autostart_snapshot` with machine UUID, diagnostic UTC timestamp and four entry
arrays. `diff` requires snapshots from the same machine and writes a version-1
`app_footprint` with `app_id`, `installer_sha256`, both snapshot SHA-256 values,
`created_utc`, `entries` and `changed_existing`. Both commands refuse to overwrite
an output. IDs use lowercase letters, digits and underscores. Snapshot timestamps
do not establish ordering because the offline RTC can move backward; the
operator supplies before/after paths explicitly.

| Entry array | Recorded values |
|---|---|
| `services` | Name, display name, binary path, start type, account, driver/process type, dependencies, delayed-auto flag, current status |
| `tasks` | Task path/name, exported XML, enabled flag, status, action executable paths |
| `run` | Registry key, value name, original value and registry value kind |
| `startup` | Full path, bytes, SHA-256 and shortcut target |

CODE READ: ownership includes only new names/keys/paths. Modifications of an
existing entry appear in `changed_existing`, not as a new app-owned service.
Shared prerequisites can be restored through the selected service's dependency
closure recorded by lab prep. Installers that modify an existing service without
a dependency edge need manual review; the tooling does not guess ownership.
Removed entries are not resurrected. A driver or shared system component never
becomes mutable merely because it appears in an app footprint.

CODE READ: collectors enumerate `Win32_Service`, `Win32_SystemDriver`, scheduled
tasks, HKLM and loaded ordinary-user HKU Run/RunOnce keys in native/WOW6432Node
locations, common Startup and those profiles' configured Startup folders. An
unloaded ordinary profile refuses the operation before mutations. Startup
directories/reparse items, unresolved/UNC Startup roots, unsupported service/task
names and inaccessible APIs refuse the operation. No registry hive is loaded
automatically. Default-profile templates, future accounts, browser extensions,
WMI event consumers, shell extensions, arbitrary executable installers and
interactive processes outside these autostart surfaces are outside this schema.
Use the fixed disposable local account; additional accounts require a new review.

## Capture preparation and protection floor

CODE READ (`lab_prep.ps1`): validate all footprint files and pin their hashes,
then sweep the live catalog as well as app metadata. This includes services and
tasks missing from every installer diff, such as pre-existing updaters. Disable
all unprotected services with `Disabled` and stop them in reverse dependency
order; `Manual` alone would allow trigger starts. Disable/stop unprotected tasks,
remove literal Run/RunOnce values, and move startup files to a quarantine outside
Startup. The report retains original modes, delayed-auto flags, task XML,
registry values, startup hashes, quarantine destinations and attempted actions.

CODE READ: the protected service floor includes NVIDIA display containers,
all kernel/filesystem drivers (including NVIDIA and virtio), Windows Installer,
sshd, QEMU guest agent, TPM and crypto/key services. It also keeps boot/RPC/PnP,
event logging, scheduler infrastructure, WMI, user profiles, networking/firewall,
Windows security, licensing, desktop and audio essentials. The exact list is
`$script:KfServiceFloor`; live dependency closure is protected too. TPM/crypto,
certificate, Defender, scheduler and PnP task paths, `KF_`/`KF-` harness tasks,
and autostart actions directly naming sshd/QGA/NVIDIA container executables are
protected. Primitive mutation functions check protection again. The report
lists protected services so the coordinator can inspect exceptions explicitly.

DESIGN INTENT: protected infrastructure is an explicit exception to the owner's
background-app policy. Do not expand the allow-list just to make a capture work.
A protected dependency unexpectedly retaining an app updater falsifies the
isolation hypothesis and blocks capture for owner review. Protection wins over
the sweep; the tool does not stop required infrastructure to satisfy a count.

CODE READ: Update/servicing helpers (`wuauserv`, `UsoSvc`, `WaaSMedicSvc`, BITS,
DoSvc), telemetry (`DiagTrack`, `dmwappushservice`), Search (`WSearch`) and time
sync (`W32Time`) are subject to the global sweep. They are additionally forbidden
prestart dependencies. App updater services/tasks receive the same sweep without
depending on vendor-name guesses. A selected app can restore its own ordinary
services; after that test the reset disables them again.

CODE READ: a final live audit requires every unprotected service to be Disabled
and Stopped, every unprotected task disabled and neither Running nor Queued, and
no remaining unprotected Run/startup entries. Only then is `complete: true`
published. Failures produce a nonzero exit, `LAB_PREP_FAIL reason=<policy-code>`
and an incomplete report. An attempted-action row is not a success receipt.
Access denied, Medic/tamper-protection refusal or profile/Startup incompatibility
must be investigated; the scripts do not take ownership, bypass protection or
silently accept a partial cleanup. A failed cleanup does not authorize capture.

## Per-app prestart and teardown

CODE READ (`app_prestart.ps1`): `--app ID` / `-App ID` selects a pinned footprint
and complete same-machine lab report. It checks the footprint hash, installer
hash and current service identity (including binary, account and dependencies)
before changes. A global reset first removes any previous app's enabled state.
The selected app's services and required unprotected dependencies regain their
recorded modes/delayed-auto flags and start in dependency order. An originally
Disabled service is refused rather than silently enabled. Protected services
are untouched; a required protected dependency must already be running.

CODE READ: the shared service wait deadline is 1–120 seconds, using
`ServiceController.WaitForStatus`; the host guest-channel call also has a finite
timeout. A failure prints `APP_PRESTART_FAIL app=ID action=Start reason=CODE`.
Only exit zero plus the exact `APP_PRESTART_DONE` line permits the workload.
The deadline does not prove a helper has finished internal initialization;
application-specific readiness remains a coordinator experiment.

CODE READ: tasks stay disabled by default. Explicit task IDs must belong to the
selected footprint; task XML is parsed with DTDs prohibited and no resolver.
Credential-based logon modes are refused. An explicit task is re-registered
enabled with all automatic triggers removed and remains ready for manual app
invocation; it is not automatically started. Run/RunOnce and startup shortcuts
are never restored by prestart. Apps relying on those mechanisms need an
explicit supported test-time launch recipe rather than background autostart.

CODE READ (`scripts/bench/windows/appmatrix/prestart.py`, `winapps.py`,
`gen_apps.py`): an app may opt in using
`"prestart": {"timeout_s": 30, "tasks": []}`. Unknown fields, invalid IDs,
invalid/duplicate task paths and unbounded timeouts are refused. Apps without
the field keep their existing execution path; no stock app is implicitly opted
in. Guest scripts are pushed at run time. Private footprint/report data must be
present in the image at the default ProgramData paths. The wrapper resets in a
`finally` block after every opted-in app. A failed reset records
`APP_LAB_RESET_FAIL ... fresh_guest=required` and requests a fresh guest before
the next workload. A missing or changed footprint also blocks cleanup: retain
these records immutably with the image.

## Coordinator falsifiers and Windows-only dry run

DESIGN INTENT: on a disposable Windows guest, take before/after snapshots around
two official installers with distinct helpers. Inspect changed-existing rows,
loaded profiles, 32/64-bit keys, task XML and all protected exceptions. Run prep
elevated, require a complete report, reboot, and rerun the catalog audit before
final shutdown/capture. Never rerun prep over a previous report: it would replace
original service modes with Disabled. Preserve the first report; after reboot,
use `app_prestart.ps1 -App ID -Action Reset` for a fresh audit/reset.

DESIGN INTENT: falsifier for isolation is any unprotected helper from app A
starting during app B, including after reboot and after task triggering. Test
both app orderings and inspect running processes and scheduled-task events.
Falsifier for protection is any action against a driver, protected service/task
or required dependency, or a lost SSH/guest-agent channel. Stop and report either
condition. Close interactive apps and writers before capture; disabling future
autostart does not terminate every existing interactive process.

UNTESTED: real Windows PowerShell 5.1 compatibility; CIM service/driver values;
registry/profile collection; localized task/service names; Windows Update/Medic
permissions; task registration/trigger stripping; SCM wait behavior; delayed
auto-start; protected dependency completeness; quarantine ACLs/paths; NVIDIA,
virtio, TPM, SSH and QGA continuity; application readiness/licensing; reboot and
capture persistence. Linux PowerShell fixtures exercise policy orchestration
only. The Windows provider source contracts cannot establish these behaviors.

DESIGN INTENT: local checks are `python3 scripts/ci/test_app_autostart.py`,
`python3 -m unittest discover -s scripts/bench/windows/appmatrix/tests` and
`python3 scripts/ci/app_autostart_mutants.py`. The dedicated GPU-free workflow
requires PowerShell rather than treating missing fixture execution as evidence.
Mutations must have at least two independently named failing fixture tests.

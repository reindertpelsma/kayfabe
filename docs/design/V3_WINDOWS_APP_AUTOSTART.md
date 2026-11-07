STATUS: DESIGN-ONLY, 2026-11-07. Owner correction: ordinary app services use
Manual demand start. Windows behavior, reboot persistence and app acceptance are UNTESTED.

# Offline image application autostart policy

DESIGN INTENT (owner/coordinator correction, 2026-11-07): this replaces the
initial proposal to disable all app services/tasks. Convert Automatic and
Delayed-Automatic ordinary services to **Manual**, so an administrator-launched
app can request its helper with StartService. Do not write Disabled for an
ordinary service. Disable only exact service/task IDs on the explicit
updater/telemetry policy; preserve ordinary scheduled tasks. Remove unprotected
Run/RunOnce values and startup-folder entries. Native Windows-on-Vast remains
the primary capture path; QEMU remains an alternative. Packaging and machine
state tooling are unchanged.

## Installer footprint and rollback data

CODE READ (`app_footprint.ps1`, `app_autostart_common.ps1`): snapshot/diff remains
version 1. Capture before/after every manual official installer. Each per-app
JSON carries its app ID, installer SHA-256, both snapshot hashes, machine UUID,
new services/tasks/Run/startup entries and changed-existing audit rows. Only new
names/keys/paths establish ownership; changed-existing rows require review.

| Surface | Rollback/inventory values |
|---|---|
| Services | Name, display name, binary path, previous start type, account, driver/process type, dependencies, delayed-auto flag and status |
| Tasks | Path/name, XML, previous enabled flag, status and action executable paths |
| Run/RunOnce | Literal key/value name, previous value and registry value kind |
| Startup folders | Path, bytes, SHA-256 and shortcut target; quarantine destination in the prep action report |

CODE READ: the prep report deep-copies every affected service's original mode
before mutation, including Manual/Disabled modes and the delayed-auto flag.
Task XML/enabled flags and removed startup data remain available for rollback.
Prestart is not a rollback operation: it never restores Automatic startup or
re-enables an updater. A general rollback command is not implemented; preserve
the original report and use a reviewed rollback procedure or a pristine build.
Do not overwrite an existing complete or failed report.

DESIGN INTENT: keep these records private under
`C:\ProgramData\KayfabeLab`, with sidecar copies next to the private image
inventory. Records are data; build/harness scripts are staged at run time rather
than baked into the image. Inspect task arguments, account names and registry
values for personal data. Use disposable accounts. The offline machine-state
exception does not authorize collecting personal credentials in these records.

## Explicit updater/telemetry policy

CODE READ: `lab_prep.ps1 -PolicyPath FILE` accepts this version-1 schema. Lists
contain exact IDs, not patterns, commands or inferred vendor ownership:

```json
{
  "schema_version": 1,
  "kind": "autostart_policy",
  "services": [],
  "tasks": []
}
```

DESIGN INTENT: populate vendor updater IDs by reviewing the actual footprint,
installer documentation and acceptance behavior. Empty vendor lists do not
classify a helper as an updater. An unfamiliar service with Update in its name
still becomes Manual unless explicitly listed. Keep app-owned workload tasks
enabled; classify vendor update tasks individually.

CODE READ: built-in exact service IDs are `wuauserv`, `UsoSvc`, `WaaSMedicSvc`,
BITS, DoSvc, DiagTrack, dmwappushservice and WSearch. Built-in exact task IDs
cover the authored Windows Update/UpdateOrchestrator scan/reboot entries,
Compatibility Appraiser, CEIP Consolidator/UsbCeip and IndexerAutomaticMaintenance;
the complete IDs are `$script:KfUpdaterTasks`. There are no folder wildcards.
Vendor entries extend these defaults. The resolved policy is stored in the
version-2 lab report and used by prestart. Version-1 prep reports from the earlier
initial blanket Disabled proposal are refused; rebuild from pristine state or review
the original rollback data before preparing again. Footprint schema stays at 1.
Windows time-service shutdown remains the separate image-preparation contract.

CODE READ: prep converts ordinary Automatic (including delayed-auto) services to
Manual, leaves existing Manual/Disabled types alone, and stops unprotected
running services in reverse dependency order before capture. Only policy-listed
services receive Disabled. Only policy-listed tasks are disabled/stopped.
Unprotected Run/RunOnce entries are deleted literally and startup files are
quarantined. A final live audit checks resulting modes, stopped services,
listed tasks and startup removal before publishing `complete: true`. No-op or
failed providers produce an incomplete report and a nonzero exit.

## Protected infrastructure

CODE READ: the protection floor covers NVIDIA display containers, every
kernel/filesystem driver (including NVIDIA/virtio), Windows Installer, SSH/QGA,
TPM/crypto/key services and boot/RPC/profile/network/security/licensing/desktop
infrastructure. Live dependencies are protected too. TPM/crypto/certificate,
Defender, scheduler/PnP tasks, harness tasks and direct SSH/QGA/NVIDIA container
autostart commands remain protected. An explicit policy naming a live protected
entry is rejected before mutations. Primitive writes check protection again;
the service primitive independently refuses Disabled for a non-updater.

DESIGN INTENT: inspect the report's protected list. A protected dependency that
unexpectedly retains an app updater falsifies the isolation design and blocks
capture for owner review. Access denied or Medic/tamper-protection refusal is a
failure, not permission to bypass protection. No ownership/ACL bypass is used.

CODE READ: collection includes services/drivers, scheduled tasks, HKLM and loaded
ordinary-user HKU native/WOW6432Node Run/RunOnce keys, and common/profile Startup
folders. An unloaded ordinary profile, inaccessible API, unsupported name,
Startup directory/reparse item or unresolved/UNC root refuses the operation.
Default-profile templates, future accounts, WMI consumers, shell extensions and
arbitrary installer-created launch mechanisms are outside the schema. Timestamps
are diagnostic because a fixed RTC can move backward across boots.

## Optional acceptance escape hatch

DESIGN INTENT: first run an app's acceptance test with ordinary demand start and
**without prestart**. Only an app whose acceptance test demonstrates that a
required helper did not start on demand should opt in. The harness runs as an
administrator; real StartService behavior still needs the Windows dry run.

CODE READ: `app_prestart.ps1 --app ID` validates the pinned footprint, same-machine
version-2 report and current service identity. It starts only that app's recorded
ordinary services and required unprotected dependencies, using a shared bounded
wait. Services must already be Manual; it does not change startup types, import
tasks, restore Run entries or reset unrelated apps. Policy-listed updater
services cannot be selected, even through a dependency. `-Action Reset` stops
only this selected service closure. Failures print an app/action/policy-code
line and return nonzero.

CODE READ: the app-matrix descriptor is optional
`"prestart": {"timeout_s": 30}` (1–120 seconds). Task overrides and unknown fields
are refused. No stock app opts in. The wrapper requires exit zero plus the exact
success marker, and runs selected-service teardown in a finally block. Failed
teardown requests a fresh guest. Scripts are pushed at run time; private records
must already be at their fixed ProgramData paths.

DESIGN INTENT: demand-start services can outlive an app. For isolation acceptance,
use a pristine guest per app. Guest reuse requires app-specific cleanup
acceptance evidence for its helpers. The optional prestart hook is not a universal service-cleanup
mechanism, and Manual does not suppress Windows service triggers. Do not claim
cross-app isolation merely from the startup-type conversion.

## Coordinator dry run and falsifiers

DESIGN INTENT: on a disposable Windows guest, capture footprints around two
installers, explicitly classify update tasks/services, prepare elevated, inspect
all previous modes/protected exceptions, reboot and inspect actual startup.
Test ordinary demand start without prestart; only then justify an escape hatch.
Use a fresh guest per app for the first acceptance experiments. Falsifiers are
any Disabled write to an ordinary service, any ordinary task changed by prep,
any protected mutation, a missing previous mode, an updater starting, or a
required helper failing demand start. Stop capture and report the finding.

UNTESTED: Windows PowerShell 5.1, CIM/SCM/registry/task APIs, Update/Medic
permissions, delayed-auto behavior, startup triggers, protected dependencies,
NVIDIA/virtio/TPM/SSH/QGA continuity, quarantine ACLs, reboot persistence,
application readiness/licensing and demand-start acceptance. Linux PowerShell
fixtures exercise policy only; source contracts do not establish Windows behavior.

DESIGN INTENT: run `python3 scripts/ci/test_app_autostart.py`, the app-matrix
fixtures, and `python3 scripts/ci/app_autostart_mutants.py`. Each mutant must have
two independently named killers. The GPU-free workflow requires PowerShell.

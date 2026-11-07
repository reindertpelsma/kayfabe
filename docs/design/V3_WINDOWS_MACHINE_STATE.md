STATUS: DESIGN-ONLY, 2026-11-07. Offline fixture tooling; no Windows boot,
TPM service, GPU, image capture, R2 transfer or deployment was exercised here.

# Offline Windows image machine state

DESIGN INTENT (owner decision, coordinator 2026-11-07): store
`MACHINE_STATE.json` beside the pristine image and packaging manifest in private
R2. **NON-SENSITIVE-BY-DESIGN** applies to this offline image's disposable guest
material: its generated TPM state, guest SSH key, throwaway Windows password,
OVMF variables and generated GOP signing key. It does not authorize collecting
personal credentials, host SSH keys, provider tokens, vendor signing keys or
ordinary production TPM state. Keep image and state out of git, public releases,
logs and shared caches. No R2 integration is deployed by this design.

DESIGN INTENT: this dated owner exception supersedes the earlier image proposal's
fresh-TPM rule for this offline image only. The ordinary per-VM rules still apply
outside that scope. The primary image build remains native Windows on Vast with
the proposed Alpine UEFI ESP capture. The QEMU session wrapper remains an
alternative; packaging/manifest/upload scripts retain their existing contract.
Native capture must supply compatible firmware/TPM material before this QEMU
restore path can be attempted. That conversion is UNTESTED, not implemented here.

## Schema and boundaries

CODE READ (`scripts/bench/windows/machine_state.py`): schema version 1 has these
root fields. Nested objects reject unknown fields too; duplicate JSON keys are
rejected at every depth. Base64 blobs carry both their byte count and SHA-256.
CODE READ: `verify --image` also compares image bytes; omitting `--image`
checks the schema and embedded blobs only. Apply always checks the image bytes.

| Field | Meaning |
|---|---|
| `schema_version`, `classification`, `created_utc` | Integer 1, explicit offline classification, real UTC capture metadata |
| `image_sha256` | SHA-256 of the exact packaged image bytes, not the uncompressed virtual disk contents |
| `capture_window` | Explicit `start_utc` and `end_utc` supplied by the capture operator |
| `rtc_base_utc`, `time_service_disabled` | One UTC datetime inside that window; inventory must record time service disabled |
| `smbios` | Canonical `uuid`, `serial`, `manufacturer`, `product` |
| `nics` | One to eight unique unicast `mac` addresses |
| `disk` | `serial` and QEMU device `model` (`virtio-blk-pci` in version 1) |
| `machine` | Versioned QEMU `type` (`pc-q35-X.Y`), `cores`, `ram_mb` |
| `monitor` | `edid_id`: current fixed KF3 ID `KFB-0001-00000001` |
| `cpu_observed` | Diagnostic capture-host `brand`, `family`, `model`, `stepping`, sorted `flags` |
| `cpu_pin` | OPTIONAL object containing only `brand`; absent in ordinary capture output |
| `files` | Allowlisted relative paths to `{bytes, sha256, data_base64}` objects |

CODE READ: `cpu_observed` comes from the first `/proc/cpuinfo` processor record
(or an explicit fixture file). It is not a promise about guest-visible ISA and
never selects a CPU model. Without `cpu_pin`, the existing A/B `-cpu` arguments
remain unchanged. With `cpu_pin`, the launcher appends only `model-id=<brand>`
to that existing argument. Feature arrays, CPU model names, unknown pin keys and
comma/property injection are refused. Override brands are limited to 48 ASCII
bytes and virtio disk serials to 20 bytes to avoid silent identity truncation.
The legacy `-vmx` setting is unchanged;
the snapshot adds no feature masks. Brand spoofing across hosts is UNTESTED.

CODE READ: SMBIOS strings may contain literal commas; the JSON retains the raw
name and the launcher bridge doubles those commas for QEMU's option parser.
CPU brands still reject commas, and all property strings reject `=` and control
characters. Source: QEMU 10.2's [SMBIOS option dispatch](https://github.com/qemu/qemu/blob/v10.2.0/system/vl.c#L3191-L3197)
and [option-value parser](https://github.com/qemu/qemu/blob/v10.2.0/util/qemu-option.c#L51-L81).
UNTESTED falsifier: a restored Windows SMBIOS name loses a comma or becomes an
additional QEMU property. The coordinator's identity comparison must catch it.

CODE READ: the monitor field accepts only the current authored KF3 EDID identity
(`crates/kf-disp/src/edid.rs`, manufacturer KFB/product 1/serial 1). There is no
new arbitrary EDID override. Plain std VGA in the alternative installer mode
does not establish this KF3 monitor identity. The disk model is the QEMU device
model, not an invented Windows WMI disk description.

CODE READ: required files are `tpm/tpm2-00.permall`,
`firmware/OVMF_VARS.fd`, `secrets/ssh_ed25519`, `secrets/ssh_ed25519.pub`, and
`secureboot/db.key`, `secureboot/db.crt`, `secureboot/db.guid`. Optional files are
`tpm/tpm2-00.savestate`, `tpm/tpm2-00.volatilestate`, `secrets/win_password`.
Capture searches the VM's secureboot directory first, then its parent's
secureboot directory, or an explicit `--secureboot-dir`. It does not recursively
archive secrets. Symlinks, missing required files and oversized blobs are refused.
The apply path restores the signing material under the working VM and directs
GOP signing there so its certificate can match the captured OVMF db.

## Operator workflow (UNTESTED design)

DESIGN INTENT: first finish manual installation and guest preparation, disable
hibernation and Windows time service, obtain a clean shutdown and package the
disk through the existing workflow. Record the actual guest identities before
shutdown in an operator facts JSON containing exactly `smbios`, `nics`, `disk`,
`machine`, `monitor` with the nested fields above. Capture does not fabricate
these values or discover physical hardware identifiers from a powered-off disk.
Falsifier: the supplied identifiers differ from guest evidence on the first
restore, or the native image lacks compatible QEMU TPM/firmware state.

DESIGN INTENT: apply takes the standalone packaged qcow2 expected by the
existing launcher. A native raw capture needs conversion and a state sidecar
bound to the derived artifact's SHA-256; that conversion remains outside this
tool. CODE READ: apply hashes and copies opaque bytes to `disk/base.qcow2`;
it does not detect the input format or convert it. Input format is an operator
precondition, established by the separate packaging workflow.

```sh
# All paths below are operator placeholders outside the checkout.
python3 scripts/bench/windows/machine_state.py capture \
  --vm /private/build/vm --image /private/release/windows.qcow2 \
  --facts /private/build/facts.json \
  --window-start 2026-11-07T10:00:00Z --window-end 2026-11-07T11:00:00Z \
  --rtc-base 2026-11-07T10:30:00Z \
  --output /private/release/MACHINE_STATE.json
python3 scripts/bench/windows/machine_state.py verify \
  /private/release/MACHINE_STATE.json --image /private/release/windows.qcow2
python3 scripts/bench/windows/machine_state.py apply \
  /private/release/MACHINE_STATE.json --image /private/release/windows.qcow2 \
  --vm /private/runs/run-one
WINVM_ROOT=/private/runs WINVM_NAME=run-one \
  WINVM_MACHINE_STATE=/private/release/MACHINE_STATE.json \
  bash scripts/bench/windows/win_vm.sh run --overlay attempt-001
```

DESIGN INTENT: use the real capture window, not the example's date. Supply
`--password-file` with raw password bytes (not the session env-file) if the
owner's session changed the account password: the old host password file may
be stale. Capture requires a VM-side
`image-inventory.json` recording `time_service_disabled: true`. That inventory
assertion and caller-supplied capture window are evidence inputs, not an offline
proof that Windows actually obeys them. Do not add `--cpu-brand` by default;
it exists only for an explicit brand-only override.

CODE READ: initial apply requires a new VM directory, copies the exact base
image and state, and creates an installed marker. The canonical JSON stays
outside the working VM. Before each logical `run`, under its VM lock, the
opt-in hook validates the manifest/image and managed-state digest, restores
fresh TPM/OVMF/key bytes, clears unlisted regular TPM files and optional state
files absent from the snapshot (unexpected directories/symlinks are refused),
and requires an unused overlay name. It never updates the pristine manifest.
An applied directory refuses `run` without the opt-in. Restricted user-mode
networking is required. Guest-requested reboots within one logical run retain
that run's state and overlay; the next logical run resets again. Use a new VM
directory or new overlay name for the next attempt; no existing overlay is deleted.

CODE READ: snapshot runs refuse `WINVM_OVERLAY_BACKING`; a different parent disk
would escape the image hash binding. Alternate backing remains available when
the snapshot opt-in is off. `WINVM_APPS_ISO` remains available as a separate
runtime layer, including with the snapshot opt-in; it does not become part of
the pristine machine state.

DESIGN INTENT: retain an independently trusted image/state hash pair in the
release record. SHA-256 detects accidental substitution against that record;
it is not a signature authenticating an untrusted JSON download. Uploading this
additional sidecar is a coordinator task: the existing upload command is not
extended to publish machine state automatically.

## Coordinator dry run (all UNTESTED)

1. Restore twice from the same trusted image/state pair. Read SMBIOS, disk/NIC
   identifiers, RTC and KF3 EDID inside Windows. Falsifier: any requested field
   differs, or time synchronization escapes the fixed capture window.
2. Change guest TPM NV state and firmware variables in the first disposable run;
   after shutdown start the second with a fresh overlay. Falsifier: the second
   run inherits those changes or pristine image/state bytes change.
3. Check Secure Boot and GOP signature binding and guest SSH/password access.
   Falsifier: the captured certificate and restored key disagree, the password
   input was stale, or driver initialization depends on uncaptured state.
4. Compare default CPU argv on two hosts with different ISA capability sets;
   attempt a brand-only override separately. Falsifier: observational flags mask
   host features, a default model becomes pinned, or a brand-only override changes
   the legacy feature switches. Application identity/activation continuity is
   UNTESTED and is not promised by the schema.
5. Verify lawful licences and evaluation expiry on real UTC independently.
   A fixed guest clock does not extend licensing rights or establish a valid
   certificate chain. No real-world reproducibility claim precedes this dry run.

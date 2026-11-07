# Alternative: build a private Windows image in QEMU

The owner chose native Windows-on-Vast with an Alpine UEFI ESP swap for the real
image capture. These helpers describe the alternative QEMU path; native capture
is maintained separately. The image-build helpers are experimental and have only been checked with mocks.
Use them on an authorized Linux builder with the existing Windows VM prerequisites,
noVNC and websockify installed. A real Windows capture and reuse still need testing.

Choose a fresh VM name and keep its state outside the checkout:

```bash
export WINVM_ROOT=/workspace/winvm
export WINVM_NAME=owner-image-build
export CARGO_TARGET_DIR=/workspace/image-build-target
```

Create an owner-only file outside the checkout with one literal line,
`WIN_IMAGE_PASSWORD=<disposable password>`. Use an editor so the password does
not enter shell history. Set its permissions to `0600`. The file is data: do not
add shell quotes or `export`.

```bash
scripts/bench/windows/image_session.sh install \
  --env-file /workspace/image-password.env \
  --ssh-host root@builder.example.invalid --host-ssh-port 22
```

For an already installed VM, use `start` in place of `install`. The wrapper starts
an `image-build` overlay, sets the local Windows `kf` account password over SSH
stdin and prints the builder's websockify command and the workstation's SSH
forwarding command. Run those commands, open the printed local noVNC URL, and
install applications that need no GPU through official installers. This mode uses
plain std VGA with no kf3 device; driver/GPU-dependent steps belong to the
UNTESTED broker mode below.
Use throwaway accounts only and record each installer's version, official URL,
full SHA-256, exact steps and licence notes. Keep SPEC, scripted open-app bundles
and the harness for run time.

If an official installer or activation needs outbound guest networking, add
`--allow-internet` to `install` or `start`. The default keeps the guest network
restricted. This changes only the launcher's `WINVM_NET_RESTRICT` knob; keep
offline and online first-run observations separate in your records.

For GPU-dependent steps, the broker/headless-compositor/browser-screencast route
is **UNTESTED DESIGN**. Prepare a reserved Xvfb/Openbox display with the owner's
browser-screencast, a Picom XRender compositor and the broker's X11 window, all
under the same user/display. The coordinator must test these services and remote
input before using them. The wrapper prints a proposed command sequence but
starts none of those services. It never offers live noVNC for a kf3 run.

After a clean stop of mode A, start the same overlay in mode B using the live
caller-owned broker socket:

```bash
scripts/bench/windows/image_session.sh start --resume --display-mode broker \
  --broker-socket /workspace/winvm/owner-image-build/run/display-broker.sock \
  --env-file /workspace/image-password.env --ssh-host root@builder.example.invalid
```

Keep WINVM_KF3_DISPLAY on: off removes the display. Use the printed port-6081
SSH tunnel and browser-screencast's private token URL. The UNTESTED fallback is
QMP screendump polling plus QMP keyboard/relative-pointer input, not a live video
console. If both viewing paths fail, stop and report the failed dry run. A later
clean stop can be followed by `start --resume --display-mode std-vga` to return to
A. Adapter switching and real Windows behavior still need testing.

Check the session with:

```bash
scripts/bench/windows/image_session.sh status
```

Complete the experimental [autostart preparation](#application-footprints-and-lab-preparation)
before final capture. Close applications and background writers. Create a JSON file selecting actual
application roots and mutable file exclusions; an empty root list produces
package/driver inventory only:

```json
{
  "app_roots": [],
  "exclude": []
}
```

Each exclusion has a full-path wildcard `pattern` and a `reason`. Then prepare
and stop the guest, and package it into a new directory outside the checkout:

```bash
scripts/bench/windows/image_session.sh stop --guest-config /workspace/image-roots.json
scripts/bench/windows/image_package.sh create --out /workspace/owner-image \
  --installers /workspace/manual-installers.json
```

`manual-installers.json` is your JSON checklist array; omit `--installers` if no
manual installer record is available, and complete it before relying on the
image. Preparation requires BitLocker off, disables hibernation/fast startup
and Windows time synchronization, collects inventory and zero-fills free space.
An owner-staged sdelete in `C:\kf\tools` is used if present; accept its EULA
interactively first. The built-in fallback leaves a recorded 64 MiB reserve.
Preparation scripts/configuration are removed before shutdown. Keep all generated
files private and review the inventory before upload.

Packaging requires a clean guest-shutdown event and an unchanged disk. A forced
stop does not qualify. If preparation fails, inspect the running guest, correct
the problem, and rerun `stop`. If clean-shutdown validation fails, inspect the
VM state and use the existing launcher to reboot the same `image-build` overlay
under the session's recorded tag before preparing/stopping it again. Keep the
same VM identity. Do not edit receipts or bypass guards. A new independent build
uses a fresh `WINVM_NAME`.

The output is `windows.qcow2` plus `manifest.json`. It contains no cloned TPM
state or host signing keys. Importing the disk into fresh per-VM firmware/TPM
state is not implemented by these helpers; coordinate that step before use.

For private transfer, put a caller-generated presigned PUT URL in an owner-only
file outside the checkout, without shell tracing. Use a separate presigned GET
for verification:

```bash
scripts/bench/windows/image_package.sh upload --package /workspace/owner-image \
  --url-file /workspace/image-put.url
scripts/bench/windows/image_package.sh verify --package /workspace/owner-image \
  --url-file /workspace/image-get.url
```

Uploads use single PUT with retries and support artifacts up to 5 GiB. Larger
images need caller-managed multipart upload; `verify` can still check their
presigned download. Retain the manifest/checksum independently. Never publish
the disk, its inventory or signed URLs. No storage is deployed by these helpers.

## Application footprints and lab preparation

This procedure is DESIGN-ONLY and needs the coordinator's Windows dry run.
It also applies to the native build. Stage `app_footprint.ps1`, `lab_prep.ps1`,
`app_prestart.ps1` and `app_autostart_common.ps1` together in a temporary guest
tools directory. Run elevated with the fixed disposable account's profile
loaded. Keep records private under `C:\ProgramData\KayfabeLab`.

Before each official installer, take a new snapshot. After installation, take
another and produce the app's footprint, supplying its actual pinned SHA-256:

```powershell
.\app_footprint.ps1 snapshot -OutputPath C:\BuildRecords\app_a-before.json
# Install the official application by hand, then:
.\app_footprint.ps1 snapshot -OutputPath C:\BuildRecords\app_a-after.json
.\app_footprint.ps1 diff -BeforePath C:\BuildRecords\app_a-before.json `
  -AfterPath C:\BuildRecords\app_a-after.json -App app_a `
  -InstallerSha256 $InstallerSha256 `
  -OutputPath C:\ProgramData\KayfabeLab\footprints\app_a.json
```

Review the new entries and `changed_existing` rows for every app. After all
installers and manual first-run steps, close applications and run:

```powershell
.\lab_prep.ps1
```

Require exit zero, `LAB_PREP_DONE` and `complete: true` in
`C:\ProgramData\KayfabeLab\lab_prep.json`. Preserve that original report and
the footprints. If it fails, stop the capture procedure and inspect the policy
code; do not bypass permissions or overwrite the report by blindly retrying.
Reboot and use an installed app ID to repeat the audit/reset without replacing
the original service-mode record:

```powershell
.\app_prestart.ps1 --app app_a -Action Reset
```

Test app switching before capture. Opt an app into the app-matrix hook with
`"prestart": {"timeout_s": 30, "tasks": []}` only after reviewing its footprint.
Keep task IDs empty unless the app needs manual task invocation. Stage the
private records at the default ProgramData paths; scripts are delivered at run
time. The [design and Windows-only checks](../design/V3_WINDOWS_APP_AUTOSTART.md)
describe protection exceptions, failure handling and coverage limits.

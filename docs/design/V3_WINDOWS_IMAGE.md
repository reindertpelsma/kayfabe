STATUS: DESIGN-ONLY, 2026-11-07. Owner/coordinator design; host tooling is covered
by GPU-free fixtures. No Windows boot, installed application, hardware, bucket,
upload, activation, compression ratio or performance is verified by this lane.

# Private Windows image: QEMU alternative path

## Owner choice and layers

DESIGN INTENT (owner/coordinator wind-down, 2026-11-07 UTC): the licensed-image
program is **DEFERRED** to a later round. The earlier choice of **native Windows
on Vast (windows-on-vast)** with an Alpine UEFI ESP swap for raw-disk capture
remains a future design choice, not an active build commitment. This page and
image_session.sh describe an **alternative QEMU path**, not the chosen native
capture implementation. Packaging, manifests and presigned-transfer
helpers remain available; create currently consumes a clean QEMU-session qcow2,
not an arbitrary native raw capture. Native capture integration is outside Y1.

DESIGN INTENT: the owner installs the stock NVIDIA driver and licensed software
manually through official installers using throwaway accounts. The base image,
run-time scripts and fresh-install claim lane remain separate:

| Layer | Intended contents | Verification obligation |
|---|---|---|
| Base image | Fresh Windows, virtio/SSH/guest channel, stock NVIDIA driver, owner-installed licensed software; optionally the few gate registry settings | Manual installer checklist, hashes, inventory, clean shutdown, artifact hash |
| Run-time scripts | Harness, registry tweaks, SPEC and the approximately 50 open Windows apps, GUI anchors/templates, probes and tests | Install separately for each run; pin versions and keep run evidence |
| Fresh-install release lane | Official pinned OS/driver installation with no prebuilt-image shortcut | Recheck that the stock driver installs on a fresh OS at the release revision |

DESIGN INTENT: reproducibility means the same documented manual steps, pinned
installer hashes and a reviewed inventory. It does not mean byte-identical
Windows disks. An image boot cannot establish the fresh-install claim. Driver
matrix runs may replace the guest driver at run time; record both the baked
driver and the tested driver.

CODE READ: `scripts/bench/windows/win_vm.sh` supplies `install`, `run`, `ssh`,
`scp-to`, `scp-from`, `status` and `stop`; the new wrapper uses those verbs. The
launcher has the opt-in network knob described below. `run` uses a writable overlay above `disk/base.qcow2`;
packaging flattens `disk/image-build.qcow2` into one compressed qcow2. Nothing
commits an overlay back into the sealed parent.

## Why retain the QEMU alternative

CODE READ: the launcher attaches the system disk on virtio-blk, exposes VNC at
`run/vnc.sock`, and uses `-rtc base=utc,driftfix=slew`. The wrapper opts into
`WINVM_VM_IDENTITY=1` for install and run, preserving `identity.env` across QEMU
restarts for `-uuid`, SMBIOS serial and MAC. The ordinary launcher's defaults
remain unchanged.

INFERRED: the QEMU alternative can avoid an extra native-to-virtio storage
migration step by installing directly on the same virtual disk controller as CI.
This does not override the owner's native-build choice or establish that native
capture will fail. UNTESTED: importing either image into fresh firmware/TPM state
and booting it on the target configuration remain coordinator experiments.

CODE READ: win_vm.sh install/check boots use `-vga std`. A run with `--no-kf3`
uses the same plain std-VGA display for the installed overlay. Ordinary kf3 run
mode instead uses `-vga none -device kf3-gpu,...`; `WINVM_KF3_DISPLAY=off` removes
the guest display, not just the QEMU console. Thus the presence of vnc.sock does
not establish a usable live Windows console in kf3 mode. Coordinator/owner
report (2026-11-07): the QEMU console copy path is low performance, lacks a live
Windows qualification and may remain only for screenshots; the broker is the
product viewing path. No live kf3 VNC session is part of this alternative.

DESIGN INTENT: export only the qcow2 disk and manifest. Never clone swtpm state,
SSH private keys, Secure Boot signing keys or the host `secrets/` directory.
Owner ruling §K in `docs/OWNER_RULINGS.md` forbids duplicating TPM seeds between
VMs. Provision fresh per-VM TPM/firmware state for consumers; importing a disk
into that state is an unverified coordinator task. Preserve the manifest's
non-secret machine identity for an image's sequential rebuild/run workflow;
concurrent guests need distinct identities and their own qualification.

## Owner manual checklist template

DESIGN INTENT: complete this before packaging. Leave application rows empty until
the owner supplies the actual apps and accepts their official installers. Do not
add example licensed apps as if they were installed or licensed.

Build date (real UTC): ______  Host revision: ______  Windows edition/build: ______
Driver version: ______  Image session tag: ______  Owner: ______

| Installer / version | Official URL | Pinned full SHA-256 | Exact manual steps / reboot / first-run steps | Throwaway-account note | Licence version, accepted date, CI/offline/image-use notes |
|---|---|---|---|---|---|
| | | | | | |
| | | | | | |
| | | | | | |

DESIGN INTENT checklist:

1. Use only throwaway accounts without payment methods or personal credentials.
   Record licence restrictions; installer acceptance and permission for automation
   remain separate questions. Private storage does not grant licence rights.
2. Record the official URL, version and full SHA-256 of **each installer actually
   used**, including a stock driver if it differs from the repo's default media
   pins. Keep the original installers and licence texts in owner-private storage.
3. Install licensed software manually. Keep SPEC and scripted open-app bundles
   out of this base. Do not store personal browser profiles, payment details,
   password managers or ordinary user accounts in it.
4. Record any required first-run steps and whether they need a GPU/network.
   Close applications, launchers, update services and background writers before
   inventory; list mutable file exclusions with reasons. Review installed packages
   for accidental run-time bundles or harness binaries before saying "done".
5. Verify the system volume is decrypted and BitLocker protection is off. Do not
   use TPM-sealed credentials or Windows Hello as image dependencies.
6. Supply inventory roots/exclusions and the manual installer checklist; run
   `image_session.sh stop`, then package. Review the private manifest and retain
   its checksum independently before any upload.

CODE READ: the manifest embeds `--installers` JSON if supplied (an owner record,
not a licence validator). Suggested JSON shape, with no populated app rows:

```json
[]
```

DESIGN INTENT: each owner-added array element should carry `name`, `version`,
`official_url`, `sha256`, `manual_steps`, `throwaway_account_note` and
`license_notes`. The media pins extracted from the checked-in launcher's `MEDIA`
array identify the repo defaults; they are **not proof** that those bytes were
used for the owner's later manual app/driver installations.

## Session and preparation contract

CODE READ: `scripts/bench/windows/image_session.sh` supports `install`, `start`,
`status` and `stop`. Both startup verbs require `--env-file`, `--ssh-host` and
optionally `--host-ssh-port` for the Linux builder. `install` first completes the
existing unattended OS installation. Both then start an overlay, wait for guest
SSH and set local `kf`'s password through stdin. Passwords are not exported or
put in argv. The env file is data, never sourced: one
`WIN_IMAGE_PASSWORD=literal value` line, with optional blank/comment lines.
Shell quoting is not interpreted. `--allow-internet` explicitly opts into
outbound installer/activation traffic; the default remains restricted.

CODE READ: `--display-mode std-vga` is the default **mode A**. It runs the
installed overlay with `win_vm.sh run --no-kf3` (`-vga std`), keeping the same
non-GPU display as the install/check boots. Use A for Windows setup and app
installers that need no GPU. It prints websockify's Unix target command on builder
loopback port 6080 and the workstation's exact SSH forward. noVNC assets are
expected at `/usr/share/novnc`. It does not start or install websockify.

CODE READ: `--display-mode broker --broker-socket <path>` selects **mode B** and
passes `display-broker=<path>,display-broker-uid=<caller uid>` using the existing
`--kf3-extra` option. It requires an existing caller-owned Unix socket and rejects
QEMU separators/whitespace in its path. It refuses `WINVM_KF3_DISPLAY=off` before
install/boot, since a broker needs the display plane. It does not print a live
VNC command for B. No default QEMU argv change is introduced by these wrapper
options; the original launcher fixture remains the default-shape check.

UNTESTED DESIGN — **all of mode B**, including its prerequisites, broker attach,
GPU/driver presentation, remote input, compositor, browser capture, encoding and
fallback, requires the coordinator's dry run. The wrapper prints commands; it
does not manage these display services. The coordinator supplies the broker
binary/socket and reserves a headless display before selecting B. A proposed
X11 route is browser-screencast's Xvfb + Openbox display, Picom's XRender
compositor, the broker's X11 window, and browser-screencast's x11grab capture:

```bash
# UNTESTED DESIGN: separate builder terminals, same reserved display and user.
browser-screencast --headless --headless-display :99 --headless-wm openbox \
  --listen 127.0.0.1 --port 6081 --capture x11grab --generate-token --no-audio
DISPLAY=:99 picom --backend xrender
DISPLAY=:99 /opt/nvkvm-broker/nvkvm-display-broker \
  --socket /workspace/winvm/owner-image-build/run/display-broker.sock \
  --backend x11 --persist --title 'Windows image build'
# Workstation: substitute the builder host/port, then use its private token URL.
ssh -N -p 22 -L 127.0.0.1:6081:127.0.0.1:6081 root@builder.example.invalid
```

CODE READ: [browser-screencast server.py at a7bde9fc](https://github.com/reindertpelsma/browser-screencast/blob/a7bde9fc4525fddab5ffe13d5d7561537f16a5c7/server.py)
parses these flags; its headless helper creates Xvfb/window-manager processes.
This source reading supplies no end-to-end Windows result. The coordinator must
pin/review the version installed on the box, install dependencies separately,
keep the generated token private, and select a free display/port. A blank broker
window, failed capture, lost input or stalled refresh falsifies this proposed B
route; do not substitute a live kf3 noVNC console as if it had a qualification.

UNTESTED DESIGN — B fallback: request QMP `screendump` to a host-local PPM every
2 seconds and transfer/view each completed screenshot over the existing SSH
connection; inject keyboard events with QMP `send-key`, or relative pointer
movement/buttons with `input-send-event`. This is polling, not a live video path.
It depends on the console screenshot hook and is unavailable if that hook is
removed; stop the B experiment if neither broker nor screenshot/input works.
The wrapper prints a single screenshot request and a Return-key example. For
example (not executed in Y1):

```bash
python3 scripts/bench/windows/qmp.py /workspace/winvm/owner-image-build/run/qmp.sock \
  cmd screendump '{"filename":"/workspace/winvm/owner-image-build/logs/console.ppm"}'
python3 scripts/bench/windows/qmp.py /workspace/winvm/owner-image-build/run/qmp.sock \
  cmd send-key '{"keys":[{"type":"qcode","data":"ret"}]}'
python3 scripts/bench/windows/qmp.py /workspace/winvm/owner-image-build/run/qmp.sock \
  cmd input-send-event '{"events":[{"type":"rel","data":{"axis":"x","value":20}}]}'
```

CODE READ: after a clean `stop`, `start --resume` reopens the same image-build
overlay with a new session tag and chosen display mode. It checks the previous
clean phase/receipt and unchanged disk fingerprints, clears the old clean
receipt, and retains the existing VM identity. This permits A -> B -> A while
keeping packaging tied to the last completed preparation/shutdown. A running
session cannot be resumed. Real Windows switching between std VGA and kf3 is
UNTESTED, including display-adapter rebind and first GPU use.

CODE READ: the env file must be outside the checkout, caller-owned and unreadable
by group/other users. Guest command output is suppressed, including failures.
The original host secrets/win_password is not updated; do not use desktop-prep
with that stale password afterward. `status` prints the selected mode's viewing
instructions again. The wrapper never logs a browser token.

CODE READ: `stop` stages `scripts/bench/windows/image_prepare_guest.ps1` and a
JSON configuration, executes the script over the existing SSH channel, pulls
the completed inventory to the host, removes those staged preparation files and
the install check script (`C:\kf\check.ps1`),
then calls `win_vm.sh stop`. A preparation failure leaves the session available
for inspection and produces no clean receipt. `status` prints the console
commands again. Use a fresh `WINVM_NAME` per new build; the wrapper refuses a
second session or an existing image-build overlay unless an explicit clean resume is requested.

CODE READ: guest preparation disables hibernation with `powercfg.exe /h off`,
sets fast-startup `HiberbootEnabled=0`, stops/disables W32Time and reads these
conditions back. It requires a fully decrypted BitLocker volume. It inventories
the machine uninstall keys (64/32-bit) and the `kf` user's uninstall key,
`Get-Package`, NVIDIA PnP signed-driver versions and the Windows build. It hashes
every non-excluded file under chosen roots with SHA-256 and records each excluded
path/reason and the exclusion rules. Reparse points are refused. Other users'
per-user packages are outside that inventory.

Configuration example (paths are illustrative; choose the actual installed roots):

```json
{
  "app_roots": [],
  "exclude": []
}
```

DESIGN INTENT: an exclusion is `{"pattern":"<full-path wildcard>","reason":"<why mutable>"}`.
An empty root list intentionally provides package/driver inventory only, not
application file coverage. Hashes describe files as collected, not an atomic
filesystem snapshot; the owner must stop writers.

CODE READ: if the tools directory (`C:\kf\tools`) contains `sdelete64.exe` or
`sdelete.exe`, preparation calls it with `-z` on the system drive. The owner must
stage the official binary and accept its EULA interactively; nothing downloads
it or accepts terms silently. [Microsoft documents `-z` as zeroing free space](https://learn.microsoft.com/en-us/sysinternals/downloads/sdelete).
Otherwise a new temporary file is filled with actual zero buffers, flushed and
deleted in `finally`. This fallback leaves **64 MiB unzeroed**, recorded as
`reserve_bytes`, so Windows can finish shutdown. Concurrent disk writers or an
I/O error fail preparation. A pre-existing zero file is never overwritten or
deleted. Zero-fill is a compression aid, not a secure-erasure claim; NTFS metadata,
excluded files and newly written data can retain old content.

## Packaging and private transfer

CODE READ: `scripts/bench/windows/image_package.sh create --out <new-directory>`
holds the session lock and the launcher's `run/vm.lock`. It requires the session's
clean receipt, unchanged overlay/base disk stat fingerprints and VM identity, matching inventory SHA-256
and the final boot's QMP log showing a guest-origin `guest-shutdown` with no panic.
The existing launcher's fallback QMP quit is **not** sufficient evidence. These
are local workflow checks, not a hostile-guest attestation or NTFS repair tool.

CODE READ: it runs `qemu-img check`, `qemu-img convert -c -f qcow2 -O qcow2`,
checks the output, compares virtual disk content with the source and verifies
the output has no backing-file dependency. Output is staged in a sibling
temporary directory and renamed only after success. The manifest records the
artifact name, length, SHA-256, `qemu-img info`, guest inventory, real host UTC
build date, repo media pins, host kayfabe revision/dirty flag, non-secret VM
identity and shutdown evidence. No TPM/firmware/secrets directory is archived.

DESIGN CHOICE: compressed qcow2 gives one flattened directly bootable object and
uses the already-required qemu-img tool; raw plus zstd would need an additional
decompression/storage step before boot. Compression size/time benefits are
unmeasured. [QEMU documents compressed conversion and warns against modifying an in-use image](https://www.qemu.org/docs/master/tools/qemu-img.html).

CODE READ: `upload --package <directory> --url-file <file>` validates the local
artifact against its manifest, then performs a single PUT with three curl retries.
`verify` downloads using a separately supplied presigned GET into temporary
storage, checks SHA-256 and length, then deletes the download. Neither command
generates URLs or calls a Worker. The caller supplies a short-lived URL in an
owner-only file outside the repo. The URL is passed to curl through stdin config,
not argv, and curl output/diagnostics are suppressed. HTTPS and an HTTP 2xx
response are required; redirects are refused. No long-lived R2 credential is
stored or requested.

DESIGN LIMIT: multipart/resume is deferred. Single R2 PUT is limited to 5 GiB;
the helper refuses larger artifacts. A larger Windows image needs a caller-managed
multipart uploader and then this helper's presigned GET verification. The
coordinator must measure the compressed size before selecting that path. Use a
fresh presigned URL if retries outlive its validity; retries restart the whole
PUT. Do not expose presigned URLs through shell tracing, shared terminals or git.

## Risks to resolve on the coordinator's box

DESIGN RISKS (unmeasured in Y1):

- Fast startup, hibernation and dirty NTFS can produce an unusable capture.
  `qemu-img check` checks the container, not NTFS; guest preparation plus a clean
  shutdown must be followed by a boot/read check. Never package after a kill or
  panic; reboot the build guest and prepare/shut down again.
- The repo pins an evaluation edition. Record its install/expiry dates using real
  time and plan lawful rebuilds or an appropriate licence. A faked QEMU clock and
  disabled time sync do not renew an evaluation licence or grant usage rights.
  This wrapper keeps `base=utc`; it does not introduce a faked clock. Any later
  offline/fixed-time feasibility experiment must record real and guest times
  separately, including activation/TLS failures.
- A stock NVIDIA driver can need rebind/restart on another GPU die. The owner
  expects upgrades to work, but cross-die driver rebind and application activation
  must still be measured. Preserve machine identity without copying TPM secrets.
- Some official installers and first launches need a functioning GPU or online
  activation. Completing those steps and then running offline cannot be proven
  by this lane's mocks. Pin UI automation inputs separately at run time.
- A private disk can contain throwaway account tokens and installed licensed
  software. Use **throwaway accounts only, private R2 only, never public**: no
  public release assets, public object URLs, shared caches, fork CI or container
  registries. Review licence notes before copying it to a rented box and delete
  the box's disk/copies at teardown. Keep SPEC's separate storage rules in
  `docs/DOWNLOADS.md` and the score restrictions in `docs/SPEC_RESULTS.md`.

## Dry-run plan (future coordinator work; no execution authorized in Y1)

DESIGN INTENT: the coordinator will test on one authorized Linux Vast box, keep
all steps/evidence at the exact host revision and destroy only that session's
resources at the end.

1. UNTESTED QEMU alternative, mode A: falsifier for the boot-shape hypothesis: the installed disk cannot boot on
   the launcher with fresh per-VM firmware/TPM state. Install via the wrapper;
   check Secure Boot, virtio-blk, identity stability across restarts and the
   printed loopback noVNC/SSH commands. Record the QEMU argv and Windows build.
2. UNTESTED mode B: first start the headless compositor, broker and browser-screencast, then resume in broker mode. Falsifier for the session contract: the supplied local password fails console
   sign-in, broker/browser rendering or input fails, or the password appears in logs/argv. Use a disposable password, verify sign-in,
   inspect logs locally without committing secrets, and complete the manual rows.
   Test QMP screenshot polling/input separately; record its limitations and any absent hook.
3. Falsifier for preparation: any read-back condition is false, inventory misses
   a chosen test file or its exclusion, hashes disagree, or the zero-file cleanup
   fails. Test both a staged sdelete (EULA accepted by owner) and the documented
   fallback. Record inventory coverage and exclusions; keep raw inventory private.
4. Falsifier for the capture guard: a busy VM, forced QMP quit, panic, changed
   source or failed check still yields a package. Exercise those negative cases
   on disposable builds, then re-prepare a clean build. Do not weaken guards to
   continue after a refusal.
5. Falsifier for packaging: `qemu-img compare` differs or the artifact has a
   backing dependency. Package the clean overlay; measure bytes, duration and
   required free space. Retain manifest and SHA-256 independently.
6. Falsifier for private transfer: a corrupted download passes SHA-256, a secret
   URL appears in logs, or an object is public. Only the coordinator may arrange
   private storage and presigned URLs. For >5 GiB, use caller-managed multipart;
   fetch through a presigned GET and compare with the retained manifest.
7. Falsifier for reuse: an independently provisioned guest cannot boot/sign in,
   NTFS is dirty, or the stock driver/app fails first run. Test the same die and
   a different die, record activation/rebind and offline behavior, and verify
   run-time scripts/SPEC/open apps are installed separately. Run the independent
   fresh-install release lane; do not update release claims from this dry run
   without the actual stage evidence.

## Local verification and limits

CODE READ: `python3 scripts/ci/test_windows_image.py` uses fake `win_vm.sh`,
`qemu-img` and `curl`; `python3 scripts/ci/windows_image_mutants.py` requires each
listed mutant to fail at least two named tests. Guest PowerShell checks are
explicitly source contracts, not Windows execution. Neither suite boots a VM,
downloads an image, contacts R2 or establishes a release claim.

## Deferred licensed-image notes (DESIGN-ONLY)

DESIGN INTENT (owner/coordinator wind-down, 2026-11-07 UTC): retain these earlier
ideas for a later licensed-image round. They are not implemented by this PR,
are not active lane tasks, and have no Windows/hardware qualification here.

- DESIGN-ONLY: a versioned machine-state JSON beside the private image could
  bind image/blob SHA-256 values, SMBIOS, NIC/disk identifiers, machine type,
  monitor identity and a fixed RTC base to pristine TPM/OVMF state. Each run
  would copy that state afresh and never write back counters/NV changes. CPU
  observations would be diagnostic only; an optional override would change
  only the brand string and never mask ISA features. The owner's offline
  non-sensitive-by-design exception would cover approved disposable guest
  state only, never personal/host credentials.
- DESIGN-ONLY: before/after installer footprints could retain new services,
  tasks, Run/RunOnce values and startup items plus exact installer hashes and
  original modes for rollback. The final owner correction calls for ordinary
  Automatic/Delayed-Automatic services to become Manual, with Disabled reserved
  for exact updater/telemetry service/task IDs. Protected NVIDIA/driver,
  installer, SSH/guest-agent, virtio and TPM/crypto infrastructure must remain
  untouched. Prestart would be an optional service-only escape hatch justified
  by an app's failed demand-start acceptance test. Manual alone would not
  establish cross-app isolation; cleanup/reboot acceptance and fresh guests
  would still need a separate plan.
- DESIGN-ONLY: per-app partitions/packs remain deferred. No partition layout,
  boot integration or publication plan is selected here. A future design must
  resolve immutable-base ownership, application/data separation, hashes,
  rollback and licence/activation behavior before implementation.

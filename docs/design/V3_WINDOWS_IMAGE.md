STATUS: DESIGN-ONLY, 2026-11-07. Owner/coordinator design; host tooling is covered
by GPU-free fixtures. No Windows boot, installed application, hardware, bucket,
upload, activation, compression ratio or performance is verified by this lane.

# Prebuilt private Windows image

## Layers and claim boundaries

DESIGN INTENT (owner, 2026-11-07): build a fresh Windows installation **inside our
own QEMU guest on a Linux Vast box**. The owner installs the stock NVIDIA driver
and licensed applications by hand through official installers. The owner says
"done", and tooling prepares and packages the resulting disk. This is an owner's
private image, never a redistributable project release.

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
launcher adds only the opt-in network knob described below. `run` uses a writable overlay above `disk/base.qcow2`;
packaging flattens `disk/image-build.qcow2` into one compressed qcow2. Nothing
commits an overlay back into the sealed parent.

## Why build in this QEMU shape

CODE READ: the launcher attaches the system disk on virtio-blk, exposes VNC at
`run/vnc.sock`, and uses `-rtc base=utc,driftfix=slew`. The wrapper opts into
`WINVM_VM_IDENTITY=1` for install and run, preserving `identity.env` across QEMU
restarts for `-uuid`, SMBIOS serial and MAC. The ordinary launcher's defaults
remain unchanged.

INFERRED: installing in the same virtual boot/storage shape as CI reduces the
risk of a native Windows-on-Vast disk failing to boot under virtio-blk. A native
disk capture and WinRE/WinRM transfer are outside this design. No boot migration
has been measured here.

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
`status` and `stop`. `install` completes the existing unattended OS install and
then starts the interactive build overlay. `start` uses an already installed
VM. Both require `--env-file`, `--ssh-host` and optionally `--host-ssh-port` for
the Linux builder. They wait for guest SSH and set the existing local `kf`
account password through SSH stdin. `--allow-internet` opts into outbound guest
networking for official installer/activation steps via `WINVM_NET_RESTRICT=off`;
the default remains `on` (restricted). The launcher's VM configuration change is
this validated on/off substitution; adjacent diagnostic/log handling also gets
ShellCheck cleanup. Dry-run tests compare the full default QEMU argv against an
`origin/next` fixture, compare it with explicit `on`, and confirm that `off`
changes only the network restriction. Passwords are neither exported nor put in
argv. The env file is data, never sourced: one `WIN_IMAGE_PASSWORD=literal value`
line, with optional blank/comment lines; shell quoting is not interpreted.

CODE READ: the env file must be outside the checkout, owned by the caller and
unreadable by group/other users. It is not copied into logs or a manifest. Guest
command output is suppressed, including failures. The guest account policy can
reject a password; the wrapper then refuses readiness. The builder's original
`secrets/win_password` is not changed; do not use `desktop-prep` with that stale
value after setting the image account password.

CODE READ: the printed console commands start websockify/noVNC on the builder's
loopback TCP port 6080 with `--unix-target <VM>/run/vnc.sock`; the workstation
uses `ssh -N -p <port> -L 127.0.0.1:6080:127.0.0.1:6080 <builder>` and opens
`http://127.0.0.1:6080/vnc.html?autoconnect=1`. The wrapper does not install or
start websockify, open firewall ports or forward an SSH agent. noVNC assets are
expected at `/usr/share/novnc`. Change that path manually if the package uses a
different location. See the [websockify Unix target implementation](https://github.com/novnc/websockify/blob/master/websockify/websocketproxy.py).

CODE READ: `stop` stages `scripts/bench/windows/image_prepare_guest.ps1` and a
JSON configuration, executes the script over the existing SSH channel, pulls
the completed inventory to the host, removes those staged preparation files and
the install check script (`C:\kf\check.ps1`),
then calls `win_vm.sh stop`. A preparation failure leaves the session available
for inspection and produces no clean receipt. `status` prints the console
commands again. Use a fresh `WINVM_NAME` per new build; the wrapper refuses a
second session or an existing image-build overlay.

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

1. Falsifier for the boot-shape hypothesis: the installed disk cannot boot on
   the launcher with fresh per-VM firmware/TPM state. Install via the wrapper;
   check Secure Boot, virtio-blk, identity stability across restarts and the
   printed loopback noVNC/SSH commands. Record the QEMU argv and Windows build.
2. Falsifier for the session contract: the supplied local password fails console
   sign-in or appears in logs/argv. Use a disposable password, verify sign-in,
   inspect logs locally without committing secrets, and complete the manual rows.
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

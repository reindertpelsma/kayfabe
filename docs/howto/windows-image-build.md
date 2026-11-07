# Build a private Windows image

The image-build helpers are experimental and have only been checked with mocks.
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
install the stock NVIDIA driver and licensed software through official installers.
Use throwaway accounts only and record each installer's version, official URL,
full SHA-256, exact steps and licence notes. Keep SPEC, scripted open-app bundles
and the harness for run time.

If an official installer or activation needs outbound guest networking, add
`--allow-internet` to `install` or `start`. The default keeps the guest network
restricted. This changes only the launcher's `WINVM_NET_RESTRICT` knob; keep
offline and online first-run observations separate in your records.

Check the session with:

```bash
scripts/bench/windows/image_session.sh status
```

Close applications and background writers. Create a JSON file selecting actual
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

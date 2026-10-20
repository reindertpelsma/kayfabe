# Install the beta candidate locally

STATUS: BETA CANDIDATE, 2026-10-20. There is **no supported external-user path**
yet. These are reviewable local build/install steps; a clean-directory hardware
walk-through remains Stage 5. No binary download, signing or CI release is claimed.

CODE READ: x86_64 Linux, working KVM with in-kernel irqchip/MSI irqfd, stock
NVIDIA kernel driver and matching CUDA driver libraries are required. The host
GPU stays bound to NVIDIA: no VFIO rebind or dedicated IOMMU group is required.
Do not combine kf3 with VFIO, iommufd, vfio-user, SEV/COLO or another guest-RAM
pinning device: the device refuses that combination. Host and broker are trusted;
there is no protection of guest state from a malicious host administrator.
Guest userspace and root are untrusted at the device boundary.

CODE READ: [the support matrix](SUPPORT_MATRIX.md) distinguishes old measured
rows from untested C6. Rootless was measured on product `0a746117`, RTX 4070,
open host driver 595.91.07, Ubuntu 26.04/kernel 7.0.0-34: Linux fast suite30/30
and gates9/9 plus10, Windows580.88 with a test broker. The user had only kvm group
access, zero capabilities, 8 MiB memlock and zero hugepages. That does not certify
C6 or a real compositor/VRAM broker. NVIDIA nodes were 0666 on that distro;
/dev/kvm and /dev/udmabuf needed kvm access. Other distributions must be checked.
A desktop using the same NVIDIA card was not validated by that audit.

## Build without replacing system QEMU

CODE READ: run from this checkout. Install compiler/Rust/tooling and development
packages first; the commands below require root only for package installation.
Rust is pinned by rust-toolchain.toml. QEMU must be 10.2.4; obtain and verify its
release archive using [QEMU's downloads](https://www.qemu.org/download/).
This lane's local artifact does not establish a portable library/glibc floor.

```sh
sudo apt-get install build-essential ninja-build pkg-config python3 python3-venv \
  libglib2.0-dev libpixman-1-dev libslirp-dev \
  libwayland-dev wayland-protocols libxcb1-dev libxcb-dri3-dev \
  libxcb-present-dev libxcb-render0-dev libxcb-xinput-dev ovmf swtpm qemu-utils
export CARGO_TARGET_DIR="$PWD/.build/kayfabe-target"
nice -n 19 bash scripts/bench/build_kf3.sh /path/to/qemu-10.2.4 "$PWD/.build/qemu-build-kf3"
nice -n 19 make -C broker -j2
```

CODE READ: `/path/to/qemu-10.2.4` is an operator-supplied source directory, not a
repository path. The builder compiles the Rust archive and QEMU C overlay as one
binary, checks the archive by cargo's successful fingerprints, and publishes
`.build/kf3-bins/<revision>/qemu-system-x86_64`. Existing revision binaries are
never overwritten. Its pc-bios/qemu-bundle links point to its own QEMU build;
keep that build until making a self-contained staging directory. The builder
also resolves dangling bundle links through Ninja's declared generated-data
targets before publishing; it refuses missing data without a build target.

```sh
REV=$(git rev-parse --short=8 HEAD)
python3 scripts/release/stage_release.py \
  --qemu-bin ".build/kf3-bins/$REV/qemu-system-x86_64" \
  --broker-bin broker/nvkvm-display-broker --out .build/release
bash scripts/kayfabe-install.sh --from .build/release --prefix "$HOME/.local/opt/kayfabe-beta"
"$HOME/.local/opt/kayfabe-beta/bin/kayfabe-doctor.sh"
```

CODE READ: staging requires a clean checkout and matching component version
output. A reviewed prebuilt kf3-bins binary of this exact revision can replace
the source build step; it still needs its matching firmware/bundle tree. Nothing
installs over `/usr/bin/qemu-system-x86_64`. `/opt/kayfabe-beta` is an alternative
prefix requiring administrator permission. The prefix must not already exist.

## Release layout and preflight

| Prefix entry | Purpose |
|---|---|
| VERSION | One canonical version, currently 0.1.0-beta.1 |
| bin/qemu-system-x86_64 | QEMU with kf3 built in; -device help prints the version |
| bin/nvkvm-display-broker | Same version; --version exits before opening a session |
| bin/kayfabe*.sh and bin/kayfabe.py | Launcher, doctor, prefix install/uninstall |
| bin/pc-bios and bin/qemu-bundle | Firmware and build bundle, copied during staging |
| docs, LICENSE, release.json | Instructions, repository licence and file/build identity |
| Optional guest module | Not required to boot; not shipped by this staging script |

CODE READ: doctor is read-only. It reports PASS/WARN/FAIL and prints corrective
commands; it never executes the printed fixes, installs a driver, changes groups,
raises limits or starts a VM/CUDA workload. It queries nvidia-smi and sysfs BAR
resources, checks KVM/NVIDIA permissions and optional display udmabuf access, historical driver/family evidence,
free VRAM, versions, desktop session, groups, disk, memlock, hugepages and cgroup.
WARN on historical/untested support and unlimited cgroup memory is intentional.
Missing udmabuf access warns about the unavailable DMA-BUF tier; the existing
shared-memory broker path remains available.
The launcher does not invent the still-unmeasured QEMU cgroup overhead allowance.
Use the desktop user for both broker and QEMU. Logout/login after a group change.
Do not run doctor against an occupied validation GPU during a reserved lane.

```sh
"$HOME/.local/opt/kayfabe-beta/bin/kayfabe-run.sh" --version
"$HOME/.local/opt/kayfabe-beta/bin/nvkvm-display-broker" --version
"$HOME/.local/opt/kayfabe-beta/bin/qemu-system-x86_64" -device help
```

CODE READ: the launcher refuses mixed QEMU/broker/launcher versions. All parts
must come from one staged directory; the version label alone is not an artifact
checksum. release.json records the source SHA and file hashes. Packaging of
non-glibc libraries, signatures/attestations, CI, licence/provenance review,
security review and log/telemetry hygiene remain release gates.

## Uninstall

```sh
"$HOME/.local/opt/kayfabe-beta/bin/kayfabe-uninstall.sh" --prefix "$HOME/.local/opt/kayfabe-beta"
```

CODE READ: uninstall removes only unchanged manifest files inside this prefix;
user-added or modified files remain. It does not remove the user's disk, UEFI
variables, TPM state or logs. Those live beside the disk in `.kayfabe/<vm-id>`.
See [running and first boot](RUNNING_KAYFABE.md) for the next steps.

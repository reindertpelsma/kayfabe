STATUS: RESEARCH, 2026-11-08. Evaluation of the owner's chroot alternative; not an implemented backend or a replacement for the current QEMU/cloud-init builder. MEASURED evidence is limited to the [bounded filesystem probe](../../traces/apps_image_chroot_eval_20261108/README.md).

# Linux apps image without KVM during the build

**OWNER CORRECTION, 2026-11-08:** compiled outputs are not expected to be byte-identical across boxes. Compare only PINNED inputs and require app presence; record BUILT binary/image hashes for information. This supersedes the earlier app-hash equality proposal and strict level-2 acceptance proposal. The filesystem byte-equality experiment remains research, not a verification or release gate.

**DESIGN — owner input, 2026-11-08:** build the open image independently on each of eight release boxes, or on a trusted builder/CI runner, without KVM. Keep NVIDIA proprietary inputs outside the image. Compare each box's inventory hash, minimize temporary disk, and investigate byte reproducibility. Preserve the current [recipe and QEMU path](V3_APPS_IMAGE_RECIPE.md) until a replacement is qualified.

**RECOMMENDATION:** prototype a Python-orchestrated **mmdebstrap directory + native chroot + fresh ext4 packing** backend. Target **level 1: PINNED input inventory identical** first. Treat **level 2: identical unbooted artifact bytes** as optional diagnostic research, not an acceptance condition. No complete chroot image, cross-host pair, boot or GPU run has been qualified by this evaluation.

**Notation:** CODE READ covers repository inspection and cited upstream documentation/source; DESIGN describes proposed work; INFERRED marks planning estimates; MEASURED refers only to the [2026-11-08 local probe](../../traces/apps_image_chroot_eval_20261108/README.md). Commands below are proposed argv data, not an available chroot CLI.

## Bootstrap choice and portability

**CODE READ / DESIGN:** the alternatives differ in prerequisites and how much image policy they own:

| Tool | Useful capability | Constraint and proposed use |
|---|---|---|
| debootstrap | Creates a distro directory through upstream bootstrap scripts; familiar minimal starting point | Debian documents that its output is not reproducible out of the box. Keep as a fallback bootstrap implementation, with the same explicit normalization and package lock; upstream shell internals remain dependencies. |
| mmdebstrap | Apt dependency resolution, multiple explicit repositories, root and user-namespace modes, reproducibility support | Preferred first prototype: explicit directory output, Ubuntu keyring, exact sources, no project-authored shell hooks. Host apt/dpkg/Perl and helper versions must be pinned or provided by a tools tree. |
| mkosi | Declarative packages, separate build packages, tools trees and image/partition construction | Useful later if a self-booting disk is required. More host dependencies and namespace requirements; pin a tested version. Current upstream options cannot be assumed to exist in Noble's packaged 20.2. |

**CODE READ:** sources: [Debian reproducible installs](https://wiki.debian.org/ReproducibleInstalls), [Noble mmdebstrap 1.4.3 manual](https://manpages.ubuntu.com/manpages/noble/man1/mmdebstrap.1.html), [mkosi upstream manual](https://github.com/systemd/mkosi/blob/main/mkosi/resources/man/mkosi.1.md), and [Noble mkosi package](https://packages.ubuntu.com/noble/admin/mkosi). The proposed mmdebstrap path uses directory output, not an ext4 output option from a newer manual.

**CODE READ / DESIGN:** “any Linux with root” needs a narrower acceptance contract. The current binaries, wheels and compile targets are amd64/sm_86. A native build needs an x86-64 Linux kernel capable of executing the pinned Ubuntu userspace, sufficient capabilities, compatible filesystem semantics, and the pinned bootstrap/tool dependencies. An ARM host needs an additional cross-build or user-mode emulation design; it does not execute this recipe unchanged. A root user inside a restricted container can lack required capabilities. Neither case should silently fall back to an unisolated build. No `/dev/kvm`, GPU device, loop device or running systemd is needed for the proposed native build itself.

## Privilege contract

**CODE READ / DESIGN:** proposed root mode has these requirements; this is not a claim that every Vast container currently grants them:

| Operation | Privilege or host condition |
|---|---|
| Enter target root | `CAP_SYS_CHROOT`; always enter `/` afterwards and close inherited host directory/socket descriptors |
| Private mount/PID/UTS/IPC/network namespaces, proc/tmpfs/devpts mounts | `CAP_SYS_ADMIN` and kernel/seccomp/LSM permission for the corresponding namespace/mount operations; make propagation private before mounting |
| Package ownership and privilege drops | `CAP_CHOWN`, `CAP_FOWNER`, `CAP_FSETID`, `CAP_SETUID`, `CAP_SETGID`, and ordinary root DAC access; package-specific file capabilities can additionally need `CAP_SETFCAP` |
| Minimal device nodes | `CAP_MKNOD` if created; expose only explicitly needed null/zero/random/tty devices, never host GPU or block devices |
| Network setup | No external networking inside the build namespace; configuring an interface additionally needs `CAP_NET_ADMIN`. A local `file:` apt repository avoids an interface entirely. |
| Fresh ext4 from an existing directory | `mke2fs -d` writes a regular file without mounting or loop setup; source ownership/xattrs must still be correct and readable |
| Optional rootless bootstrap | mmdebstrap user-namespace mode needs permitted user namespaces plus appropriate uid/gid mappings/helpers; it is an optional separately qualified path |

**CODE READ:** [chroot(2)](https://man7.org/linux/man-pages/man2/chroot.2.html) specifies the chroot capability and explicitly does not define a security sandbox. The [mmdebstrap modes](https://manpages.debian.org/testing/mmdebstrap/mmdebstrap.1.en.html) describe root-mode mounts and unprivileged uid mappings. A namespace wrapper is necessary process/mount hygiene, not proof that arbitrary privileged maintainer scripts are safe. Use dedicated trusted build capacity where possible; do not bind host `/run`, D-Bus, `/boot`, `/lib/modules`, `/usr/local/cuda`, or whole `/dev` into the target. Keep host package databases and configuration outside all writable build paths. Preflight refusal is a failing build, not permission to drop isolation or disable host security globally.

**MEASURED, 2026-11-08 ([probe record](../../traces/apps_image_chroot_eval_20261108/probe.json)):** the filesystem-only experiment ran as uid/gid 65534 with effective capabilities zero, without mounts, chroot, package installation, networking or QEMU. This establishes no privilege result for the bootstrap/compile stages.

## Proposed recipe and execution changes

**CODE READ:** the current recipe's 48 requested apt packages supplement a pinned cloud image. They are not a complete from-scratch OS definition. Its `guest.py` entry point requires the running guest kernel, masks services with `--now`, uses virtio reporting ports and calls systemd poweroff. It cannot be reused as the chroot entry point unchanged.

**DESIGN:** extend and version the same recipe with an explicit bootstrap profile, host tools pins, complete boot package closure, filesystem settings and immutable build environment. Preserve the existing cloud-image profile and its pins. A bootstrapped root has a different origin and potentially a different dpkg inventory: do not claim that an unused cloud-image SHA was its base, or compare its new OS package set against the old cloud-image manifest as though they were identical. Both deployment locations must select the same bootstrap/profile/toolchain digest.

**DESIGN — execution sequence:**

1. Validate the recipe, architecture, capabilities and free space. Fetch signed, SHA-pinned snapshot metadata and all needed package pins on the host. Preserve main/universe-only validation, archive signatures and J4's bounded fetch/error handling. Disable only frozen metadata expiry, not signatures. Supply local apt pools accepted by the hash/signature checks and hash-locked wheels; the chroot remains offline.
2. Bootstrap a disposable build directory from the pinned Ubuntu snapshot. Set explicit package recommendations policy, locale, timezone, hostname, umask, uid/gid allocation and build path. Install `policy-rc.d` returning 101; a project-provided policy helper can be Python. Upstream bootstrap/dpkg/build tools may internally use shell, as they already do in the current path.
3. Execute the recipe's compile argv lists in that directory, with the pinned CUDA compiler/toolkit available transiently. Do not discover a toolkit or native CPU/GPU architecture from the box. Keep shared cudart rules, ELF checks, licence retention and the `nvidia-layer-static` exception. Export only audited open outputs; destroy the entire NVIDIA-equipped build directory before assembly.
4. Bootstrap a **fresh** open assembly directory. Install declared open packages, wheels, models, user/configuration files and the audited export. Install the declared kernel/modules and initramfs tooling explicitly; the cloud image no longer supplies them. Retain all dependencies required by subsequent offline provisioning, including kernel headers/compiler needs, until a smaller profile has itself been qualified.
5. Generate the initramfs for `6.8.0-147-generic` explicitly, never the host's `uname`. Use a declared module policy/list for the actual virtio/root filesystem contract, not host probing. Pin hooks and compressor; exclude host firmware, machine identity, resume devices and proprietary NVIDIA files. Remove build-only caches, sources and temporary policy files; preserve runtime configuration and licences.
6. Collect an offline inventory and perform the complete package/file/linkage scan **inside the assembly root**. Obtain installed kernel identity from its packages/files; only the later boot establishes the running kernel. Reject dirty scans, missing apps, unexpected packages and input-pin drift. Record compiled-output hashes without rejecting their differences. Normalize filesystem metadata and create a fresh regular-file ext4 artifact, with no deleted builder blocks copied into it.
7. Publish only a build-complete/unbooted receipt, open artifact and manifest. Run a distinct boot verification against a disposable overlay/copy; require the actual running kernel, complete live inventory, fresh completion marker and unchanged input artifact. Runtime NVIDIA provisioning remains a later transient operation from official pinned sources.

**CODE READ / DESIGN:** CUDA compilation still needs NVIDIA tools **before** producing the open image. The runtime-only policy does not make `nvcc` open-source or eliminate that temporary build dependency. Host J4 acquisition and disposable compiler storage remain necessary; no NVIDIA payload enters the open root/image, R2 or GitHub. Reusing a per-run pinned installer cache until the gate finishes is possible outside the image, followed by deletion; it is not a distributed cache.

## What does and does not need a running init

**CODE READ / DESIGN:** upstream mechanisms and the proposed handling are:

| Work | Offline handling or limitation |
|---|---|
| Apt/dpkg, pip, native compilation, ldconfig, users/groups, writing files | Work on a prepared root directory with declared dependencies. Any package requiring a live daemon is a qualification blocker, not a reason to ignore its failure. |
| systemd enable/disable/mask/preset | Use explicit offline `--root` operations or declared links. `start`, `stop`, `restart`, `daemon-reload`, `--now` and poweroff require a different phase; do not call the host manager. |
| Service auto-starts | `policy-rc.d` denial is supported for policy-aware package helpers; it is not a universal block on a program launching a daemon directly. No host runtime sockets; investigate incompatible maintainer scripts. |
| Kernel/modules/initramfs | Installation and archive generation do not require booting that kernel. Force the target version and module policy; `uname`, `/proc` and `/sys` otherwise describe the builder kernel/hardware. No modprobe, DKMS host-kernel autodetection or GPU test in the build. |
| cloud-init, SSH keys and machine ID | Write configuration offline; execute first-boot behavior only in verify/runtime. Do not bake per-box keys or machine IDs into the reusable artifact. Retain cloud-init if reusing its existing NoCloud verification agent. |
| Snaps | A plain chroot has no snapd/systemd mount lifecycle. Do not add snap installation or transitional debs that demand it to this open profile. Offline seeding would be separate work and still needs first-boot validation. Current app pins do not require snaps. |
| Udev, D-Bus, network configuration, GPUs, live driver loading | Files can be installed/configured offline; actual service/device behavior needs boot/hardware qualification. |

**CODE READ:** supporting interfaces: [systemctl offline root](https://freedesktop.org/software/systemd/man/latest/systemctl.html), [invoke-rc.d policy/status 101](https://manpages.debian.org/testing/init-system-helpers/invoke-rc.d.8.en.html), [Ubuntu mkinitramfs](https://manpages.ubuntu.com/manpages/noble/man8/mkinitramfs.8.html), [module selection](https://manpages.debian.org/trixie/initramfs-tools-core/initramfs.conf.5.en.html), and [Snap system architecture](https://snapcraft.io/docs/reference/system-architecture/). Their existence is not a completed install of this package set.

## Filesystem, boot contract and determinism

**DESIGN:** first prototype a raw ext4 root plus separately hashed kernel/initramfs files. QEMU's [direct Linux boot](https://www.qemu.org/docs/master/system/linuxboot.html) can use those artifacts with `root=UUID=...`; no partition mounting is required during construction. **An ext4 file alone is not a BIOS/UEFI-bootable disk.** The existing disk-boot gate needs an explicit adapter or a second, fully specified GPT/bootloader image contract. If firmware boot is required, pin partition IDs/layout, ESP writer, bootloader and configuration; include them in level-2 comparison. Do not silently replace the old disk artifact with a root filesystem.

**DESIGN:** use QEMU only during verify/runtime for this candidate backend. CPU boot verification can offer [TCG](https://www.qemu.org/docs/master/system/introduction.html) (`-accel tcg`, a TCG-compatible CPU model) when KVM is absent; current KVM/`-cpu host` arguments need an explicit alternative. This does not qualify GPU passthrough on TCG or remove the later release gate's hardware/KVM requirements. Choose and then measure a separate bounded TCG boot budget.

**DESIGN:** pin the complete e2fsprogs/tool-library closure and exact `mke2fs.conf`; specify block/inode counts, features, reserved space, journal policy, root ownership, label, UUID and hash seed. Derive IDs deterministically from the recipe/profile; set fixed epoch, locale and timezone. Normalize numeric owners, modes, symlinks/hardlinks, xattrs/ACLs/capabilities and directory ordering; handle all relevant inode times. Set `lazy_itable_init=0` and `lazy_journal_init=0`; use a new sparse regular file rather than storage-dependent discard behavior. A fixed timestamp environment does not promise identical output across formatter versions or configurations. Do not strip capabilities or licences to make hashes agree.

**MEASURED, 2026-11-08 ([probe record](../../traces/apps_image_chroot_eval_20261108/probe.json)):** e2fsprogs 1.47.2 produced three equal normalized 32 MiB fixture hashes, including opposite tree creation orders. Each image allocated 4,603,904 bytes locally; seven variants took 10.29 seconds including hashing/checking. Making just one file's mtime older changed the image hash while all file-content hashes stayed equal. Omitting the fixed epoch, UUID or directory hash seed separately changed it too. Seven read-only `e2fsck -f -n` checks left image hashes unchanged. This is one host and tiny fixtures, not full-image or cross-machine evidence.

**CODE READ:** [`mke2fs -d`/hash seed](https://manpages.debian.org/testing/e2fsprogs/mke2fs.8.en.html) provides offline population and explicit hashing control. In [e2fsprogs 1.47.2 population code](https://github.com/tytso/e2fsprogs/blob/v1.47.2/misc/create_inode.c), the fake-time path clamps newer timestamps; older timestamps can remain, explaining why epoch alone is insufficient. The probe's exact config/tool hashes and synthetic UUIDs are recorded. Host root is unnecessary for this regular-file operation, but that says nothing about package bootstrap privileges.

**DESIGN:** eliminate other entropy before claiming level 2: build paths, compiler versions/defaults, native CPU detection, build parallelism and unordered archives; hostname/resolver files copied by bootstrap; apt logs/caches and random identities; Python bytecode paths/timestamps; initramfs hook selection, ordering and compression. Fix the build path/hostname/environment, use prefix mapping where supported, and audit every resulting output. mmdebstrap specifically documents the host hostname/resolver leak in its [epoch discussion](https://manpages.ubuntu.com/manpages/noble/man1/mmdebstrap.1.html). The kernel's [reproducible-build guidance](https://docs.kernel.org/kbuild/reproducible-builds.html) covers build identity, paths and timestamps.

**DESIGN:** remove generated SSH host keys, machine IDs, random-seed files and per-box/cloud-init completion state from the **assembly root** before packing. Declare first-boot identity/key generation and its ordering before sshd starts. Do not normalize by copying one builder's private keys or assigning every runtime instance one machine identity. Fixed filesystem IDs identify a recipe artifact; they are separate from runtime machine identities.

**RECOMMENDATION:** level 1 now means equal PINNED inputs with apps present, not equal compiled bytes; it is not an established result for this backend. Level 2 is optional research for a normalized artifact, not a release gate. Record **unbooted** raw filesystem, kernel and initramfs hashes separately for information. First boot changes identities, keys, logs and filesystem state; verify must protect the original artifact. Identical raw bytes also do not imply identical compressed/qcow2 bytes unless that representation and its tool/library versions are fixed separately.

## Build time, disk and eight independent boxes

**CODE READ, recipe accounting in [recipe audit](../../traces/apps_image_chroot_eval_20261108/recipe_audit.json):** open payloads excluding the unused cloud image are 2,920,084,211 bytes (2.72 GiB). Current non-wheel official compiler inputs total 1,989,158,032 bytes (1.85 GiB compressed); the full separate official list is 4.79 GiB compressed. Apt packages are additional. Its existing 12 GiB apt-download cap is a ceiling, not a forecast. Compiler and installer unpacked sizes are not their compressed download sizes.

**HISTORICAL MEASUREMENT ONLY:** [R5's 2026-11-07 record](../../traces/apps_linux_20261107/README.md) reports the old bundle step at 18.8 minutes on a 30-vCPU box and old guest assembly at 9.1 minutes; that mixed NVIDIA-containing image occupied 20.2 GiB. Those are neither chroot timings nor an open-only size measurement. The old path uses more compile parallelism than this recipe's two-job defaults.

**INFERRED — planning ranges, not measured capacity promises:** assume amd64, two compile jobs, 16 GiB RAM, SSD storage, 1–3 GiB compressed apt downloads, an 8–12 GiB final populated open root, 12–18 GiB active builder/source/object working set, and less than 1 GiB tools overhead:

| Quantity | Initial planning range |
|---|---|
| Cold build, bootstrap/compile/scan/pack | 45–120 minutes per box; download rate and CUDA/C++ compilation dominate uncertainty |
| Warm public-input cache, fresh build roots | 35–100 minutes; this still rebuilds the apps |
| Peak allocated build disk, staged roots sequentially | Approximately 24–36 GiB; target 40 GiB free including margin, subject to complete-build qualification |
| Packed open root | 8–12 GiB used is an estimate; virtual filesystem size is a separate fixed recipe setting with runtime growth allowance |
| Runtime provisioning disk | Additional and separately budgeted: official cache, transient input disk and writable overlay can add 10–20 GiB; never charge these bytes to the open artifact's contents |
| Eight simultaneous cold builds | Approximately 45–120 minutes wall time for the slowest box, 6–16 aggregate box-hours, plus later verification/gates; repeated downloads consume aggregate upstream bandwidth |

**DESIGN:** the smaller-disk target depends on never retaining both roots together: build/export, destroy build root, then assemble/pack. Fetch models/Blender/wheels only for assembly; delete compressed installer/source/apt caches when no longer needed; pack directly with `-d`, without another full root tar. During packing both assembly directory and output data exist: sparse capacity is not zero physical usage. Export checks and licences stay mandatory. Cache reuse is optional and never proof of an independent build.

**DESIGN:** retain the working path's estimated 60 GiB requirement until a complete alternative has established its resource envelope; the 40 GiB target is not a new admission rule. Recompute free-space needs before each phase from actual root/cache/export sizes plus output and reserve. If apt input approaches its 12 GiB cap or roots exceed these assumptions, stop before allocation; do not treat the estimate as permission to fill the box. No full build or privileged bootstrap was run on the shared development machine.

## Cross-box receipt and promotion criteria

**DESIGN:** each box emits a `pinned` part and a `built` part, matching manifest schema 3. Canonical `pinned_inventory_sha256` covers only package names/versions/architectures, pip freeze with wheel hashes, downloaded input artifact URLs/SHA256 pins, source commits/tags, recipe/profile and host-tool source hashes. Downloaded model bytes remain input-pinned. BUILT records every compiled binary, installed-distribution, raw-root/kernel/initramfs and image hash for information. Box aliases, host kernel, timestamps, scan counters and runtime observations are not cross-box identity. Compare the eight PINNED digests and require each declared app to exist; do not compare binary/image hashes. Each box must still independently satisfy clean-scan, media and boot/protocol validation. The implemented `build_image.compare_inventory` compares validated PINNED/BUILT manifest/report objects under this rule. Do not use the chroot's running host `uname` as guest identity.

**DESIGN — falsifiers before qualification:**

1. **Chroot feasibility:** any required bootstrap/package/build step that cannot complete without a running init/device is a counterexample. Record it; fix the declared profile or keep the old backend.
2. **Level 1 across hosts:** two boxes with the same recipe/profile/tools/architecture yielding different PINNED inventory digests falsify the claim. The coordinator requires the expected eight unique successful receipts and compares them with a qualified reference; no majority vote, missing-row pass, or baseline replacement.
3. **Level 2 research only:** different unbooted artifact hashes indicate different bytes and may be localized to metadata, compiler outputs, kernel/initramfs or container representation. They do not fail `verify`, the per-box report or the release comparison.

**DESIGN:** then require CPU boot verification of the actual artifact, the existing NVIDIA provisioning/module checks and the GPU apps gate with the same-binary host reference. Only after two-host builds, all-eight PINNED comparison with apps present, boot compatibility, complete NVIDIA scans and resource-peak qualification may the coordinator switch the default or remove a working path. A cached qualified open artifact uses the identical recipe/profile and verification contract; official NVIDIA payloads remain transient per-run official fetches, never R2/GitHub objects.

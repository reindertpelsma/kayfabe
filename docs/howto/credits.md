# Credits and notices

## Licence

Kayfabe is dual-licensed: you may use, copy, modify and distribute it under **either** the Apache License 2.0 **or** the GNU General Public License version 2 or (at your option) any later version (`Apache-2.0 OR GPL-2.0-or-later`). The texts are [`LICENSE-APACHE`](https://github.com/reindertpelsma/kayfabe/blob/master/LICENSE-APACHE) and [`LICENSE-GPL`](https://github.com/reindertpelsma/kayfabe/blob/master/LICENSE-GPL); [`LICENSE`](https://github.com/reindertpelsma/kayfabe/blob/master/LICENSE) lists the exceptions. <!-- src: LICENSE, README.md -->

Exceptions you should know before building or redistributing: <!-- src: LICENSE -->

- Some files are derived from **gVisor's `nvproxy`** (Copyright The gVisor Authors, Apache License 2.0). They are available under the Apache License 2.0 **only** until they are re-derived independently. `LICENSE` lists them.
- The `kf3-gpu` device is built inside a QEMU source tree, and QEMU as a whole is GPL-2.0. The device links the crates that contain the gVisor-derived files. **A QEMU binary built from this tree is therefore not yet distributable under the GPL.** Read `LICENSE`, exception 2, before redistributing one.
- `third_party/` holds git submodules (the Linux kernel, NVIDIA's open-gpu-kernel-modules and gVisor), each under its own licence. They are reference sources: not built, linked or copied into kayfabe.
- The `broker/` directory (the display broker) is marked `GPL-2.0 OR Apache-2.0`, where the GPL option is version 2 only; either option may be chosen. <!-- src: THIRD_PARTY_NOTICES.md section 6 -->
- A file that carries its own copyright or licence notice is under that notice. Traces, logs and captures contain data produced by third-party software and hardware.

## Third-party notices

[`THIRD_PARTY_NOTICES.md`](https://github.com/reindertpelsma/kayfabe/blob/master/THIRD_PARTY_NOTICES.md) holds the full notices. In short: <!-- src: THIRD_PARTY_NOTICES.md -->

| What | From | Licence |
|---|---|---|
| Struct layouts, class ids, control ids, register offsets and bit fields, **generated** from NVIDIA's headers | NVIDIA [open-gpu-kernel-modules](https://github.com/NVIDIA/open-gpu-kernel-modules) ("ogkm"), pinned as reference sources | MIT |
| Video-engine class method headers | NVIDIA [open-gpu-doc](https://github.com/NVIDIA/open-gpu-doc) | MIT |
| Host display-broker, imported from the author's earlier project | [nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv) | GPL-2.0 OR Apache-2.0 |
| Capability allowlists derived from `nvproxy` | [gVisor](https://github.com/google/gvisor) | Apache-2.0 |
| The QEMU in which the device is built | [QEMU](https://www.qemu.org/) | GPL-2.0 |
| Reference reading only (nouveau, nova) | the Linux kernel, pinned as `third_party/linux` | each file's own header |
| The only external crate in the shipped crates' dependency closure | [libc](https://github.com/rust-lang/libc) | MIT OR Apache-2.0 |

<!-- src: THIRD_PARTY_NOTICES.md sections 1-6, broker/IMPORTED.md -->

Crates used only by tests and developer tools are listed in `Cargo.lock` and are not linked into a shipped artifact. <!-- src: THIRD_PARTY_NOTICES.md section 3 -->

NVIDIA's open-gpu-kernel-modules source is the reference the generated tables are checked against, and the version a given table comes from is recorded with it. <!-- src: third_party/README.md, THIRD_PARTY_NOTICES.md section 1.1 -->

## Related projects

Other projects attempt something close to kayfabe: a GPU for a virtual machine that runs the stock NVIDIA driver while the host keeps using the card. They are credited here for readers who want to compare approaches. Kayfabe does not copy code from them.

- **Caraxes** ([silasmue/Caraxes](https://github.com/silasmue/Caraxes), MIT; a fork of [UniStuttgart-IKR/Leandro](https://github.com/UniStuttgart-IKR/Leandro)). A synthetic NVIDIA GPU served over vfio-user that plays the GSP firmware, shadows the guest's page tables and runs the guest's channels on host channels. It is the closest in shape to kayfabe: like kayfabe, it leaves the host's NVIDIA kernel driver in place and works as a client of it. It makes different design choices elsewhere. Its README describes the project and its current limits. <!-- src: https://github.com/silasmue/Caraxes README -->
- **Open-IOV** ([open-iov.org](https://open-iov.org/index.php/Main_Page)). A community project with the same goal: unmodified NVIDIA drivers inside virtual machines without a vendor vGPU licence. Its site describes replacing the vendor's vGPU licensing and management layer with one host kernel module (`vgsp.ko`) that implements the interface the guest driver expects, the GPU System Processor and its resource-manager firmware. Kayfabe differs in a basic way: it keeps NVIDIA's own host kernel driver and acts as an unprivileged client of it, so the host keeps using the GPU while the VMs share it. The page does not say whether Open-IOV's design lets the host keep using the GPU. Documentation is under CC-BY-4.0 and code under BSD-3-Clause (GPL-2.0 for the kernel modules), per its site. <!-- src: https://open-iov.org/index.php/Main_Page -->

## Trademarks

NVIDIA, GeForce, CUDA and the like are trademarks of their owners. Windows is a trademark of Microsoft Corporation. All other names are the property of their owners. They are used here only to say what kayfabe works with. **Kayfabe is not affiliated with or endorsed by NVIDIA or Microsoft.** <!-- src: docs/NOTICES.md -->

## No vendor binaries

Kayfabe ships no vendor binaries: no NVIDIA or Microsoft driver, firmware, image or installer. Where the tests and guides use them, you obtain them from the vendor yourself. <!-- src: docs/NOTICES.md -->

The Windows harness downloads, each pinned by SHA-256: Microsoft's Windows 11 Enterprise LTSC 2024 evaluation image, the virtio-win 0.1.302 drivers (Red Hat / Fedora project), Win32-OpenSSH 10.0.0.0p2 (Microsoft) and NVIDIA's 580.88 Windows driver. The test tooling fetches other vendors' benchmark and application installers. A draft note in the repository records that the terms of some of these are unclear (for example the Windows evaluation image, the free edition of Unigine Heaven and FurMark) and that some (3DMark, Geekbench scripting) may not be fetched automatically. Read each vendor's terms before you fetch. <!-- src: scripts/bench/windows/win_vm.sh, docs/DOWNLOADS.md (DRAFT 2026-10-23) -->

## Your licences

You must comply with the licences of the drivers and operating systems you run with kayfabe. In particular: <!-- src: docs/NOTICES.md -->

- The NVIDIA driver licence applies to the driver you install on the host and in the guest, including its limits on datacentre deployment and on hosting.
- Windows must be licensed for the way you use it.

Kayfabe's own licence does not grant any rights to third-party software.

## Acknowledgements

- NVIDIA, for publishing the open GPU kernel modules and documentation from which the interfaces are generated (MIT-licensed; notices in `THIRD_PARTY_NOTICES.md`).
- The gVisor authors, for `nvproxy`, from which capability allowlists derive, and for the ioctl formats it documents.
- The nouveau and nova developers and the Linux kernel, read as reference.
- The QEMU project.
- The virtio-win project and Red Hat, for the Windows virtio drivers the harness installs.
- The author's earlier projects `nvkvm-pv` and the archived `nvkvm` prototype (credits in `archive/nvkvm/CREDITS` in the repository), from which the display broker was imported.

<!-- src: THIRD_PARTY_NOTICES.md, third_party/README.md, LICENSE exception 2, broker/IMPORTED.md, scripts/bench/windows/win_vm.sh -->

## Questions about a licence or notice

Open an issue in the repository. Report a suspected vulnerability privately: see [Security model](security.md). <!-- src: SECURITY.md -->

Verified on: not applicable to behaviour; the texts are taken from `LICENSE`, `THIRD_PARTY_NOTICES.md` and `docs/NOTICES.md` as of master (2026-10-26). This page is not legal advice, and the repository marks its own provenance policy wording as a draft awaiting legal review. <!-- src: docs/design/V3_PROVENANCE.md header -->

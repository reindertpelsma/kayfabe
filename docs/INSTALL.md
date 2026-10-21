# Trying kayfabe from source

**STATUS: RESEARCH, 2026-10-22**, based on master `e64b8e5f6` plus D1's documented colour defaults. Interfaces are still experimental. CODE READ describes source checks; MEASURED names existing hardware evidence; **UNVERIFIED** means this lane did not validate the statement on hardware. Every CODE READ statement below is **UNVERIFIED on hardware** for this reduced configuration unless it explicitly cites an existing hardware run. Build/command checks are recorded in [D1 evidence](../traces/install_flags_20261018/README.md).

## Host and guest compatibility

MEASURED (historical revisions, not a new certification of this branch): Linux hosts with the **open NVIDIA kernel modules**, an NVIDIA GPU and KVM run the stock NVIDIA guest driver while the host retains its GPU. The host-driver matrix measured 575.57.08, 580.65.06, 580.95.05 and 580.159.04; the owner's RTX 4070 Windows runs use host 595.91.07 and guest 580.88. See [driver matrix](design/V3_DRIVER_MATRIX.md), [Windows matrix](../traces/win11_userring_20261014/) and [detailed status](STATUS_DETAIL.md). This is a list of tested cells, not a claim that every driver/GPU/OS combination works.

| GPU/guest | Existing MEASURED evidence and limits |
|---|---|
| Linux: Turing TU116, Ampere GA106/GA104/GA102, Ada AD106/AD104, Blackwell GB203/GB205 | Driver/CUDA runs in [status](STATUS_DETAIL.md), [Turing](../traces/v3_turing_master/), [Ada/Blackwell](../traces/v3_families_master/), [RTX 5080](../traces/v3_blackwell/). Display coverage is narrower than CUDA coverage. |
| Windows 11: Ada AD104 (RTX 4070), stock 580.88 | Desktop, D3D/OpenGL/Vulkan and application evidence in [README](../README.md) and [Windows matrix](../traces/win11_userring_20261014/). |
| Other Windows dies | UNVERIFIED as a general install target: README records AD106/AD103 desktop experiments with a borrowed display page, GA104 GPU-only, GB206 still Code 43. |
| Hopper, datacenter Blackwell, GA100 | UNVERIFIED: source coverage is not hardware support; STATUS_DETAIL records GA100/GB10B refusals. |

CODE READ, UNVERIFIED (fresh hardware): QEMU **10.2 or newer** is required by the C device's lockless-I/O compile guard. Guest RAM must be shareable for the device's memory listener. The commands below use `memory-backend-memfd,share=on`. QEMU must have KVM with a kernel irqchip; TCG cannot realize kf3. Available VRAM must cover the real `fb-mb` store and other users; host BAR1 must cover guest apertures plus kayfabe's headroom. See [`kf3.c`](../qemu/hw/misc/kf3/kf3.c) and [`cardbudget.rs`](../crates/kf-qemu/src/cardbudget.rs).

CODE READ, UNVERIFIED (IOMMU-off hardware): kf3 uses the host NVIDIA driver as a client, with **no VFIO binding, PCI passthrough, or per-VM IOMMU group assignment**. Its realize path has no IOMMU-on requirement. The production launcher accepts host group modes DMA-FQ or identity; that does not prove an IOMMU-disabled host. Leave the host's normal NVIDIA/IOMMU setup in place. A guest vIOMMU is absent from the measured ordinary Windows workload configuration; [vIOMMU research](design/V3_VIOMMU_NEED_RESEARCH_20261013.md) lists features outside that scope. kf3 refuses to share a VM with VFIO or other devices that pin its BAR windows.

## Build the device and broker

CODE READ, UNVERIFIED (distribution provisioning): Debian/Ubuntu package names below follow the repo's provisioner and broker Makefile; use equivalent packages on other distributions. The commands pin the audited source change `33f7a1703` for reproducibility; the guide adds documentation after that code revision. The pinned Rust 1.99.0 toolchain in [`rust-toolchain.toml`](../rust-toolchain.toml), including `x86_64-unknown-uefi`, is needed (the GOP ROM embeds a Rust UEFI image). `qemu-img` comes from `qemu-utils`; the build script disables QEMU tools.

```bash
sudo apt-get install build-essential git curl xz-utils python3 python3-venv \
  ninja-build pkg-config libglib2.0-dev libpixman-1-dev libslirp-dev \
  zlib1g-dev ovmf qemu-utils libwayland-dev wayland-protocols \
  libxcb1-dev libxcb-dri3-dev libxcb-present-dev libxcb-render0-dev libxcb-xinput-dev
git clone https://github.com/reindertpelsma/kayfabe.git
cd kayfabe
git checkout 33f7a1703
rustup target add x86_64-unknown-uefi
curl -fLO https://download.qemu.org/qemu-10.2.4.tar.xz
tar -xf qemu-10.2.4.tar.xz
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR="$PWD/target-local"
nice -n 19 bash scripts/bench/build_kf3.sh "$PWD/qemu-10.2.4" "$PWD/qemu-build-kf3"
# Use the exact binary printed in the final KF3_BUILT line.
export QEMU="$PWD/kf3-bins/$(git rev-parse --short=8 HEAD)/qemu-system-x86_64"
"$QEMU" -device kf3-gpu,help
nice -n 19 make -C broker -j2
```

CODE READ, UNVERIFIED (packaging): [`build_kf3.sh`](../scripts/bench/build_kf3.sh) builds `kf-qemu`, overlays the C device into your QEMU source, and installs a per-revision binary beside the build directory. Keep its `pc-bios`/`qemu-bundle` links and build tree with it; copying just the executable can lose firmware. The script may rebuild a directory with a different configure fingerprint. Source builds are for local use; [LICENSE](../LICENSE) records the unresolved QEMU binary distribution issue.

CODE READ, UNVERIFIED (live host display): in a graphical desktop terminal, as the same ordinary user that will run QEMU:

```bash
install -d -m 700 "$XDG_RUNTIME_DIR/kayfabe"
export KF_SOCKET="$XDG_RUNTIME_DIR/kayfabe/display.sock"
./broker/nvkvm-display-broker --socket "$KF_SOCKET" --backend auto --persist
```

CODE READ, UNVERIFIED (live host display): keep that broker running. In a second terminal in the repo, set `QEMU` and `KF_SOCKET` to those same values. kf3 uses the **device's** `display-broker` property; the broker README's `-display nvkvm-broker` example is for a different frontend. No display server connection is needed by QEMU itself. [`broker/README.md`](../broker/README.md) explains backends, credentials and `Ctrl+Alt+G` grab / `Ctrl+Alt+F` fullscreen. Diagnostics such as `--stats-line`, `KF3_PROF`, `KF3_MAPLOG` and `KF3_DIAG_*` are optional; leave them unset.

## Linux guest: one kf3 device

CODE READ, UNVERIFIED (new minimal launch): start with a prepared **x86-64 Linux qcow2 disk** whose NVIDIA driver matches the host version. To install the OS first, add your installer ISO with `-cdrom /path/to/linux.iso -boot d`; no NVIDIA driver is needed for the UEFI framebuffer. Save one OVMF variables file and UUID per VM. `4096` below is an example 4 GiB VRAM allocation; choose a budget your card can provide.

```bash
mkdir -p vm-linux
cp -n /usr/share/OVMF/OVMF_VARS_4M.fd vm-linux/OVMF_VARS.fd
# Set these to your own persistent VM UUID and prepared disk.
export VM_UUID=11111111-2222-4333-8444-555555555555
export LINUX_DISK="$PWD/vm-linux/linux.qcow2"
"$QEMU" -nodefaults -no-user-config -machine pc,accel=kvm,memory-backend=ram0 \
  -object memory-backend-memfd,id=ram0,size=8G,share=on -m 8G -smp 4 -cpu host \
  -uuid "$VM_UUID" \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,file=vm-linux/OVMF_VARS.fd \
  -drive "file=$LINUX_DISK,format=qcow2,if=virtio" \
  -netdev user,id=net0 -device virtio-net-pci,netdev=net0 \
  -device qemu-xhci,id=xhci -device usb-tablet,bus=xhci.0 -device usb-kbd,bus=xhci.0 \
  -display none -serial stdio \
  -device "kf3-gpu,fb-mb=4096,display=on,gop=on,x11-dispsw=on,display-broker=$KF_SOCKET"
```

CODE READ, UNVERIFIED (fresh install): install your distribution's **stock NVIDIA open driver** package, or NVIDIA's stock `.run` installer with matching kernel headers, then reboot and check guest `nvidia-smi`. Use the same version as the host to omit `guest-driver`. For a different measured Linux version add `guest-driver=<exact-version>`; an ABI mismatch is refused. Historical CUDA ladders passed guests 575.57.08, 580.105.08, 580.159.04, 590.48.01, 595.84 and 610.57.04 in the [driver matrix](design/V3_DRIVER_MATRIX.md); these do not establish every desktop/display cell. No guest helper or patched kernel is required by this path.

## Windows 11 guest: current research configuration

CODE READ, UNVERIFIED (fresh hardware): **the flag-free Windows default is not complete on this branch**. The production launch still selects runlist/probe behavior, some awaiting an owner ruling or source derivation. [The inventory](design/V3_FLAG_INVENTORY_20261022.md) gives the blocking names and reasons. Promoting those to defaults without that work would hide the gap. The template below preserves those executable production selections, minus the three colour options that D1 promotes and dead lifecycle/timer names. UNVERIFIED: it has not been booted on hardware as a reduced command.

CODE READ, UNVERIFIED (fresh install): use a per-VM disk/UUID/MAC and regular, non-Secure-Boot OVMF for the unsigned embedded GOP. For a new Windows 11 install, also provide a TPM 2.0 emulator (`swtpm`) and Windows/virtio installer media as described in [`scripts/bench/windows/README.md`](../scripts/bench/windows/README.md). The following template is for an **already installed** Windows 11 disk with virtio storage/network drivers and the registry setup below; it is not a complete Windows installer command. Windows Secure Boot requires the signed, matching `gop-efi` path documented by that README; do not use an arbitrary firmware image.

```bash
mkdir -p vm-windows
cp -n /usr/share/OVMF/OVMF_VARS_4M.fd vm-windows/OVMF_VARS.fd
export VM_UUID=66666666-7777-4888-8999-aaaaaaaaaaaa
export WINDOWS_DISK="$PWD/vm-windows/windows.qcow2"
env KF3_WIN_USER_CHANNELS_PASSTHROUGH=1 KF3_WIN_TWIN_DEFAPI_OBJECT=1 \
  KF3_KERNEL_GR_CE=1 KF3_KERNEL_GR_WORK=1 KF3_TRANSLATED_CE_RELAY=1 KF3_DEFERRED_API=1 \
  KF3_PREEMPT_BIND_PROBE=1 KF3_DISPLAY_IMP_ENABLE=1 KF3_DISPLAY_CTRL_PROBE=1 \
  KF3_ZCULL_BIND_PROBE=1 KF3_SW_RUNLIST_HOST_OWNED=1 KF3_ASYNC_PREEMPT=1 \
  KF3_GFX_POOL_PROBE=1 KF3_SW_RUNLIST_PROBE=1 KF3_MEMORY_LIST_PROBE=1 KF3_DISPLAY_CAPS_PROBE=1 \
  "$QEMU" -nodefaults -no-user-config -machine pc,accel=kvm,smm=on,memory-backend=ram0 \
  -object memory-backend-memfd,id=ram0,size=12G,share=on -m 12G -smp 8 -cpu host,-vmx \
  -uuid "$VM_UUID" \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,file=vm-windows/OVMF_VARS.fd \
  -drive "file=$WINDOWS_DISK,format=qcow2,if=virtio,cache=none,aio=threads" \
  -netdev user,id=net0 -device virtio-net-pci,netdev=net0,mac=52:54:00:12:34:56 \
  -device qemu-xhci,id=xhci -device usb-tablet,bus=xhci.0 -device usb-kbd,bus=xhci.0 \
  -global i440FX-pcihost.pci-hole64-size=32G -display none -serial stdio \
  -device "kf3-gpu,bar1-size=8589934592,guest-driver=580.65.06,display=on,gop=on,display-broker=$KF_SOCKET"
```

CODE READ, UNVERIFIED (omissions): `fb-mb=8192` and `bar2-size=33554432` are exact device defaults. `gpu-uuid=auto` derives a stable guest GPU UUID from the persistent `-uuid`, host GPU and slot; an explicit guest GPU UUID is optional by source contract. A same-user broker needs no `display-broker-uid`; set it to the broker user's numeric uid across users. `bus`, `addr` and `id` are unnecessary for automatic PCI placement of one device. Windows keeps the production 8 GiB `bar1-size`: replacing it with the 256 MiB default needs hardware validation. Linux uses that default in this template. The [inventory](design/V3_FLAG_INVENTORY_20261022.md) covers every property individually.

MEASURED (580.88 on AD104, historical runs): install NVIDIA's **stock Windows 580.88 driver**, enable GSP at both the adapter and service registry keys, and apply the documented DWM overlay setting. Follow [`WINDOWS_GUEST_SETUP.md`](WINDOWS_GUEST_SETUP.md) and [`gsp_on.ps1`](../scripts/bench/windows/gsp_on.ps1); discover the adapter key rather than assuming `0000`. A driver installed without GSP enablement can report Code 43. UNVERIFIED: other Windows driver branches are not an install-support claim.

## Limits and troubleshooting

- MEASURED (historical): [Windows sleep/wake](design/V3_S3_RESUME.md) does not survive; four Linux app-matrix failures need UVM demand paging. README records long playback resets and narrower Windows die coverage. The [security audit](OWNER_RULINGS.md#p-a-security-audit-before-the-first-public-binary-2026-10-03) remains a release gate; no audited isolation claim follows from these launch examples.
- CODE READ, UNVERIFIED (your host): if realize reports VRAM/host BAR1 budget refusal, reduce `fb-mb`/`bar1-size` or close allocations belonging to you. The device intentionally refuses an unbacked advertised resource. Check host `nvidia-smi` and the exact QEMU error; do not change timers.
- CODE READ, UNVERIFIED (your host): collect QEMU/broker logs, host `journalctl -k` lines containing `NVRM`/`Xid`, host/guest driver versions, GPU die and source revision. A host Xid is evidence of a GPU/driver fault, not proof of its cause. Do not declare containment from one successful guest.
- CODE READ, UNVERIFIED (your guest): for a TDR/Code 43, first check the GSP registry keys and `guest-driver` ABI, then the first `USER_RING_REFUSED`/display refusal and Windows System log events 153/4101. Increasing TDR/timeouts hides a symptom; this install path does not prescribe it. Keep the original workload and logs for a report.

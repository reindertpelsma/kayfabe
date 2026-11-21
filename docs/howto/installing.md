# Install

If you can already give a virtual machine a GPU with VFIO, you can use kayfabe in four steps. The difference is that the host keeps using the GPU, and the VM gets an emulated NVIDIA GPU that runs on the real one.

1. Get the kayfabe QEMU: download a release, or build it.
2. You now have a `qemu-system-x86_64` binary that contains the `kf3-gpu` device.
3. Install your own Windows or Linux guest, with NVIDIA's own driver, the way you would for VFIO.
4. Start the VM with `-device kf3-gpu` where you had the VFIO device.

What the host needs is KVM and permission to open the NVIDIA device nodes. Kayfabe is not a kernel module and needs no VFIO rebind, no IOMMU group and no change to the host's NVIDIA driver. See [Requirements](requirements.md) for the supported GPUs and driver versions.

## Step 1. Get the kayfabe QEMU

**Download.** No release binary exists yet. The page for it is created at the first release. Whether and when a binary is published is the project owner's decision, not a licence limit of the source ([Credits and notices](credits.md)).

**Build it yourself.** [Building from source](building.md) has every step. Once the Rust toolchain and the build packages are installed, the three commands that produce the binary are:

```sh
python3 -I scripts/ci/qemu_source.py fetch .          # QEMU 10.2.4, checked against a pinned hash
tar -xf qemu-10.2.4.tar.xz
bash scripts/bench/build_kf3.sh "$PWD/qemu-10.2.4"
```

The last command prints a line `KF3_BUILT <path> rev=<revision> ...`. The path is your binary. Keep its `pc-bios` folder beside it; the build puts it there.

Other QEMU versions than 10.2.4 are not documented. The device is built into the QEMU binary; there is no loadable module yet.

## Step 2. What you have

A normal `qemu-system-x86_64` of QEMU 10.2.4 with one more device type, `kf3-gpu`. A VM without a `kf3-gpu` argument is plain QEMU. [Command-line reference](cli-reference.md) lists every property of the device.

## Step 3. Install your guest and its driver

Install the guest the way you always do, from the vendor's installer, on whatever QEMU or libvirt setup you use. Kayfabe patches nothing in the guest.

- **Linux.** Install NVIDIA's driver in the guest (the `.run` installer or your distribution's package). Every recorded run used the open kernel module flavour. Nothing else is set in the guest. See [Run a Linux guest](linux-guest.md).
- **Windows.** Install NVIDIA's driver with NVIDIA's installer, and apply the documented guest settings with the script `scripts/windows/kayfabe-windows-setup.bat` (it starts `kayfabe-windows-setup.ps1` beside it): run it in the guest once before NVIDIA's driver installer (phase `pre`) and once after it (phase `post`), then restart. Without the settings the driver stops with Code 43. See [Run a Windows guest](windows-guest.md).

**Which driver version.** A guest driver close to the host's driver is recommended and not required. If you leave the device property `guest-driver` unset, the device assumes the guest runs the host's driver version and adjusts to the version the guest driver announces when both have the same layout for the early start-up messages. A guest on another driver branch than the host needs `guest-driver=<the guest driver's version>`. If that is needed, QEMU's log says so by name; see [Troubleshooting](troubleshooting.md#the-guest-driver-and-the-host-driver-do-not-match). A pair of host and guest versions that kayfabe has not measured is refused by name rather than guessed.

## Step 4. Start the VM with kayfabe

Take the command line you use with VFIO and change two things: the device, and the guest RAM, which must be a shared memory file.

With VFIO:

```
qemu-system-x86_64 -machine q35,accel=kvm -cpu host -m 8192 ... \
    -device vfio-pci,host=0000:01:00.0
```

With kayfabe:

```
kayfabe-qemu -machine q35,accel=kvm,memory-backend=ram0 -cpu host -m 8192 ... \
    -object memory-backend-memfd,id=ram0,size=8192M,share=on \
    -uuid <a fixed uuid for this VM> \
    -device kf3-gpu,id=kf0,fb-mb=8192,bar1-size=134217728,bar2-size=33554432
```

`kayfabe-qemu` stands for the binary from step 2. The VM must not contain a VFIO device for the same GPU. [Command-line reference](cli-reference.md) says what each argument does and what the others do. If you already have a VM, [Attach to a VM you already have](attach-to-existing-vm.md) lists exactly what changes, including libvirt notes.

- `gpu-minor=<n>` picks the host GPU (the number in `/dev/nvidia<n>`); the default is 0. One kayfabe device per host GPU.
- `fb-mb` is real video memory taken from the host card; on a card with less than 10 GiB use the card's memory minus 2048 MiB.
- `display=on` gives the guest a virtual monitor; leave it off for a headless GPU. See [Display and the broker](display.md).

What is not verified yet: that an unmodified VFIO guest image boots with only the device swapped. The aim is that it does for Linux; the equivalence test that boots one image on both is not in the tree yet.

## Permissions

QEMU must be able to open `/dev/kvm`, `/dev/nvidiactl`, `/dev/nvidia<n>` and `/dev/nvidia-uvm`. Run it as an ordinary user. One run as a throwaway user whose only group was `kvm` passed the Linux test suite and a Windows guest; it relied on NVIDIA's default mode 0666 for the `nvidia*` nodes and was recorded on one host. See [Security model](security.md). `nvidia-modprobe -c 0 -u` creates missing nodes.

## Check that it works

- QEMU's log has a line that starts with `kf3: host GPU minor=0 ...` and names the card. A refusal names its cause; see [Troubleshooting](troubleshooting.md).
- In a Linux guest, `nvidia-smi` names the card.
- In a Windows guest, `nvidia-smi` works and the display adapter has no error in Device Manager.

If something does not start, [Troubleshooting](troubleshooting.md) lists the messages. If you find a defect, [Report a problem](report-a-problem.md) says what to attach.

## Stopping a guest

Shut the guest down from inside it, or send ACPI power-down through QEMU (`system_powerdown` on the monitor). Wait for the QEMU process to exit before you start another VM on the same GPU: the card's memory is released a moment after QEMU exits.

## Stock drivers, nothing patched

Stock host and guest NVIDIA drivers are unmodified by default. Kayfabe ships no NVIDIA binaries. The repository also contains opt-in research code (host-driver experiments, guest instrumentation, a diagnostic QEMU observer) that is never part of the build or the install above; a run with an instrumented driver does not establish a stock-driver result. `python3 scripts/ci/no_patch_check.py` prints the complete table of patches in the repository and checks that no stock path applies one. The device itself is part of the custom QEMU build described in [Building from source](building.md): "stock drivers" does not mean an unmodified distribution QEMU.

## Not available yet

- A downloadable release (see step 1).
- Distribution packages, container images, systemd units and sandbox profiles. The security model recommends confining the QEMU process; see [Security model](security.md).
- Proxmox and libvirt are not tested; [Attach to a VM you already have](attach-to-existing-vm.md) has a libvirt sketch.
- Several VMs on one GPU as a supported configuration.

## Appendix. Optional helper scripts

The repository's `scripts/bench/` directory holds the scripts the maintainers use to test on rented, disposable machines. **None of them is needed to install or run kayfabe**, and they are not a supported installer. They are listed so that you know what exists.

> **Warning.** The provisioning scripts purge NVIDIA driver packages, stop the host's display manager, load and unload kernel modules, create a tap network device, change `ip_forward` and add `iptables` rules, and they run as root. They are written for throwaway test machines. Do not run them on a machine you care about; read them first.

| Helper | What it does |
|---|---|
| `scripts/bench/provision_host_driver.sh` | Installs the open NVIDIA kernel module on a test host (default version 580.159.04). |
| `scripts/bench/provision_bench_tree.sh` | Prepares a test tree: QEMU build, a guest disk, a tap network, the guest-side test client. |
| `scripts/bench/display/interactive.sh` | Starts a Linux desktop guest in a broker window ([Display and the broker](display.md)). |
| `scripts/bench/windows/win_vm.sh` | Installs and boots a Windows 11 test guest ([Windows test harness](windows-harness.md)). |
| `scripts/bench/v3_gates.sh`, `scripts/fastguest/fast_suite.sh` | The GPU gates and the 30-arm suite ([Test it yourself](test-yourself.md)). |

The launch scripts take an exclusive lock, `/tmp/kayfabe-fastguest.lock`, because GPU test runs must be strictly serial.

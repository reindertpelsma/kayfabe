# Run a Linux or Windows guest

STATUS: BETA CANDIDATE / HARDWARE WALK-THROUGH PENDING, 2026-10-20.
No external support path. Follow [INSTALL](INSTALL.md) and doctor first;
[historical support evidence](SUPPORT_MATRIX.md) is not C6 acceptance.

CODE READ: `guest-os=linux` is the direct-device default; Windows requires only
`guest-os=windows` to select its compatibility behaviour. No KF3 environment
flags are set by the launcher. ABI version, display and ordinary VM properties
are separate from those retired environment selectors. Guest driver must be
explicit in this launcher: pre-fn1 layout differences make host-only guessing
unsafe. Windows580.88 uses the Linux ABI twin580.65.06.

## A new Linux guest

1. Obtain an x86_64 ISO from [Ubuntu](https://ubuntu.com/download/desktop) and
   verify its published checksum. The current ISO is not a tested C6 guest.
2. Create the user's own disk with the system qemu-img utility.
3. Start the broker in the desktop session; preview, then run the launcher.
4. Install Linux, remove --iso on subsequent starts, and install an accepted
   stock NVIDIA guest driver. Use the exact matching guest-driver version.
   Linux needs no Windows registry changes. This new-guest flow awaits Stage 5.

```sh
qemu-img create -f qcow2 "$HOME/linux.qcow2" 80G
mkdir -p "$XDG_RUNTIME_DIR/kayfabe"
"$HOME/.local/opt/kayfabe-beta/bin/nvkvm-display-broker" \
  --socket "$XDG_RUNTIME_DIR/kayfabe/broker.sock" --backend auto
```

CODE READ: run the broker in one terminal, the following in another. The ISO
filename is operator-supplied. `--print` performs read-only discovery and prints
the exact generated argv with shell quoting; it writes no VM state and launches
no TPM/QEMU. GPU-free tests use offline facts, never the real device.

```sh
"$HOME/.local/opt/kayfabe-beta/bin/kayfabe-run.sh" \
  --disk "$HOME/linux.qcow2" --guest-driver 580.159.04 \
  --broker "$XDG_RUNTIME_DIR/kayfabe/broker.sock" --iso /path/to/linux.iso --network on --print
```

CODE READ: remove --print to launch. --ram defaults8G and --cpus defaults8.
The candidate retains QEMU's --no-reboot: a guest reboot exits QEMU. Run the same
launcher command again to continue installation; remove the ISO after setup.
The existing disk, UEFI variables and TPM state are reused.
--no-display selects headless Linux; use a guest configured for serial access
or networking. The minimal direct device is `-device kf3-gpu,id=gpu0` plus normal
QEMU VM arguments, but its legacy8GiB/256MiB/32MiB resource defaults do not fit
every card. The launcher derives safer host-sized arguments, as described below.

## A new Windows guest

1. Obtain your licensed x64 ISO from [Microsoft](https://www.microsoft.com/en-us/software-download/windows11).
   Download a signed virtio driver ISO using [virtio-win's installation guide](https://virtio-win.github.io/Knowledge-Base/Driver-installation.html).
   Neither Windows nor NVIDIA installers are redistributed here.
2. Create an empty qcow2 disk; start the same-user broker. The launcher uses
   UEFI and a private copy of OVMF variables, plus its own swtpm child/state.
3. Preview the command. Run without --print, load virtio storage/network drivers
   from the second ISO during setup, and finish OOBE offline.
4. Obtain stock Windows580.88 from [NVIDIA's driver downloads](https://www.nvidia.com/en-us/drivers/).
   Follow [WINDOWS_GUEST_SETUP](WINDOWS_GUEST_SETUP.md) to enable GSP in the adapter
   and service keys before first driver start, set the documented DWM workaround,
   and reboot. Relaunch the same command after the reboot exits QEMU. These are
   guest driver prerequisites, not host environment flags.
5. Remove both --iso options after installation. --network on explicitly enables
   outbound slirp after offline setup. Sleep/wake remains unsupported. All C6
   boot/workload claims await Stage 5; do not change TDR timers as a workaround.

```sh
qemu-img create -f qcow2 "$HOME/windows.qcow2" 100G
"$HOME/.local/opt/kayfabe-beta/bin/kayfabe-run.sh" \
  --os windows --disk "$HOME/windows.qcow2" --guest-driver 580.65.06 \
  --iso /path/to/windows.iso --virtio-iso /path/to/virtio-win.iso --print
```

CODE READ: the Windows device fragment is
`-device kf3-gpu,id=gpu0,guest-os=windows,guest-driver=580.65.06,display=on,gop=on`
plus host-derived aperture/store and broker socket/uid properties. It does not
require P19 env settings. The launcher supplies unique per-host/disk VM UUID,
SMBIOS serial and MAC; GPU UUID uses the device's existing per-VM auto derivation.
No host-owned launcher is changed. Existing explicit QEMU property values remain
accepted by the device; --fb-mb/--bar1-size expose resource overrides.

## What the generated arguments do

| Argument group | Purpose / optionality |
|---|---|
| KVM, in-kernel irqchip, CPU host,-vmx | Required virtualization path; no nested VMX exposure |
| Shared memory-backend-memfd | Required guest RAM mapping model |
| fb-mb | VRAM minus2GiB, rounded down to GiB, capped8GiB; allocation still may refuse |
| bar1-size / bar2-size | Largest power-of-two BAR1 fitting BAR2(32MiB)+PRAMIN(1MiB)+headroom(16MiB)+optional GOP overlap |
| guest-driver | Explicit measured ABI layout; no nearest-version fallback |
| display/GOP/broker/uid | Optional for headless Linux; default visible Windows path; same desktop uid |
| OVMF / private VARS / TPM | UEFI boot; TPM child for Windows; independent per VM |
| virtio disk + iothread | Disk IO outside vCPU; qcow2 format is explicit |
| user network | Offline by default; --network on explicitly enables outbound slirp |
| xHCI tablet/keyboard | Absolute input and keyboard via broker |
| QMP / serial log | Per-VM local diagnostics; QGA is optional and not required by this launcher |

CODE READ: directly changing unset QOM defaults would violate the binding Linux
baseline requirement. Host-derived resource values are supplied by the launcher
instead. Socket defaults to `$XDG_RUNTIME_DIR/kayfabe/broker.sock`, uid to the
invoking user, UUID to existing device auto derivation. No pacing/tick source,
cursor or timeout policy is changed. The launcher reports cgroup limitations;
it does not promise host memory isolation when memory.max is unlimited.

## Display without a broker

CODE READ: the retained QEMU console can show the display without a broker.
For that diagnostic path, take the printed QEMU line, retain device
`display=on,gop=on`, remove `display-broker` and `display-broker-uid`, and use
`-display vnc=127.0.0.1:1` in place of `-display none`. Connect a local VNC viewer
to port5901. This is the [QEMU VNC frontend](https://www.qemu.org/docs/master/system/invocation.html),
not a launcher mode; the build enables VNC and disables GTK/SDL. Keep the listener
on loopback. The console/screendump fallback retains its existing pacing and
cursor behavior; no C6 performance or desktop acceptance is claimed for it.
`--no-display` instead disables the display device path for headless Linux.
The [display design](design/V3_DISPLAY.md) distinguishes the diagnostic console
from the product broker path. Both real display paths wait for Stage5 retest.

## Troubleshooting and evidence

CODE READ: stderr prints `kf3: kayfabe`, `effective-config guest-os=...`, derived
caps die/branch/unmeasured notices and named realization refusals. Save stderr
to a private file when reporting. Serial log and QMP/TPM/UEFI state are under the
disk's `.kayfabe/<vm-id>` directory; the launcher does not delete user disks.
Inspect doctor FAIL first. Mixed versions require reinstalling one complete
prefix. NoMemory needs a smaller --fb-mb or less concurrent GPU memory use;
BAR1-budget refusal needs a smaller aperture. Unmeasured driver/layout/caps rows
are not repaired with neighboring values or timers. A broker session failure
requires checking socket ownership and the actual graphical session.
`--stats-line` is optional broker measurement and off by default.

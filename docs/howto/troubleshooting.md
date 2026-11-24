# Troubleshooting

This page lists symptoms that the repository documents, with the cause and the action it records. It is not exhaustive. Where the repository does not explain a symptom, the page says so. <!-- src: scripts and documents named per entry -->

**First rule: if the same workload passes on the host with no VM and fails in the guest, the defect is kayfabe's.** Run the equivalent program on the host first; only a failure there indicts the machine. If you find a defect, [Report a problem](report-a-problem.md) says what to attach.

## The messages you will meet

Kayfabe refuses a configuration by name rather than guessing. The most common messages and where they are explained:

| You see | What it means |
|---|---|
| `kf-host: HOST-ABI REFUSED <what>: <reason>` in QEMU's log | Kayfabe asked the host driver a question that the host driver's version cannot carry, and did not ask it. A few such lines at VM start are harmless; see [below](#host-abi-refused). |
| `the guest driver says it is "..." but this device's layouts were selected for ... - set the device property guest-driver=...` | The guest's driver is not the version the device assumed. See [The guest driver and the host driver do not match](#the-guest-driver-and-the-host-driver-do-not-match). |
| The NVIDIA display adapter shows Code 43 in a Windows guest | The Windows guest settings are missing. See [Windows guest](#windows-guest) below and [Run a Windows guest](windows-guest.md). |
| The guest has a GPU but no screen | The display is off by default. See [No display](#no-display-in-the-guest). |
| `kf3: a virtio-balloon device is in this VM; kf3 refuses to run beside a memory balloon, ...` | Remove the balloon: drop `-device virtio-balloon-pci`, or set `<memballoon model='none'/>` in libvirt. |
| `kf3: requires -accel kvm` | The VM does not run under KVM. |
| `no fd-backed guest RAM (the VM needs memory-backend-memfd,share=on)` | Guest RAM must be a shared memfd; see [Command-line reference](cli-reference.md). |
| `store of N MiB refused: NoMemory` | `fb-mb` is larger than the card can give. |

### HOST-ABI REFUSED

Kayfabe builds every request to the host's NVIDIA driver itself, from the layouts of the host driver's own version. When a request cannot be expressed in the host driver's layout, kayfabe does not send it and prints one line (rate-limited):

```
kf-host: HOST-ABI REFUSED <what>: <reason>
```

The reason is one of: the host driver's version is not a measured version (`host driver X is not a measured tag of the driver matrix; kayfabe carries host structs only between measured layouts`); the request sets a field that the host driver's version does not have (`... has no [...], and the request sets them — the host cannot be asked this; refusing rather than letting it read the bytes as another field`); or a layout could not be carried to or from the host driver's layout.

- **A handful of such lines while the VM starts, and the guest then works,** is expected. On one recorded run on a 595.91.07 host, seven came at start-up from kayfabe's own questions about things that driver version does not have; none came from a guest request and the guest was unaffected.
- **A host driver that is not a measured version** is refused every time. Use a version listed in [Requirements](requirements.md).
- **Lines that repeat while a guest program fails** are a defect or an unsupported pair of versions. [Report a problem](report-a-problem.md) with the log and both driver versions.

A guest whose own control layout differs from the host's, refused by this mechanism end to end, has not been run on hardware.

### The guest driver and the host driver do not match

By default the device assumes the guest runs the host's driver version, and switches to the version the guest driver announces when it starts if the two versions agree on everything the guest had already used. The guest driver is always checked against the version the device is using.

- If the guest's driver is on another branch than the host's (for example a 580 guest on a 595 host), QEMU's log shows:

  ```
  the guest says it is driver X; cannot re-select from Y (the version the device defaulted to): ... differs and the guest already consumed it. To serve this guest add guest-driver=X to the kf3-gpu device
  ```

  Add exactly the `guest-driver=X` the message prints to the `kf3-gpu` device and start the VM again. For a Windows driver the value is the Linux tag of the same driver build; the Windows harness passes `580.65.06` for Windows driver 580.88.
- If you declared `guest-driver=` and it does not match the guest, the log says `the guest driver says it is "X" but this device's layouts were selected for Y ... set the device property guest-driver=X`: declare the version it names. A declared version is never overridden.
- A guest driver version that kayfabe has not measured is refused by name (`driver X was never measured (looking up ...)`). Use a measured version ([Requirements](requirements.md)).
- A guest driver version that does not parse: `guest driver version "..." does not parse (want major.minor[.patch])`.

A driver close to the host's is recommended. The message text is read from the code; a run of such a pair on hardware has not been recorded.

### No display in the guest

The virtual display is off by default: without `display=on` the guest has a GPU for compute and offscreen rendering and no monitor. For a screen, give the device `display=on` (and `gop=on` for the boot screens), remove the VM's other VGA with `-vga none`, and view it through VNC or the display broker ([Display and the broker](display.md)). The device refuses a few combinations by name: `kf3: display-broker needs display=on (the broker shows the virtual display)`, `kf3: display-broker-vram needs display-broker (it is the broker's GPU-copy rung)`, and `gop=on` with its own ROM conflicts: `kf3: gop=on refused: ... (drop the property, or set gop=off; ...)`.

## Collect evidence before you change anything

- **The host's kernel log.** A host GPU fault does not raise anything in the guest: the host driver services it, kills the offending channel and prints an `NVRM: Xid` line in the host's kernel log. Read it with `dmesg | grep -i xid` (or `journalctl -k | grep -i xid`). Without watching that log, "the copy silently did nothing" and "the GPU faulted" look the same. The helper `bash scripts/bench/host_xid_watch.sh <tag> -- <command>` runs a command and exits 6 when a new host Xid appeared across it and 7 when it could not read the log. <!-- src: scripts/bench/host_xid_watch.sh -->
- **QEMU's own log.** Start QEMU with its standard error kept in a file; kayfabe's messages (the `kf3:`, `kf-host:` and `kf-rm:` lines) are there. <!-- src: crates/kf-qemu/src/device.rs, crates/kf-host/src/lib.rs -->
- **The guest driver's own log:** `dmesg` in a Linux guest, the System log in a Windows guest. It is not in the serial log unless you capture it. <!-- src: scripts/bench/boot_capture.sh -->
- **Is the job alive?** A killed job and a running job look the same if you only check for missing output. Check that the process is alive; `pgrep -x qemu-system-x86_64` never matches, because the process name is truncated to 15 characters. <!-- src: scripts/bench/boot_capture.sh -->
- **Which binary ran.** The device is linked into the QEMU binary, so a source change that was never relinked runs the old device. Check that you started the binary the build printed. <!-- src: scripts/fastguest/run_fast_guest.sh -->

## Building

| Symptom | Cause and action |
|---|---|
| `error[E0463]: can't find crate for std` on a crate unrelated to musl | The musl target is missing for the pinned toolchain: `rustup target add --toolchain 1.99.0 x86_64-unknown-linux-musl`. <!-- src: scripts/bench/provision_box.sh --> |
| `kf-gop-image: the Rust standard library for x86_64-unknown-uefi is not installed ...` | Add the target as the message says: `rustup target add x86_64-unknown-uefi --toolchain <pinned>`. <!-- src: crates/kf-gop-image/build/support.rs --> |
| `cargo not on PATH` from `build_kf3.sh` | `export PATH=$HOME/.cargo/bin:$PATH`. <!-- src: scripts/bench/build_kf3.sh --> |
| `<dir> is not a QEMU source tree` | Pass the extracted QEMU 10.2.4 source directory (it must contain a `VERSION` file). <!-- src: scripts/bench/build_kf3.sh --> |
| `kf3_gop.h: No such file or directory` (an old failure) | The overlay copy takes every `*.c`, `*.h` and `meson.build`; use the current `build_kf3.sh`. <!-- src: scripts/bench/build_kf3.sh --> |
| `no kf3 binary for this checkout's revision ...` from a launcher | Run `build_kf3.sh` from this checkout, or set `QEMU_BIN`. <!-- src: scripts/fastguest/run_fast_guest.sh --> |
| `<qemu> has no tpm-crb: rebuild with scripts/bench/build_kf3.sh` | The binary was built before `--enable-tpm` and `--enable-slirp` were added to the configure line. Rebuild. <!-- src: scripts/bench/windows/win_vm.sh --> |

## The device refuses to start

The device checks its configuration when QEMU starts and refuses by name rather than clamping silently. If QEMU prints nothing and the serial log is empty, read the QEMU log first: the harness reports `FAST_VERDICT=CRASH (the serial log is EMPTY ...)`, usually because a concurrent run killed it or QEMU failed to start. <!-- src: scripts/fastguest/run_fast_guest.sh, crates/kf-qemu/src/cardbudget.rs -->

| Message or symptom | Meaning and action |
|---|---|
| `store of N MiB refused: NoMemory` | `fb-mb` is larger than the card can give. Lower it. On the 8 GiB RTX 3060 Ti the harness script records that 6144 MiB boots; the bench's rule for cards under 10 GiB is (VRAM - 2048 MiB), rounded down to whole GiB. <!-- src: scripts/fastguest/run_fast_guest.sh --> |
| `a guest BAR1 of 256 MiB does not fit: 256 MiB + 16 MiB headroom > 256 MiB host aperture. The largest that fits here is 128 MiB` | Lower `bar1-size` (the harnesses use 128 MiB). The sum of guest BAR1, BAR2, the 1 MiB PRAMIN window and 16 MiB headroom, over all kayfabe devices on the card, must fit the host BAR1. <!-- src: scripts/fastguest/run_fast_guest.sh, crates/kf-qemu/src/cardbudget.rs --> |
| `the GPU still holds N MiB after 30 s - a previous VM did not release it` | The previous VM's GPU memory outlives its QEMU for a moment. Wait until `nvidia-smi` shows the GPU idle (`KF_GPU_IDLE_MIB`, default 512), or the store reservation will likely fail. <!-- src: scripts/fastguest/run_fast_guest.sh --> |
| `host driver X is not a measured tag of the driver matrix; kayfabe carries host structs only between measured layouts` | The host driver version is not one the repository measured. Use a version in [Requirements](requirements.md). <!-- src: crates/kf-abi/src/hostabi.rs --> |
| `driver X was never measured (looking up ...)` or `guest driver X: ...` | The same, for the guest version declared by `guest-driver=` (or defaulted to the host's). A release between two measured tags can move a field, so kayfabe refuses rather than borrow a neighbour's layout. <!-- src: crates/kf-abi/src/matrix.rs, crates/kf-qemu/src/device.rs --> |
| `guest driver version "..." does not parse (want major.minor[.patch])` | Fix the `guest-driver=` value. <!-- src: crates/kf-qemu/src/device.rs --> |
| `the guest driver says it is "..." but this device's layouts were selected for X - set the device property guest-driver=...` | The guest's driver version (read when its driver starts) differs from the declared one, and kayfabe refuses `SET_GUEST_SYSTEM_INFO` rather than answer with another release's layouts. Declare the version the message names. A Windows driver is keyed as its Linux tag; the harness's example is `580.65.06` for Windows driver 580.88. <!-- src: crates/kf-rm/src/guestsysinfo.rs, scripts/bench/windows/README.md --> |
| `the guest says it is driver X; cannot re-select from Y (the version the device defaulted to): ... differs and the guest already consumed it. To serve this guest add guest-driver=X to the kf3-gpu device` | You did not set `guest-driver=`, so the device defaulted to the host's driver version Y, and the guest's driver X belongs to a branch whose first messages differ from Y's (for example a 580.x guest on a 595.x host, or the reverse). kayfabe never lets the guest choose the host side, so it refuses by name. Add exactly the `guest-driver=` the message prints (the guest's version) to the `kf3-gpu` device. <!-- src: crates/kf-rm/src/lib.rs --> |
| `kf3: requires -accel kvm` | The VM is not running under KVM. The device does not work with TCG or another accelerator. <!-- src: qemu/hw/misc/kf3/kf3.c --> |
| `kf3: RAM discard is already disabled by something that pins guest RAM (a VFIO, iommufd or vfio-user device, an nvme:// drive, ...)` | Another device in the same VM pins guest RAM. Remove it; kayfabe refuses to share a VM with one. <!-- src: qemu/hw/misc/kf3/kf3.c --> |
| `channel-budget` refused | Below 256, or above what the host can give (or 2048). Leave it at `0`. <!-- src: qemu/hw/misc/kf3/kf3.c --> |
| `display-max-fps` refused | Outside 24..75, or without `display=on`. <!-- src: qemu/hw/misc/kf3/kf3.c --> |
| `/proc/driver/nvidia/version` lacks "Open Kernel Module" | The host has the closed module loaded, even if the installer exited 0. The installer's exit code is not evidence; check that string. <!-- src: scripts/bench/provision_host_driver.sh --> |
| "installation was canceled due to ... an alternate driver installation" | The distribution's `nvidia-*` packages are held; unhold and purge them first. This message points at the wrong cause. <!-- src: scripts/bench/provision_host_driver.sh header --> |
| Realize refuses a second device on the same card | Asking for the same card twice is refused when the BAR1 budget does not fit. <!-- src: docs/STATUS_DETAIL.md section 3 --> |

## A mapping is refused while the guest runs

| Message or symptom | Meaning and action |
|---|---|
| `no fd-backed guest RAM (the VM needs memory-backend-memfd,share=on)` | The VM's RAM is ordinary anonymous memory. The device does not refuse at start; this appears at the first guest memory mapping. Use `-object memory-backend-memfd,id=ram0,size=<N>M,share=on` with `-machine ...,memory-backend=ram0` and an equal `-m <N>`. <!-- src: crates/kf-qemu/src/mem.rs (place_ram), scripts/bench/boot_nvkvm.sh --> |
| `window map 0x...: placement budget (8192) exhausted` | Each CPU window admits at most 8192 live guest placements, counted across guest RAM and video-memory (store) views; the boot-display seed is counted separately. The refusal comes before any host call, so placements the guest already holds keep working, and a successful unmap gives capacity back. There is no property to raise the limit; it bounds the host file descriptors and memory mappings one guest can hold. By design. How close real Windows or Linux guests come to the limit has not been measured. <!-- src: crates/kf-mem/src/cpuwin.rs lines 54-58 and 585-590; docs/howto/requirements.md "Memory and apertures" --> |

## Linux guest

| Symptom | Cause and action |
|---|---|
| The guest driver does not load: `modprobe nvidia` fails, `MODPROBE_RC=1` | First check that the guest has a C compiler: with none, the installer's `apt` step never finished and the failure reads like a kernel mismatch. Then read where `/var/log/nvidia-installer.log` stops; a clean log that ends at "Kernel module compilation complete" is the signature of a killed install, not a failed one. <!-- src: scripts/bench/provision_bench_tree.sh --> |
| CUDA returns error 719 and the host log shows `Xid 31` (`FAULT_PDE`, `FAULT_PTE`, `FAULT_RO_VIOLATION`) | The application uses managed (UVM) memory that is first touched by the CPU or GPU, or kernels that access pageable host memory. Demand paging is not delivered to the guest. The four Linux UVM applications and Windows `managed_t` are unsupported for now; reports list `unsupported (UVM)`, pending a measured and bounded per-fault GR hold. They fail the checks, but not always visibly: `conjugateGradientUM` printed `result = SUCCESS` with a wrong result after a host `Xid 31`, so compare numerical output and read the host log; the exit status alone is not a safe test. With the device property `unmapped-reads-zero` on, which is the default, such accesses can read zero instead of failing: a managed-memory program may then finish with wrong numbers and no error 719, so compare numerical output. Set `unmapped-reads-zero=off` for a VM that uses managed memory and no Vulkan sparse resources, and see the property in the [Command-line reference](cli-reference.md). A host-side research patch exists but is not part of the stock path. <!-- src: README.md, docs/STATUS_DETAIL.md section 2; docs/design/V3_APP_MATRIX.md section 3 (conjugateGradientUM row) --> |
| A Vulkan application that uses sparse residency hangs, then reports a lost device; the host log shows `Xid 109` (`CTX SWITCH TIMEOUT`) and the `kayfabe` log `RC host twin ... except_type=0x6d (Xid 109)` | Sparse residency needs reads of non-resident pages to return zero, which the device property `unmapped-reads-zero` gives and which is on by default, so first check that the VM's `-device kf3` line does not set `unmapped-reads-zero=off` (with it off the host faults on those reads and this is the result). The Vulkan CTS sparse-residency cases hang this way with the property off and pass with it on (Linux guest, RTX 4090; the hang alone also on an RTX 3090). Windows guests have not been run. <!-- src: docs/design/V3_SPARSE_RESIDENCY_RCA.md --> |
| The `kayfabe` log shows `RC host twin ... except_type=0x1f (Xid 31)` | The same: the host driver reset a channel after a GPU memory-management fault. Check the host log. <!-- src: docs/STATUS_DETAIL.md section 2 --> |
| A thin-guest run reports `FAST_VERDICT=FAIL (rows verified, but these guest channels never reached hardware)` | Some guest channel ran on the CPU path instead of the GPU, which the harness treats as a failure even when the output is correct. Use `KF_REQUIRE_FORWARD=0` only for a deliberate control run. <!-- src: scripts/fastguest/run_fast_guest.sh --> |
| Runs interfere with each other; empty logs | GPU runs must be strictly serial. Use the lock (`/tmp/kayfabe-fastguest.lock`). <!-- src: scripts/fastguest/run_fast_guest.sh --> |
| Very slow guest, slow eager PyTorch decoding | The cost is in many small launches: every doorbell write is a trapped write (a VM exit), which is expensive on nested hosts. This is documented as the main performance cost; an optional guest doorbell module that avoids the exit is designed, not built. <!-- src: docs/STATUS_DETAIL.md section 4, README.md --> |
| A Vulkan app asks for a HIGH or REALTIME queue (`VK_KHR_global_priority`) and there is no priority boost | By design: the guest's request for a queue priority (the runlist scheduling policy, the interleave level, and the realtime promotion and runlist restart) is accepted and recorded as the VM's own state, and nothing is applied on the host. Every guest queue schedules at the host's default, as under a cloud vGPU; the device creates and runs normally. Honouring it would mean changing the host's global runlist policy, which is shared by every VM and by the host's own programs, and a guest never does that. The driver offers HIGH and REALTIME only to a process with `CAP_SYS_NICE` inside the guest (run as root or grant it), exactly as on the host; an unprivileged guest process sees MEDIUM only. This is by design; `vkCreateDevice` success for LOW, HIGH and REALTIME in the guest has not been measured on hardware after the realtime controls were added. <!-- src: docs/design/V3_QUEUE_PRIORITY.md, crates/kf-rm/src/queuepriority.rs --> |
| Stripes or green lines on the boot screen | The stripes were the guest driver's own GPU-written video memory drawn as pixels. A rule that shows the boot layer only while that memory is clean was added. Whether it removes every stripe is not documented yet. <!-- src: docs/design/V3_DISPLAY.md status 2026-10-18 --> |
| `GRAB IS PARTIAL` / `GRAB IS NOT OFFERED` in the broker log | The compositor lacks a protocol (some shortcuts still fire under grab) or the session cannot report focus loss (grab is not offered, for security). <!-- src: broker/README.md --> |
| Xorg loops restarting on `nvidia-drm` ("modeset(0): Failed to create pixmap") | Xorg chose the `modesetting` driver. Select `Driver "nvidia"` in `xorg.conf`. <!-- src: scripts/bench/display/interactive.sh --> |
| The graphical session shows nothing after the firmware screen | See [Display and the broker](display.md): `display=on`, the broker socket and uid, and the guest's `nvidia-drm.modeset=1`. |

## Windows guest

| Symptom | Cause and action |
|---|---|
| The NVIDIA display adapter shows **Code 43** | The driver ran its resource manager on the CPU instead of the GSP path kayfabe serves. Set `EnableGpuFirmware=1` in both the class key and the `nvlddmkm` service key, before the driver's first start, and restart; `kayfabe-windows-setup.bat` does it ([Run a Windows guest](windows-guest.md)). Seen on Windows 11 Pro and LTSC. <!-- src: docs/WINDOWS_GUEST_SETUP.md --> |
| Code 43 on GB206 even with the settings | A regression relative to a recorded passing Windows gate on that GPU; keep the exact source revision, driver versions and logs. GB202 was intermittent in the same sweep. <!-- src: traces/die_coverage_20261104/blackwell/README.md --> |
| The display driver resets when the Start menu opens | Set `HKLM\SOFTWARE\Microsoft\Windows\Dwm\OverlayTestMode = 5` (DWORD). The cause is not known. <!-- src: docs/WINDOWS_GUEST_SETUP.md --> |
| `dwm.exe` crash-loops after cloning an image | An image with a Windows update staged was snapshotted. Do not; block updates in test images. <!-- src: docs/WINDOWS_GUEST_SETUP.md --> |
| Boot to ssh takes about 20 minutes; the guest ignores `shutdown /s` for 5 minutes | The driver is in Code 43, which slows everything. Fix the cause (settings above). The test harness's `stop` falls back to QMP `quit`. <!-- src: scripts/bench/windows/README.md --> |
| The guest was enrolled in an unrelated company's Autopilot tenant | A default-identity VM matched it once. Give every VM a unique SMBIOS UUID, serial and MAC, and keep setup offline until after OOBE. <!-- src: docs/WINDOWS_GUEST_SETUP.md --> |
| The display driver resets (TDR) | See below. |
| `USER_RING_STAGED_SPAN_BUDGET` in the QEMU log | A Windows command buffer larger than 917,504 bytes also contains a software-class method call that kayfabe has to act on, so it cannot be inspected in the space available and its channel stops. The log line also shows `live=` (registered software calls) and `post_switch=` (entries still inspected after a ring migration). Save the log and report the application and workload. <!-- src: crates/kf-chan/src/user_turn.rs, crates/kf-chan/src/user_chunk.rs, docs/design/V3_USER_DESCRIPTOR_DECISION_20261022.md --> |
| `USER_RING_SCAN_BUDGET` in the QEMU log | Older builds only: a command buffer above 917,504 bytes that had to be inspected stopped its channel (the application then stalls and Windows resets the GPU). Current builds inspect such buffers in parts and no longer print this message; update. <!-- src: docs/design/V3_USER_DESCRIPTOR_DECISION_20261022.md --> |
| Edge resets after 7 to 16 minutes of playback | Known and unfinished. <!-- src: README.md "Not finished" --> |
| The guest does not wake from sleep | Windows sleep/wake is not supported yet. <!-- src: README.md --> |

### Reading a TDR on Windows

The Windows application matrix counts these in the guest's System log, for the time since an application started: <!-- src: docs/design/V3_WINDOWS_APP_MATRIX.md section 2.4 -->

- `nvlddmkm` event **153**, the record "Resetting / Restarting TDR", and `Display` event **4101**;
- Windows Error Reporting reports of live kernel events with codes 117, 141 or 116 (`VIDEO_TDR_TIMEOUT_DETECTED`, `VIDEO_ENGINE_TIMEOUT`) and bugchecks.

On the kayfabe side, the QEMU log gains a `GSP phase Running -> Suspending` line for each reset. The two counts are not always equal, so record both: in one recorded run the host side counted 7 resets while the guest log held 3 events and a 0x116 bugcheck. <!-- src: docs/design/V3_WINDOWS_APP_MATRIX.md section 2.4 -->

A cluster of resets can bugcheck the guest: with Windows' default `TdrLimitCount=5` within `TdrLimitTime=60` s, five resets in a minute became a 0x116 bugcheck. The application matrix therefore moves on to a fresh guest after 3. <!-- src: docs/design/V3_WINDOWS_APP_MATRIX.md section 2.5 -->

Event 153 alone does not establish a TDR: keep the provider, message and timestamp, and correlate them with `Display` 4101, Windows Error Reporting live-kernel reports or bugchecks, and the QEMU GSP transition log described above (read from the application matrix's rules). The investigation separates a harness restart triggered by its 153-inclusive budget from an independently recorded 141 or 4101 TDR. <!-- src: docs/STATUS_AND_HANDOFF.md update of 2026-10-12 21:01 UTC; docs/design/V3_WINDOWS_APP_MATRIX.md sections 2.4-2.5 -->

A TDR does not by itself mean a failure of the application, and the README's rule for a pass is the application's own completion, no host Xid and no GPU reset. <!-- src: README.md, docs/design/V3_WINDOWS_APP_MATRIX.md -->

## Not documented yet

- A guide to reading every Xid number that can appear. The repository records Xid 31 (MMU fault) and Xid 32.
- Recovery steps after a host GPU channel reset beyond reloading the driver (the provisioning script's `GPU_SETTLE` does an unload, a PCI reset and a reload, up to three times).
- What a guest-triggered GPU fault does to other VMs sharing the card: not measured. <!-- src: README.md -->

<!-- src (basis of this page): the entries are taken from the scripts, source and records named in each row (kayfabe revisions from `afb552ea` to `89fc8a222`). Not verified end to end: no entry was reproduced for this page. -->

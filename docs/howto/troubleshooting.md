# Troubleshooting

This page lists symptoms that the repository documents, with the cause and the action it records. It is not exhaustive. Where the repository does not explain a symptom, the page says so. <!-- src: scripts and documents named per entry -->

**First rule: if bare metal passes and the guest fails, the defect is kayfabe's.** Run the equivalent workload on the host with no VM first (`scripts/fastguest/bare_metal_suite.sh`, or the program itself); only a bare-metal failure indicts the machine. <!-- src: AGENTS.md "Verification" -->

## Collect evidence before you change anything

- **The host's kernel log.** A host GPU fault does not raise anything in the guest: the host driver services it, kills the offending channel and prints an `NVRM: Xid` line in the host's kernel log. Without watching that log, "the copy silently did nothing" and "the GPU faulted" look the same. `bash scripts/bench/host_xid_watch.sh <tag> -- <command>` runs a command and writes before/after/delta; it exits 6 when a new host Xid appeared across the command and 7 when it could not read the log. <!-- src: scripts/bench/host_xid_watch.sh -->
- **The guest driver's own `dmesg`** is not in the serial log unless the harness captures it. `scripts/bench/boot_capture.sh <tag>` stores it, and the host driver's output for that boot, in files. <!-- src: AGENTS.md "Bench traps", scripts/bench/boot_capture.sh -->
- **Is the job alive?** A killed job and a running job look the same if you only check for missing output. Write a start marker and an exit line and check that the process is alive. Note that `pgrep -x qemu-system-x86_64` never matches, because the process name is truncated to 15 characters. <!-- src: AGENTS.md "Bench traps", scripts/bench/boot_capture.sh -->
- **Which binary ran.** The device is linked into the QEMU binary, so a source change that was never relinked runs the old device. Print the binary's age, and run the per-revision binary (`kf3-bins/<rev>/`). <!-- src: scripts/fastguest/run_fast_guest.sh -->

## Building

| Symptom | Cause and action |
|---|---|
| `error[E0463]: can't find crate for std` on a crate unrelated to musl | The musl target is missing for the pinned toolchain: `rustup target add --toolchain 1.99.0 x86_64-unknown-linux-musl`. <!-- src: scripts/bench/provision_box.sh --> |
| `kf-gop-image: the Rust standard library for x86_64-unknown-uefi is not installed ...` | Add the target as the message says: `rustup target add x86_64-unknown-uefi --toolchain <pinned>`. <!-- src: crates/kf-gop-image/build/support.rs --> |
| `cargo not on PATH` from `build_kf3.sh` | `export PATH=$HOME/.cargo/bin:$PATH`. <!-- src: scripts/bench/build_kf3.sh --> |
| `<dir> is not a QEMU source tree` | Pass the extracted QEMU 10.2.4 source directory (it must contain a `VERSION` file). <!-- src: scripts/bench/build_kf3.sh --> |
| `kf3_gop.h: No such file or directory` (an old failure) | The overlay copy takes every `*.c`, `*.h` and `meson.build`; use the current `build_kf3.sh`. <!-- src: scripts/bench/build_kf3.sh --> |
| `no kf3 binary for this checkout's revision ...` from a launcher | Run `build_kf3.sh` from this checkout, or set `QEMU_BIN`. <!-- src: scripts/fastguest/run_fast_guest.sh --> |
| `<qemu> has no tpm-crb: rebuild with scripts/bench/build_kf3.sh` | The binary was built before 2026-10-04, when `--enable-tpm` and `--enable-slirp` were added to the configure line. Rebuild. <!-- src: scripts/bench/windows/win_vm.sh --> |

## The device refuses to start

The device checks its configuration when QEMU starts and refuses by name rather than clamping silently. If QEMU prints nothing and the serial log is empty, read the QEMU log first: the harness reports `FAST_VERDICT=CRASH (the serial log is EMPTY ...)`, usually because a concurrent run killed it or QEMU failed to start. <!-- src: scripts/fastguest/run_fast_guest.sh, crates/kf-qemu/src/cardbudget.rs -->

| Message or symptom | Meaning and action |
|---|---|
| `store of N MiB refused: NoMemory` | `fb-mb` is larger than the card can give. Lower it. On the 8 GiB RTX 3060 Ti, 6144 MiB was measured to boot; the bench's rule for cards under 10 GiB is (VRAM - 2048 MiB), rounded down to whole GiB. <!-- src: scripts/fastguest/run_fast_guest.sh --> |
| `a guest BAR1 of 256 MiB does not fit: 256 MiB + 16 MiB headroom > 256 MiB host aperture. The largest that fits here is 128 MiB` | Lower `bar1-size` (the harnesses use 128 MiB). The sum of guest BAR1, BAR2, the 1 MiB PRAMIN window and 16 MiB headroom, over all kayfabe devices on the card, must fit the host BAR1. <!-- src: scripts/fastguest/run_fast_guest.sh, crates/kf-qemu/src/cardbudget.rs --> |
| `the GPU still holds N MiB after 30 s - a previous VM did not release it` | The previous VM's GPU memory outlives its QEMU for a moment. Wait until `nvidia-smi` shows the GPU idle (`KF_GPU_IDLE_MIB`, default 512), or the store reservation will likely fail. <!-- src: scripts/fastguest/run_fast_guest.sh --> |
| `host driver X is not a measured tag of the driver matrix; kayfabe carries host structs only between measured layouts` | The host driver version is not one the repository measured. Use a version in [Requirements](requirements.md). <!-- src: crates/kf-abi/src/hostabi.rs --> |
| `driver X was never measured (looking up ...)` or `guest driver X: ...` | The same, for the guest version declared by `guest-driver=` (or defaulted to the host's). A release between two measured tags can move a field, so kayfabe refuses rather than borrow a neighbour's layout. <!-- src: crates/kf-abi/src/matrix.rs, crates/kf-qemu/src/device.rs --> |
| `guest driver version "..." does not parse (want major.minor[.patch])` | Fix the `guest-driver=` value. <!-- src: crates/kf-qemu/src/device.rs --> |
| `the guest driver says it is "..." but this device's layouts were selected for X - set the device property guest-driver=...` | The guest's driver version (read when its driver starts) differs from the declared one, and kayfabe refuses `SET_GUEST_SYSTEM_INFO` rather than answer with another release's layouts. Declare the version the message names. A Windows driver is keyed as its Linux tag; the harness's example is `580.65.06` for Windows driver 580.88. <!-- src: crates/kf-rm/src/guestsysinfo.rs, scripts/bench/windows/README.md --> |
| `channel-budget` refused | Below 256, or above what the host can give (or 2048). Leave it at `0`. <!-- src: qemu/hw/misc/kf3/kf3.c --> |
| `display-max-fps` refused | Outside 24..75, or without `display=on`. <!-- src: qemu/hw/misc/kf3/kf3.c --> |
| `/proc/driver/nvidia/version` lacks "Open Kernel Module" | The host has the closed module loaded, even if the installer exited 0. The installer's exit code is not evidence; check that string. <!-- src: scripts/bench/provision_host_driver.sh --> |
| "installation was canceled due to ... an alternate driver installation" | The distribution's `nvidia-*` packages are held; unhold and purge them first. This message points at the wrong cause. <!-- src: scripts/bench/provision_host_driver.sh header --> |
| Realize refuses a second device on the same card | Asking for the same card twice is refused when the BAR1 budget does not fit. <!-- src: docs/STATUS_DETAIL.md section 3 --> |

## Linux guest

| Symptom | Cause and action |
|---|---|
| The guest driver does not load: `modprobe nvidia` fails, `MODPROBE_RC=1` | First check that the guest has a C compiler: with none, the installer's `apt` step never finished and the failure reads like a kernel mismatch. Then read where `/var/log/nvidia-installer.log` stops; a clean log that ends at "Kernel module compilation complete" is the signature of a killed install, not a failed one. <!-- src: scripts/bench/provision_bench_tree.sh --> |
| CUDA returns error 719 and the host log shows `Xid 31` (`FAULT_PDE`, `FAULT_PTE`, `FAULT_RO_VIOLATION`) | The application uses managed (UVM) memory that is first touched by the CPU or GPU, or kernels that access pageable host memory. Demand paging is not delivered to the guest. This affects 4 of the 65 recorded CUDA applications, and it fails loudly. A host-side research patch exists but is not part of the stock path. <!-- src: README.md, docs/STATUS_DETAIL.md section 2 --> |
| The `kayfabe` log shows `RC host twin ... except_type=0x1f (Xid 31)` | The same: the host driver reset a channel after a GPU memory-management fault. Check the host log. <!-- src: docs/STATUS_DETAIL.md section 2 --> |
| A thin-guest run reports `FAST_VERDICT=FAIL (rows verified, but these guest channels never reached hardware)` | Some guest channel ran on the CPU path instead of the GPU, which the harness treats as a failure even when the output is correct. Use `KF_REQUIRE_FORWARD=0` only for a deliberate control run. <!-- src: scripts/fastguest/run_fast_guest.sh --> |
| Runs interfere with each other; empty logs | GPU runs must be strictly serial. Use the lock (`/tmp/kayfabe-fastguest.lock`). <!-- src: scripts/fastguest/run_fast_guest.sh --> |
| Very slow guest, slow eager PyTorch decoding | The cost is in many small launches: every doorbell write is a trapped write (a VM exit), which is expensive on nested hosts. This is documented as the main performance cost; an optional guest doorbell module that avoids the exit is designed, not built. <!-- src: docs/STATUS_DETAIL.md section 4, README.md --> |
| Stripes or green lines on the boot screen | [measured] the stripes were the guest driver's own GPU-written video memory drawn as pixels. A rule that shows the boot layer only while that memory is clean was added (2026-10-18). Whether it removes every stripe is not documented yet. <!-- src: docs/design/V3_DISPLAY.md status 2026-10-18 --> |
| `GRAB IS PARTIAL` / `GRAB IS NOT OFFERED` in the broker log | The compositor lacks a protocol (some shortcuts still fire under grab) or the session cannot report focus loss (grab is not offered, for security). <!-- src: broker/README.md --> |
| Xorg loops restarting on `nvidia-drm` ("modeset(0): Failed to create pixmap") | Xorg chose the `modesetting` driver. Select `Driver "nvidia"` in `xorg.conf`. <!-- src: scripts/bench/display/interactive.sh --> |
| The graphical session shows nothing after the firmware screen | See [Display and the broker](display.md): `display=on`, the broker socket and uid, and the guest's `nvidia-drm.modeset=1`. |

## Windows guest

| Symptom | Cause and action |
|---|---|
| The NVIDIA display adapter shows **Code 43** | The driver ran its resource manager on the CPU instead of the GSP path kayfabe serves. Set `EnableGpuFirmware=1` in both the class key and the `nvlddmkm` service key, before the driver's first start, and restart. [measured on Windows 11 Pro and LTSC] <!-- src: docs/WINDOWS_GUEST_SETUP.md --> |
| Code 43 on GB206 even with the settings | Open; GB206 under Windows is not solved. <!-- src: README.md --> |
| The display driver resets when the Start menu opens | Set `HKLM\SOFTWARE\Microsoft\Windows\Dwm\OverlayTestMode = 5` (DWORD). The cause is not proven. <!-- src: docs/WINDOWS_GUEST_SETUP.md --> |
| `dwm.exe` crash-loops after cloning an image | An image with a Windows update staged was snapshotted. Do not; block updates in test images. <!-- src: docs/WINDOWS_GUEST_SETUP.md --> |
| Boot to ssh takes about 20 minutes; the guest ignores `shutdown /s` for 5 minutes | The driver is in Code 43, which slows everything. Fix the cause (settings above). `win_vm.sh stop` falls back to QMP `quit`. <!-- src: scripts/bench/windows/README.md --> |
| The guest was enrolled in an unrelated company's Autopilot tenant | A default-identity VM matched it once. Give every VM a unique SMBIOS UUID, serial and MAC, and keep setup offline until after OOBE. <!-- src: docs/WINDOWS_GUEST_SETUP.md --> |
| The display driver resets (TDR) | See below. |
| Edge resets after 7 to 16 minutes of playback | Known and unfinished. <!-- src: README.md "Not finished" --> |
| The guest does not wake from sleep | Windows sleep/wake is not supported yet. <!-- src: README.md --> |

### Reading a TDR on Windows

The Windows application matrix counts these in the guest's System log, for the time since an application started: <!-- src: docs/design/V3_WINDOWS_APP_MATRIX.md section 2.4 -->

- `nvlddmkm` event **153**, the record "Resetting / Restarting TDR", and `Display` event **4101**;
- Windows Error Reporting reports of live kernel events with codes 117, 141 or 116 (`VIDEO_TDR_TIMEOUT_DETECTED`, `VIDEO_ENGINE_TIMEOUT`) and bugchecks.

On the kayfabe side, the QEMU log gains a `GSP phase Running -> Suspending` line for each reset. The two counts are not always equal, so record both: in one recorded run the host side counted 7 resets while the guest log held 3 events and a 0x116 bugcheck. <!-- src: docs/design/V3_WINDOWS_APP_MATRIX.md section 2.4 -->

A cluster of resets can bugcheck the guest: with Windows' default `TdrLimitCount=5` within `TdrLimitTime=60` s, five resets in a minute became a 0x116 bugcheck. The application matrix therefore moves on to a fresh guest after 3. <!-- src: docs/design/V3_WINDOWS_APP_MATRIX.md section 2.5 -->

A note from the Windows investigation records that event 153 alone is not proof of a TDR. Which other records settle it is not documented yet. <!-- src: docs/STATUS_AND_HANDOFF.md -->

A TDR does not by itself mean a failure of the application, and the README's rule for a pass is the application's own completion, no host Xid and no GPU reset. <!-- src: README.md, docs/design/V3_WINDOWS_APP_MATRIX.md -->

## Not documented yet

- A guide to reading every Xid number that can appear. The repository records Xid 31 (MMU fault) and Xid 32.
- Recovery steps after a host GPU channel reset beyond reloading the driver (the provisioning script's `GPU_SETTLE` does an unload, a PCI reset and a reload, up to three times).
- What a guest-triggered GPU fault does to other VMs sharing the card: not measured. <!-- src: README.md -->

Verified on: the entries are taken from the scripts, source and records named in each row (kayfabe revisions from `afb552ea` to `89fc8a222`). Not verified end to end: no entry was reproduced for this page.

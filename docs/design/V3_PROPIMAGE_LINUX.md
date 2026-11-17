STATUS: DESIGN-ONLY, 2026-11-16. Nothing in this document is built or has run on a box; every "NEW" item is a proposal.

# A Linux proprietary image (image C): design and first slice

Owner decision, 2026-11-16: image C is a **Linux** proprietary image (an Ubuntu desktop guest), the same model as the Windows images A and B: offline, a seed-derived machine identity, a recorded clock, private storage, and no login secrets on run boxes. The first application is MATLAB (Linux build) with its GPU path (Parallel Computing Toolbox, `gpuArray`), run headless with `matlab -batch` and, later, in a desktop session.

Labels: **CITED** (a vendor page read on 2026-11-16, URL given), **OBSERVED-IN-REPO** (read in the tree or on a named branch), **UNVERIFIED** (an inference or an unknown, with its falsifier), **ESTIMATE** (a number nobody measured). Application-specific entitlement handling is not part of this document; it lives with the owner, outside git.

Related, not repeated here: `docs/design/V3_PROPIMAGE_APP_AUTOMATION.md` (the Windows check pipeline, clock sidecar), the generic runner on the branch `claude/propimage-generic-20261116` (`docs/design/V3_PROPIMAGE_RUNNER.md`: image kinds, lanes, strict and standalone policy, the clock and network assert), `docs/design/V3_APPS_IMAGE_RECIPE.md` and `docs/design/V3_APPS_IMAGE_CHROOT_EVALUATION.md` (the open Linux apps image), `docs/OWNER_RULINGS.md` sections AR, AS and the proprietary-image runner ruling (AT, on the same generic branch).

## 1. What is decided and what is a proposal

| Item | Status |
|---|---|
| Linux guest, Ubuntu desktop, offline, seed identity, private R2, no login secrets on run boxes | Owner decision, 2026-11-16 |
| Clock: **one** mechanism for every proprietary image: the recorded clock from `state.json` (`last_time_at_stop_utc + gap_s`, never earlier than the seal), network off; a networked guest uses the real clock; the existing assert enforces both | Owner ruling (coordinator, 2026-11-16). No per-image or per-application clock branch exists or is proposed |
| NVIDIA guest driver and die are **not** pinned by the image | Owner constraint for ML rows; design in section 5 |
| Everything else below | Proposal |

## 2. What in `propimage.py` is generic and what is Windows

OBSERVED-IN-REPO: `scripts/bench/windows/propimage.py` (692 lines at `6bfa2dbc2`).

| Part | Verdict | Linux kind |
|---|---|---|
| `_h`, `_uuid`, `derive_ids` (SMBIOS uuid and serials, MAC, disk serial, GPU UUID, slirp subnet, ssh port, clock gap) | Generic | Reuse unchanged. **Trap:** `seed_matches` compares `derive_ids(seed)` with the state's `ids` for equality, so adding a key to `derive_ids` breaks every existing A and B `state.json`. New Linux fields (machine-id, hostname) come from a separate `derive_linux_ids(seed)` stored under its own key, and a golden test pins A and B ids byte for byte |
| `derive_password`, `init_state`, `validate_state`, `seed_matches` | Mostly generic | `validate_state` hard-codes the AMD profile and Hyper-V flags and demands a numeric `qemu.guest_driver`. The Linux kind needs an `os` field (`windows` default, so old states are unchanged), `hv_flags: []`, and `guest_driver` allowed to be null (section 5) |
| `utc`, `fmt`, `boot_clock`, clock sidecar | Generic | Same code path, no Linux branch |
| CPU profile and `cpu_preflight` | Generic mechanism, Windows-flavoured content | Reuse the named model so the guest-visible ISA is fixed (also fixes the vector paths numerical libraries pick at start-up: INFERRED, test in section 8). `hv_flags` empty for Linux; the AMD-host-only refusal stays (the Intel variant lives on `claude/propimage-intel-20261115`) |
| `build_argv` | About 80 percent generic: memfd RAM, SMBIOS, `-rtc`, disk with serial, slirp with `restrict`, virtio-rng, `kf3-gpu` device, the `cdroms` list | Per-OS switches: no swtpm and no `tpm-crb` (Linux needs no TPM; one less moving part, see the swtpm version note in `V3_PROPIMAGE_APP_AUTOMATION.md`), `smm` and Secure-Boot pflash only if section 4 chooses Secure Boot. Modes `setup`, `install`, `offline`, `ci` keep their meaning |
| `GUEST_PS`, `compare`, `run_guest_probe`, `ssh_argv` | Windows | NEW: a shell probe that prints the same JSON keys from `/etc/machine-id`, `/sys/class/dmi/id/*`, `/sys/class/net/*/address`, the virtio disk serial, `nvidia-smi --query-gpu=uuid` and `date -u`; `compare` reused with the TPM rows dropped |
| `render_unattend`, `render_setup_ssh`, `write_tools_dir`, `NVIDIA_EXE`, `PINS` | Windows | NEW: `render_cloud_init(state)` (user-data, meta-data, network-config for a NoCloud seed ISO attached through `cdroms`); no tools CD |
| `STATE_FILES`, `capture_files`, `restore_files` | Generic mechanism, Windows list | The list becomes per OS: `firmware/OVMF_VARS.fd` only |
| `pack`, `unpack`, `sha256_file` | Generic | Reuse unchanged for a one-disk image (section 6) |
| `fill_from_guest`, `cpu_fingerprint` | Mostly generic | Fed by the Linux probe (`/proc/cpuinfo` flags) |

Runner side (OBSERVED-IN-REPO on the generic branch): the image list, kinds `prebuilt`/`built`, lanes `gate`/`discovery`, strict and standalone policies, result rows, and the clock/network assert (`clocknet_assert.py`, which parses the QEMU argv and does not care about the guest OS) are OS-neutral. What must gain an OS dimension: the registry (`app_defs/*.py` definitions get an `os` key, default `windows`; a mismatch is a named refusal `APP_OS_MISMATCH`), the check harness (a sibling `propimage_linux_check.py` over ssh and bash, in the way `propimage_plain.py` is a sibling of `propimage_check.py`), and the result row (`guest_os`).

## 3. What a Linux kind needs

Identity (all derived from the seed, never captured):

| Item | Source | Where it is applied |
|---|---|---|
| SMBIOS uuid and serials, MAC, disk serial, GPU UUID, slirp /24, ssh port | `derive_ids` (existing) | QEMU argv in every mode, as for Windows |
| `/etc/machine-id` (32 hex) | NEW `derive_linux_ids`: `sha256(DOMAIN, seed, "machine-id")[:16].hex()` | Written at seal into `/etc/machine-id`; `/var/lib/dbus/machine-id` is a symlink to it. The probe compares it with the state |
| Hostname | NEW, derived the same way | Fixed in the image; the same on every clone of a seed |
| ssh host keys | Generated once at seal, stored in the image | Same on every clone of a seed (a clone is one machine). The *client* key pair is in `state.json` as for Windows (throwaway, only for the local transport) |
| Account | User `kfimage`, derived throwaway password, key-only ssh | Same field names as Windows |

OS configuration baked into the image (each item is checked by the Linux probe at boot, never trusted):

- **First boot work disabled.** cloud-init is used only in the `setup` mode; the sealed image carries `/etc/cloud/cloud-init.disabled`. The repo measured that a boot without the seed re-runs first-boot work and `snapd` seeding (`scripts/drivermatrix/stage_fat_guest.sh`), so the sealed image must be settled (`cloud-init status --wait`, `snap wait system seed.loaded`) before it is stopped. OBSERVED-IN-REPO.
- **Network offline by design.** One virtio NIC, a netplan file that matches by the state's MAC and uses DHCP (slirp `restrict=on` still answers DHCP and the loopback-forwarded ssh), no second interface, no bridges. A guest with no route must not wait for it: `systemd-networkd-wait-online` is masked, so a desktop session does not stall.
- **No time sync.** `systemd-timesyncd` and any NTP client are masked (the network is off; with a recorded clock a sync would be a second clock source). The RTC is UTC (`timedatectl set-local-rtc 0`).
- **Clock floor.** Linux has its own floors (systemd advances the clock to the mtime of its saved clock file and to its build epoch when the RTC is behind). This is the same shape as the floor measured on Windows (`V3_WINDOWS_PROPRIETARY_IMAGE.md` section 3), and the existing rule (boot at the recorded stop time plus a gap, never earlier) already satisfies it. INFERRED; falsifier: a clone whose probe reports a clock earlier than `base` or later than `base + 1800 s` fails the same `clock` row the Windows images use.
- **No unattended changes.** `unattended-upgrades`, `apt-daily*` and `snapd` refresh timers masked, kernel and driver packages held (the guest module is built for the running kernel and does not follow it: measured, `scripts/bench/provision_guest_llm.sh`).
- **Autologin for the desktop.** GDM `AutomaticLogin` for `kfimage` with Wayland off (`WaylandEnable=false`) so the session is X11 and screenshots and the kayfabe display path are deterministic; the sleep, screen blank and lock settings are off for that user and for the system. UNVERIFIED on kf3: the Linux desktop with the boot display has been recorded (`docs/howto/display.md`) but not with this image; falsifier: slice 3 below.
- **Remote access** is OpenSSH, key only, listening on the guest's 22, forwarded by slirp to `127.0.0.1:<ssh_port>` (the existing `-netdev user,...hostfwd`).
- **Evidence directory** `/var/kfapps/<run-id>/<app>`, owned by `kfimage`, pulled over ssh by an allowlist (the Windows rule, `C:\kfapps\...`).

## 4. Firmware

Proposal: UEFI (OVMF) without Secure Boot for Linux, with the variables file embedded in `state.json` as for Windows, so the boot-display option ROM (`gop=on`, an EFI driver) loads without the Windows-image key enrolment. UNVERIFIED: whether kayfabe's signed GOP is required by the Linux boot path; falsifier: boot a clone with `display=on` and read the display state. If Secure Boot is wanted, the Windows pflash arguments apply unchanged.

## 5. Driver and kernel: what is pinned, what is not

Owner constraint: for ML rows the image must not pin the die or the NVIDIA version. Consequence: **the sealed image contains no NVIDIA driver at all**, only the means to install one offline.

| Pinned in the image (reproducibility) | Not pinned (a run parameter) |
|---|---|
| Ubuntu release and cloud-image sha256; apt snapshot date (`V3_APPS_IMAGE_RECIPE.md` pins) | NVIDIA guest driver version (`guest-driver=` is the operator's declaration to the device; OBSERVED-IN-REPO `scripts/bench/guest_profile.sh`) |
| One kernel ABI, its headers and the compiler the kernel was built with (the driver module builds against exactly these; `stage_fat_guest.sh` derives the compiler from the kernel) | The die and the host driver |
| The application release and its product list (installer sha256, package list) | Host GPU count and VRAM (`fb-mb` is derived from the host card, `boot_nvkvm.sh`) |
| `qemu.machine`, `smp`, `ram_mb`, the CPU profile, the seed ids | |
| The open-source helper packages and their versions (the same lists as the open apps image) | |

How a run gets a driver: the run spec carries `guest_driver` (required when `state.qemu.guest_driver` is null; named refusal `GUEST_DRIVER_REQUIRED`). The runner makes a qcow2 clone, boots it in the existing offline `ci` shape plus **one extra read-only data disk** that holds the pinned NVIDIA `.run` (fetched by the box from NVIDIA's own origin and matched to its pin in `driver_installers`, never stored by us: V3_APPS_IMAGE_RECIPE rule), installs it with `-m=kernel-open` exactly as `stage_fat_guest.sh` does, checks `modinfo -F version nvidia` and the version of the `libcuda` it will load against the requested value, and only then starts the rows. A mismatch is a named failure (`GUEST_DRIVER_MISMATCH`), never a row failure. ESTIMATE: 2 to 4 minutes per run for the module build on a nested box (the Windows images pay a similar fixed cost for the GPU fix); to be timed on the first run.

This also means the **die must be able to run the application**. CITED (https://au.mathworks.com/help/parallel-computing/gpu-computing-requirements.html, R2026b): MATLAB supports compute capability 7.5 to 12.x; a newer architecture "might" work through forward compatibility. CITED (https://www.mathworks.com/help/parallel-computing/parallel.gpu.enablecudaforwardcompatibility.html): forward compatibility is off by default since R2020b, "can result in wrong answers and unexpected behavior", recompiles on first use (cache size advice `CUDA_CACHE_MAXSIZE`), and is not persistent across sessions (`MW_CUDA_FORWARD_COMPATIBILITY=1` persists). So a die below 7.5 gets the skip class `CONSTRAINT` with the reported capability (never FAIL), and a die above 12.x runs only the explicit forward-compatibility discovery row. UNVERIFIED (read only in a search summary of the older GPU-requirements pages; re-read before relying): the CUDA Toolkit is not needed for GPU functions, only for kernel objects and GPU Coder. CITED: `mexcuda` falls back to the `nvcc` MATLAB ships (https://www.mathworks.com/help/parallel-computing/mexcuda.html), so the image needs no CUDA toolkit package (a custom toolkit path is only via `MW_NVCC_PATH`). Linux support list, CITED (https://www.mathworks.com/support/requirements/matlab-linux.html, R2026b): Ubuntu 24.04 and 26.04 LTS are validated; 4.5 GB for MATLAB alone, 26 GB for all products; 8 GB RAM minimum, 16 GB recommended; the page warns that a future release will require AVX2 (the profile has it).

## 6. How the image is built, sealed, stored and consumed

Layers and the overlay question (evaluated honestly):

| Option | Private part | Pros | Cons |
|---|---|---|---|
| **A. One full image** (Ubuntu + application + identity), one qcow2, packed by the existing `pack` into the three-member zip | Everything (about 12 to 35 GB: ESTIMATE from the CITED install sizes plus an Ubuntu desktop of about 6 to 8 GB) | No new packing, no runner change for the disk, one hash, same `unpack`, same R2 flow as A and B | The open part is private too and is re-uploaded on every re-seal |
| **B. qcow2 overlay on a public base** | The application tree | The public base can be cached on boxes | The application tree dominates the size, so the saving is the Ubuntu share only (about 20 to 40 percent: ESTIMATE). A backing chain is also fixed by file, so a private overlay cannot sit above a per-driver overlay: the driver axis (section 5) would need one private overlay per driver version, or a base per version |
| **C. Public base + private data disk** (a second read-only ext4 volume holding the application tree, mounted at a fixed path; the driver is installed on the clone as in section 5) | The application tree and its settings | Independent of the driver axis, the base is shared by every image of the Linux kind, only the data disk is private | NEW: a fourth zip member, a manifest `layers` list, an extra `-drive` with a seed-derived serial, a base-compatibility pin (the data disk records the base sha256 and library set it was installed against), more code and tests |

Recommendation: **A for the first image**; revisit C when a second Linux application (Maya, Redshift: `/root/night/IMAGE_C_CANDIDATES_20261116.md` ranks them next) makes the shared open base worth its extra moving parts. Reasons: the byte saving of B is small, B conflicts with the driver rule, and C costs code that nothing needs until a second image exists. `built` for Linux (an install by script from official media) stays phase 2 of the runner, as for Windows.

Build (on the pc, by the owner, for the install step; GPU not required):

1. `propimage_linux.py init SEED --out state.json` (NEW) derives the identity; the MAC and all serials are fixed here, **before any application is installed**, because first-run application state may bind to them (UNVERIFIED which parts; falsifier in section 8).
2. `propimage_linux.py build` (NEW) creates the disk from the pinned Ubuntu cloud image, renders the NoCloud seed from the state, boots in `setup` mode (internet on, real clock: the assert allows exactly that) on the distro QEMU, waits for cloud-init, applies the section 3 configuration, installs the pinned kernel, headers and compiler, desktop and the application dependency packages from the pinned apt snapshot.
3. The owner installs the application from the vendor's installer with the pinned release and product list, as the application's own documentation describes (the install needs the internet; the sealed image does not), and performs the application's own first-run and entitlement step by hand. Nothing from this step enters git, the Worker or a box.
4. `propimage_linux.py seal`: stop clean; record `last_time_at_stop_utc` and the CPU fingerprint through `verify --record`; clear logs, shell histories, caches and package lists; confirm no network credential, no browser profile and no cached login exists in the image (a grep list is part of `seal`); `fstrim`, `qemu-img convert -c`; `pack` (embeds `firmware/OVMF_VARS.fd`).
5. Upload the zip to the private R2 bucket (coordinator), record the sha256 in the pin file as for image A and B. Presigned download URLs reach boxes through the Worker, as for images A and B.

Consume on a box: download by presigned URL, `unpack` (member and sha256 checks), `qemu-img create -b image.base.qcow2` clone (the immutable backing alias of the Windows pipeline), swtpm not needed, the data disk with the pinned driver `.run`, boot `ci` shape, first check (`verify`: ids, clock, GPU UUID equal to the seed UUID, driver version equal to the request), rows. Every row boots in its own fresh clone in the discovery lane (the runner's rule); gate rows may share a boot.

## 7. First slice: six MATLAB rows

All six enter the **discovery lane** first. A row is promoted to the gate lane after three consecutive clean runs on at least two dies (proposal). Every row boots `matlab -batch "<statement>"` (CITED, https://www.mathworks.com/help/matlab/ref/matlablinux.html: no desktop, exits 0 on success and non-zero on error, output on stdout and stderr, a dialog needing input is an error; `-logfile` copies the command window) from a script placed in `/var/kfapps/<run-id>/matlab/`. Inputs are generated on the **host** by numpy from a fixed seed and delivered in the row's input directory (MATLAB's random generators are never the reference). Outputs are written as raw binary plus a JSON of scalars; the **host** recomputes the references and judges. Exit status zero alone is never a pass (owner rule, `OWNER_RULINGS.md` AP item 5).

GPU use is asserted from the host: under kf3 the guest's `nvidia-smi` reports `[N/A]` for utilisation (MEASURED on a 4090 box, 2026-11-16, per the docstring of `scripts/bench/windows/gpu_assert.py` on `claude/imageb-apps-20261116`), so the shared assertion samples the host's `nvidia-smi` (`gpu` utilisation, `mem` rise) and keeps the guest samples as evidence. NEW: `gpu_assert` gets a Linux transport (the sampler started over ssh and bash instead of PowerShell); the host half is unchanged.

| Row | What it does | Host verification | GPU-use rule |
|---|---|---|---|
| `gpudiag` | `gpuDevice`, `validateGPU` (CITED: since R2024b), device name, compute capability, driver and toolkit versions, a 1e6-element `gpuArray` sum | Name, memory and capability against the host's facts; guest `nvidia-smi` UUID equals the seed UUID; capability inside 7.5 to 12.x else `CONSTRAINT` skip | `mem` |
| `matmul_fft` | Host-supplied single and double matrices, `gpuArray` `A*B`, `fft2`, `ifft2` round trip, `gather` | numpy `A@B` and `fft2` within a stated tolerance; round trip equals the input | `gpu` and `mem` |
| `solver` | `A\b` (LU), `chol`, `svd` on gpuArrays; a size sweep that forces many small kernels | Residual `norm(Ax-b)` recomputed on the host; singular values against numpy | `gpu` and `mem` |
| `dl_train` | A small CNN trained for a fixed number of iterations on a host-generated synthetic dataset with `ExecutionEnvironment=gpu` | Loss strictly decreases over the run and the trained weights reproduce the final accuracy when evaluated by the host-side script on the saved network; no equality of weights is claimed across dies | `gpu` and `mem` |
| `oversub` | Allocate gpuArrays until `gpuDevice().AvailableMemory` is exhausted, catch the out-of-memory error, `reset(gpuDevice)`, allocate again | The error is raised (not a hang or crash), allocation works again afterwards, no host Xid in the window (`host_xid_watch`), VRAM returns to baseline | `mem` |
| `cudakernel` | `mexcuda -ptx` of a host-supplied `.cu`, `parallel.gpu.CUDAKernel` from the PTX, launch with a grid, a shared-memory variant; second variant with `parallel.gpu.enableCUDAForwardCompatibility(true)` only on a die above 12.x (CITED wrong-answer warning, so the output check is the verdict) | Output bytes against the host's numpy reference | `gpu` |

Not in the first six (named so they are not forgotten): `parpool` with one `gpuDevice` per worker (several CUDA contexts on one die), `multi-gpu` and `parfeval` on GPU, MATLAB Coder / GPU Coder (UNVERIFIED whether it needs a CUDA toolkit), and a desktop-session row (slice 3). Startup cost of `matlab -batch` per row is an ESTIMATE of 20 to 60 s on bare metal and more on a nested box; the per-row hard time-box is set from the first measured run, not guessed.

## 8. Falsifiers to run before the heavy work

1. **Clone identity.** Two clones of one sealed image, one seed, on two hosts: the Linux `verify` is all MATCH on both, and the application starts on both. Falsifies "the seed identity is enough".
2. **Clock.** A clone booted at `last_time_at_stop_utc + gap_s` has `date -u` inside the `base .. base + 1800 s` window on both hosts.
3. **Driver stage.** The same sealed image with two different `guest_driver` values passes `gpudiag` with the requested version each time and the GPU UUID of the seed.
4. **Numerical reproducibility.** The same row on a clone with the fixed CPU profile and on `-cpu host` produce the same bytes or the difference is within the row's tolerance (the profile hides AVX-512 paths; INFERRED to matter for CPU-side reference sums only).
5. **Seal hygiene.** The seal grep list finds no cached credential and no browser profile; an independent reader runs it on the packed zip.

## 9. Risks

1. **Application machine-binding across clones.** An application may bind first-run state to the NIC, disk or other ids. The design fixes the ids before install and reuses them for every clone; whether that is sufficient is falsifier 1. Entitlement details are handled outside this repository.
2. **kf3 under a large CUDA stack.** The point of the image is an unusual caller of the driver (cuBLAS, cuFFT, cuSolver, cuDNN, JIT PTX, many small kernels). The first rows may crash or hang the guest or raise host Xids: the discovery lane exists for that, and a wedged guest must not stop later rows (each row has its own clone and time-box).
3. **Run-time driver install.** A kernel-module build in a nested guest adds minutes per run and can fail for reasons outside kayfabe (compiler, headers, a driver that does not build on the pinned kernel: measured for 545 on 6.8 in `stage_guest_kernel.sh`). The pinned kernel therefore limits which driver versions the image can take, so the kernel ABI is part of the image's declared support, and `GUEST_DRIVER_MISMATCH` is its own class.
4. **Size and transfer.** A full image is tens of GB per box (ESTIMATE 12 to 35 GB); download time per cell is the cost the generic runner already lists as "later".
5. **Desktop session under kf3.** Autologin, the display broker and GDM interplay are unmeasured; slice 3 is gated on slice 2.
6. **Documentation boundary.** Anything specific to the application's entitlement stays out of git and out of this directory; the public rows only name the application and its public documentation.

## 10. Effort and order

ESTIMATE (agent working days, GPU-free unless said):

| Step | Content | Effort |
|---|---|---|
| PR 1 (smallest) | `propimage_linux.py`: `derive_linux_ids`, `render_cloud_init`, Linux `build_argv` flags, the Linux probe and `compare`, state `os` field and the A/B golden test; tests and two-killer mutants | 2 to 3 days |
| PR 2 | Runner OS dimension: registry `os`, `propimage_linux_check.py`, `gpu_assert` Linux transport, `guest_driver` run parameter and the driver-stage step, named refusals; tests | 3 days |
| Owner on the pc | Open proof image: the same kind with no proprietary application (open CUDA samples), to prove the kind before MATLAB | 0.5 day of hands, 1 day of agent support |
| PR 3 | MATLAB `app_defs/matlab.py`, the six row scripts, host verifiers and their tests | 3 to 4 days |
| Owner on the pc | MATLAB install and first-run step, seal, upload | 2 to 4 hours of hands plus the download |
| First box run | Discovery lane on one die, then two | 1 day, about the cost of one box leg |

The build commands above are proposals until PR 1 lands.

## 11. Not done

Nothing here ran: no build, no boot, no application install. The vendor facts are CITED from the pages named. The six rows, the `os` registry key, the driver-stage step and the Linux probe are designs.

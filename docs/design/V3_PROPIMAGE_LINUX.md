STATUS: DESIGN-ONLY, 2026-11-16, except section 12 (slice 1: the local build tooling and its measured proof on one coordinator box, LIVE as tooling) and section 13 (2026-11-17: package manifest, offline enforcement and run-time injection, LIVE as tooling, unit-tested only, never run on a guest). Sections 1 to 11 are unchanged design and proposals; every "NEW" item outside section 12 is a proposal.

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

Sections 1 to 10 ran nothing: no build, no boot, no application install (section 12 is the exception: the tooling, the Ubuntu base and the clone proof, without any application). The vendor facts are CITED from the pages named. The six rows, the `os` registry key, the driver-stage step and the Linux probe are designs.

## 12. Slice 1: the local build tooling and its proof (no application, no GPU, no Vast)

STATUS: LIVE as tooling, 2026-11-16. Labels: **MEASURED** = ran on the coordinator box (Scaleway VM, 6 vCPU AMD, nested KVM) on 2026-11-16, run directory `/workspace/imagec_20261116/proof` (outside git; its text evidence is `traces/propimage_linux_slice1_20261116/`), revision of the tooling = this branch before its final commit; **TESTED** = GPU-free unit tests and mutants, not run against a guest; **UNVERIFIED** = a vendor behaviour or an experiment nobody has run.

The module is `scripts/bench/linux/propimage_linux.py` (new; it imports `derive_ids`, `derive_password` and the CPU profile from `scripts/bench/windows/propimage.py` unchanged, and the clock rule from `scripts/bench/windows/clockrule.py`, a byte-identical copy of the file of PR #454 so this branch is testable alone; an identical add on both branches merges cleanly). Tests: `scripts/ci/test_propimage_linux.py` (115 tests, GPU-free, run on 2026-11-16 on the coordinator box: OK); mutants (two named killers each): `scripts/ci/propimage_linux_mutants.py` (32 of 32 killed by at least two tests, same run); both run in CI. The gates `doc_freshness`, the claim ledger, `ignore_ladder`, `workflow_lint`, `no_private_refs` and `no_gpu_uuids` were run on the branch and are clean.

### 12.1 Commands

| Command | What it does |
|---|---|
| `init --seed S --dir D [--desktop] [--nic virtio-net-pci\|e1000e] [--ram-mb --smp --disk-gb]` | `D/state.json` (mode 0600): `derive_ids(S)` unchanged (SMBIOS, MAC, disk serial, GPU UUID for the later kf3 clone, slirp subnet, ssh port, clock gap), plus `linux_ids` (machine-id, hostname) in their own key, the guest account, an ssh key pair, the qemu parameters (ram, smp, disk size, NIC model, `guest_driver: null`), the recorded-clock field. Refuses to overwrite an identity. |
| `fetch-base --dir D` | The pinned Ubuntu 24.04 cloud image (`release-20261017`, sha256 `04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94`, taken from that release directory's `SHA256SUMS`), checked after download. |
| `setup --dir D` | Boots the setup VM: pinned identity, **real clock, network on** (slirp), `-vnc 127.0.0.1:<n>` only (no code path builds another bind address), ssh forward on `127.0.0.1:<ssh_port>`. First call: unattended provisioning (cloud-init seed for the account and key, then two scripts over ssh with a reboot after each). Prints the tunnel recipe. |
| `novnc --dir D` | websockify from a venv (`pip --require-hashes`, wheel pinned by sha256) and the pinned noVNC release (sha256), bound to `127.0.0.1`. |
| `auth-capture`, `auth-paste` | Sign-in helpers, section 12.4. |
| `inventory --dir D` | Read-only guest-side listing of files created or changed after provisioning (path, size, mtime, mode, uid, no contents), under the places an application keeps sign-in state plus a bounded "changed since" sweep; saved 0600 under `D/private/`. |
| `seal --dir D [--grep-regex R] [--grep-token R] [--grep-literal L] [--grep-file F]` | Credential grep inside the guest (counts per path), then cleanup (apt lists and caches, logs, journal, histories, cloud state, every `/var/tmp/kf-auth-*` file, which it then checks is gone), `fstrim`, records the guest UTC into `state.json` (`last_time_at_stop_utc`), clean poweroff, raw scan of the whole disk (counts per pattern, including free space), `qemu-img convert -c`, package manifest (`packages.tsv`, kernel and count into the state), embeds `firmware/OVMF_VARS.fd`. Patterns are printed as `kind#-sha8` labels, never as text; matches are never printed. |
| `scan-image IMAGE\|ZIP --grep-*` | An independent reader for the same grep over a sealed image. |
| `run --dir D [--name N] [--net online] [--offset-days N]` | Offline clone: a throwaway overlay of the sealed image, `restrict=on`, the recorded clock (`last_time_at_stop_utc + gap_s`) through the clock rule, the same identity; ssh in, probe, compare, stop. |
| `offline-check --dir D [--reboots N] [--advance-days N] [--matlab PATH] [--cmd C]` | A command set in an offline clone; per command only the exit code, the number of kept and dropped lines and a sha256 of the kept lines; every line that looks like an account or licence line is dropped before hashing. |
| `install-driver --dir D --run F --sha256 H [--created-utc T]` | Section 12.5. |
| `pack`, `unpack` | `image.qcow2`, `state.json`, `manifest.json` (+ `--extra` members), STORED, sha256 per member; `unpack` checks every member and the seed binding. |
| `argv STATE setup\|run ...` | The exact qemu argv. |

The desktop is optional: `init --desktop` adds `ubuntu-desktop-minimal`, GDM autologin on X11 and the no-blanking settings; without it the guest is a terminal machine (ssh, tmux, build tools, headers), which is enough for `matlab -batch` rows. Both images carry: one pinned NIC matched by MAC in a netplan file, no time sync, no unattended upgrades, held kernel, `cloud-init.disabled`, build tools plus the headers of the running kernel and the compiler it was built with (the NVIDIA module builds with exactly these on the offline clone), and the auth-URL capture shim (12.4).

### 12.2 The proof run (`/workspace/imagec_20261116/proof`, 2026-11-16)

- MEASURED: `init` with seed `kayfabe-linux-proof-20261116`, 4096 MB, 2 vCPU, 30 GB virtual disk, desktop image; `setup`: base verified, first boot with cloud-init done 28 s after start, unattended provisioning 565 s in total (stage A, kernel, headers, build tools and `ubuntu-desktop-minimal`: 462 s; stage B, configuration and identity: 3 s; two reboots of about 8 s to ssh), kernel `6.8.0-149-generic`, 1645 packages. The probe after provisioning matched every identity row (machine-id, hostname, SMBIOS uuid and three serials, MAC, disk serial), headers present, compiler `x86_64-linux-gnu-gcc-13` present, cloud-init disabled. A QMP screendump showed the GNOME desktop logged in as `kfimage` (autologin, X11). Setup used about 7.1 GB of the 30 GB virtual disk (allocated).
- MEASURED: `seal` with a test pattern set (an e-mail-like regex, a token-like regex, a literal): 576 s in total (guest grep over 157 116 files about 5 min; guest stop 9 s; raw scan; compressed convert), image `2 462 646 272` bytes (2.3 GiB), sha256 `f4474d309c5e0ff81bfb97fdcdd7f94d9a6cb13f2c398a40d234ef7a103b6c52`. The token-like regex (`eyJ` followed by 20 base64 characters) matched 15 files of the stock install (certificate bundles, PPD databases, one JWT library's docs): the report lists labels and counts per path and no match text, which is the report's purpose (a false positive is read by the owner; a real token would show in a path under the application's directories).
- MEASURED: two separate offline clones (`restrict=on`, recorded clock `2026-11-16T20:10:10Z` = stop time `19:25:09Z` + gap 2701 s): all rows MATCH on both (identity values equal to `state.json`, hence to each other), guest `date` 32 s and 40 s after the recorded base at the probe (window: base to base + elapsed + 10 s), neither ping nor https reachable, headers and compiler present. Clone boot to ssh 33 s; the shutdown of a clone took about 90 s (a user-session stop job waited for its timeout; a 20 s `DefaultTimeoutStopSec` was added to the provisioning script afterwards, NOT measured).
- MEASURED: `offline-check` with `echo` commands (not MATLAB): one reboot of the clone, both rounds exit 0 and no network, guest UTC 20:10:44 then 20:11:42; with `--advance-days 35` the clone started at `2026-12-21T20:10:43Z` and the check passed. This proves the mechanics of experiments E1 and E3 (12.3), not any application behaviour.
- Disk: MEASURED 8.7 GB resident in the run directory after all runs (setup disk 5.1 GB allocated, sealed image 2.3 GB, base 0.6 GB, noVNC and the venv, clone overlays deleted); the transient peak (the raw scan copy, the packed zip and its unpacked copy) was not measured: ESTIMATE up to about 17 GB. The coordinator's limit was 35 GB.
- MEASURED: `pack` and `unpack` of the sealed image took 12.5 s each (zip 2 463 371 583 bytes); `novnc` served `vnc.html` (HTTP 200) on `127.0.0.1:6080` only and upgraded a websocket to a clone's VNC (HTTP 101). `websockify`'s numpy dependency is optional, so the venv installs it with `--no-deps`.

### 12.3 The owner's manual steps (terminal first)

UNVERIFIED throughout: every statement about how the application signs in, what it caches, for how long it works offline and whether it contacts its vendor periodically. The experiments below settle it; their results belong in the owner's private note, never in this repository.

1. On the box: `python3 scripts/bench/linux/propimage_linux.py init --seed <SEED> --dir D --ram-mb 8192 --smp 4 --disk-gb 60` (add `--desktop` only for GUI rows), `fetch-base --dir D`, `setup --dir D`. The NIC MAC and every serial are fixed here, before anything is installed.
2. Reach the guest from your PC through the box (the guest's ssh is only on the box's loopback): `scp -J <user>@<box> -i D/ssh/id_ed25519 -P <ssh_port> <installer> kfimage@127.0.0.1:`, then `ssh -J <user>@<box> -i D/ssh/id_ed25519 -p <ssh_port> kfimage@127.0.0.1` and work inside `tmux`. (A read-only data CD works too: `build_argv(... extra_iso=...)`.)
3. Install the application as its own documentation describes. The setup VM has the network on and the real clock.
4. Sign in once, from a terminal session: `propimage_linux.py auth-capture --dir D --cmd '<path>/matlab -nodesktop -nosplash' --coordinator <user>@<box>`. It prints the mode it saw: **loopback** (the application listens on a loopback port: it prints the authorization URL and the `ssh -J ... -L` command for your PC; open the URL in your own browser; SSO happens there and the redirect reaches the guest through your tunnel) or **paste** (it prints the URL and the prompt; log in on your own PC, copy the code, run `propimage_linux.py auth-paste --dir D`, which reads the code with a hidden prompt and sends it over ssh stdin into the guest's tmux pane). Nothing you type into the guest console, and the code is never in an argument, a file, a history, a log or this tool's output. Then `matlab -batch "disp(1)"` as the quick check. With a desktop image the same helper works against the desktop session (`--desktop-env`) and the console is reachable by noVNC (12.6).
5. `propimage_linux.py inventory --dir D`: read where the sign-in state lives (paths and sizes only).
6. `propimage_linux.py seal --dir D --grep-regex '<e-mail pattern>' --grep-token '<token-like pattern>' ...`, **read the report yourself**, run `scan-image D/sealed/image.qcow2 --grep-...` as an independent reader, then `pack`.
7. Offline clone proof: `run --dir D`, then the experiments:

| # | Experiment | Exact command | PASS means |
|---|---|---|---|
| E1 | Reboot an offline clone with the recorded clock | `propimage_linux.py offline-check --dir D --name e1 --reboots 1 --matlab <path>/matlab` | exit 0 for `matlab -batch "disp(1)"` and `... "ver"` in both rounds, no network |
| E2 | After E1 passes: revoke the device or session on the vendor's account page (the owner decides), then boot the sealed image offline again | `propimage_linux.py offline-check --dir D --name e2 --matlab <path>/matlab` | the same, after the revocation |
| E3 | Throwaway clone only, never the sealed image: the clock +35 days, then +95 days | `... offline-check --dir D --name e3a --advance-days 35 --matlab ...` and `... --name e3b --advance-days 95 ...` | the same at each offset |

A FAIL at E2 or E3 means the offline image depends on the vendor; the sign-in must then be re-done per run, which this design does not allow on an untrusted box. Do not decide here.

### 12.4 Sign-in capture (OAuth loopback, copy-the-code) and its fallback ladder

UNVERIFIED whether the application signs in with a system browser and a loopback redirect, by copying a code, with an embedded web view, or with a custom URI scheme. A terminal application cannot use a web view, so the two realistic flows are supported; the helper prints what it saw into the owner's private log (`D/private/auth-capture.log`, mode 0600: mode, whether a loopback port appeared, the prompt with every code-like token and e-mail address masked), so the first real attempt answers the question.

- **The shim.** `/usr/local/bin/kf-capture-url`, also installed as `xdg-open`, `x-www-browser` and `sensible-browser` first in `PATH`, with `BROWSER` set for every login and for systemd user sessions. It opens nothing: it appends `time<TAB>url` to `/var/tmp/kf-auth-urls.log` (mode 0600) and prints the URL on the terminal. `seal` deletes it, the credential grep includes it, and `seal` refuses if any `/var/tmp/kf-auth-*` file survives.
- **The watcher.** `auth-capture --cmd C` starts C in a tmux pane (a pty) with the pane output piped to `/var/tmp/kf-auth-pty.log` (0600), then polls over ssh: URLs from the shim log and from the pty stream (ANSI stripped), and the listening TCP ports of the pane's process tree (`ss -ltnp`, new since the start). A listener found: **loopback**; none within the grace period: **paste**. For loopback it names which host the `redirect_uri` used (`localhost`, `127.0.0.1` or `::1`), forwards the **detected** port (not necessarily the one in the URL; a difference is printed), binds the owner's side to the same address family and targets the family the application listens on: `ssh -N -J <user>@<box> -i <key> -L [bind:]P:target:P -p <guest ssh port> kfimage@127.0.0.1`. The URL holds a one-time state and a PKCE challenge and expires: open it within minutes.
- **Never a code.** Any query or fragment value named `code`, `*token*`, `*secret*`, `password`, `assertion` or `*_code` (except `user_code`) is replaced by `REDACTED` before anything is printed. `auth-paste` sends the code over the ssh stdin into a tmux buffer, switches pane logging off for the paste and on again, and deletes the buffer.
- **TESTED** (unit tests: pure parsing, the plan for all three hosts and both families, redaction, masking, the paste script's order, the shim run locally, the stub app end to end locally, a scripted guest) and **MEASURED** (`auth-selftest` on an offline clone of the proof image with the stub app `scripts/bench/linux/auth_stub_app.py`, no vendor traffic, no credential): loopback with `localhost`, `127.0.0.1` and `[::1]` redirects each printed the URL and a forward command, a request through an `ssh -L` from the box (the stand-in for the owner's PC) reached the stub and its `state` matched; the terminal stub's paste flow delivered a test code of the right length and hash, and the code was not in the pane log afterwards.
- **Fallback ladder, each rung UNVERIFIED:** (1) loopback redirect, (2) copy-the-code in the terminal, (3) a custom URI scheme or an embedded web view: the helper reports "not a loopback redirect"; the next rung applies, (4) a device-code flow if the application offers one (the URL and a short user code are printed; open the URL on your PC), (5) typing the credentials into the guest console through noVNC. Rung 5 is the owner's explicit last resort (the owner prefers never to type them into the guest) and needs a `--desktop` image.

### 12.5 The NVIDIA driver at run time

The image holds no NVIDIA driver. `install_driver` (and `install-driver --dir D --run F --sha256 H`) stages the `.run` on a running offline clone and builds it there: `sudo env CC=<the kernel's compiler> sh nv-<version>.run --silent --no-x-check --no-nouveau-check --no-questions -m=kernel-open -j<n>` (the flag set of `scripts/drivermatrix/stage_fat_guest.sh` and `provision_bench_tree.sh`, where it was measured; `--no-nvidia-modprobe` is deliberately NOT used, because an unprivileged CUDA process needs `nvidia-modprobe` for the device nodes; `--skip-module-load` only where the clone has no GPU). The installer's sha256 is checked before anything is copied (`DRIVER_SHA_MISMATCH`), a missing kernel compiler is `KERNEL_CC_MISSING` (the clone cannot `apt`), and a `modinfo -F version nvidia` or `libcuda` that is not the requested version is `GUEST_DRIVER_MISMATCH`. The kf3 device takes the driver version from the run (`GUEST_DRIVER_REQUIRED` when absent). The clock floor applies: the clone boots at the recorded clock, never earlier than the installer's `created_utc` (`KF_CLOCK_BEFORE_INPUT`, refused before any boot; with no stated time the file's mtime is the fallback and a WARN). TESTED with a stub (no GPU here): the flag set, the compiler, success, a wrong sha, a missing compiler, a module or libcuda mismatch, the clock refusal and its passing case. NOT run: no kf3 device, no real installer.

### 12.6 noVNC (desktop images only)

`propimage_linux.py novnc --dir D` creates a venv under `D`, installs `websockify==0.13.0` with `pip --require-hashes` (the wheel's sha256 was read from PyPI on 2026-11-16), unpacks noVNC v1.6.0 after checking its sha256, and runs websockify on `127.0.0.1:6080` toward the VM's VNC at `127.0.0.1:5900+<display>`. Nothing is installed system wide (`websockify` and `novnc` are not installed on this box). From your PC: `ssh -N -L 6080:127.0.0.1:6080 <user>@<box>`, then `http://127.0.0.1:6080/vnc.html?autoconnect=1`; for a plain VNC client, `ssh -N -L 5907:127.0.0.1:5907 <user>@<box>` and `vnc://127.0.0.1:5907`. The VNC server has no password; the tunnel is the access control, so nothing binds a public address.

### 12.7 Credential rules

A cached sign-in is a credential: a token, not only an e-mail address. A run box (Vast) is untrusted and could read it, so the sealed image never leaves private R2 and the credential-grep report must be read by the owner before any upload; nothing here claims the cache is harmless. The report cannot prove absence (a token in an unexpected form is not found): it lists where the sign-in state lives (`inventory`) and how often the patterns hit, and the owner decides.

### 12.8 Open questions (not decided here)

1. **NIC model.** `qemu.nic_model` is a state field, default `virtio-net-pci`, `e1000e` allowed (both are built into the argv and tested). A licensed application may accept only a physical-looking adapter: whether virtio is enough is unknown. The MAC is derived and fixed either way. Decide after the first real sign-in survives (or not) an offline clone.
2. The sign-in flow and the offline behaviour of the application (12.3, 12.4).
3. Whether `-cpu` with the fixed AMD profile changes what the application checks (falsifier 4 of section 8).
4. UEFI without Secure Boot was used and booted; the GOP option ROM for kf3 was not tried.

## 13. Package manifest, offline enforcement and run-time injection (2026-11-17)

STATUS: LIVE as tooling, 2026-11-17. Labels as in section 12. **TESTED** = GPU-free unit tests and mutants with scripted guests (`scripts/ci/test_propimage_linux.py`, `scripts/ci/propimage_linux_mutants.py`); **MEASURED** = ran on the coordinator box on 2026-11-17 and is named; **UNVERIFIED** = needs a guest nobody has booted. Run on the coordinator box on 2026-11-17: 236 tests OK (121 new), 67 of 67 mutants killed by at least two named tests (35 new, in both modules), and the gates `doc_freshness`, the claim ledger, `ignore_ladder`, `workflow_lint`, `no_gpu_uuids`, `no_private_refs` and `bandwidth_check` clean. **No VM was booted for this section**: the harden, seal and inject steps were never run against a real guest, only against scripted ones. The code is `scripts/bench/linux/propimage_linux_pkgs.py` (pure functions; the guest transport is passed in) wired into `propimage_linux.py`.

Owner rules, 2026-11-17 (given in the working session; they are the design inputs here):

1. The image is fully offline: no apt and no other network inside a clone. apt may be used only in the SETUP VM, before the seal. There is no allow-list of mirrors and none will be added: `harden-offline` has no `--allow-apt-mirrors` option, on purpose.
2. Every package installed into the sealed image is documented, so the image can be reproduced like any other proprietary image: the seal writes a complete, reviewable package manifest.
3. Python dependencies stay out of the image. Python environments are built at run time from the repository's own scripts (`scripts/kfml`: `setup --venv`, `build-wheels`, hashed locks) and copied into the running offline clone; they are never stored in the sealed image or in the private store.

The wording of this section is deliberately about a private store, isolation and reproducibility. It makes no claim about any vendor's licence, activation or offline behaviour.

### 13.1 Commands

| Command | What it does |
|---|---|
| `seal --dir D [--vendor-entry 'k=v ...']... [--compare PREV] [--require-hardened]` | As section 12, plus: `packages.manifest.json` next to the sealed image (13.2), its sha256 in `state.json` (`packages.manifest_sha256`), WARN lines (13.2), and with `--compare` the diff against a previous manifest. A bad `--vendor-entry` stops the seal before any long step. `--require-hardened` turns the NOT_HARDENED warning into a refusal (the setup VM is still running when it refuses). |
| `pack STATE IMAGE --out ZIP` | Adds `packages.manifest.json` from the image's directory as a member (covered by the sha256 manifest of the zip) and prints a WARN when there is none. Refuses (`PACKAGES_MANIFEST_MISSING_OR_CHANGED`) when the state records a manifest hash and the file is missing or different. `unpack` checks the member's schema and its hash against the state. |
| `diff-packages A B` | A and B are manifests or packed zips; prints what changed (13.2), exit 1 when anything did. |
| `harden-offline --dir D` | Run on the SETUP VM as the last step before the seal (13.3). |
| `unharden --dir D [--clone N]` | A throwaway clone only (13.3). |
| `inject --dir D [--name N] --src PATH --dest GUESTPATH [--max-gb 20] [--replace]` | A file or directory into a running offline clone (13.4). |
| `inject --dir D [--name N] --wheelhouse DIR --lock FILE [--dest GUESTPATH]` | A wheelhouse, installed offline with hashes inside the clone (13.4). |

Order for a real image: `init`, `fetch-base`, `setup`, install and prepare (apt allowed, network on, real clock), `inventory`, then `harden-offline`, then `seal ... --vendor-entry ... --require-hardened`, `scan-image`, `pack`. Anything that needs the network must be done before `harden-offline`; the hardened setup VM cannot go online again (a clone can, with `unharden`, 13.3).

`STAGE_A` now also installs `python3-venv` (so a copied wheelhouse can be installed with a venv inside a clone) and `nftables` (so `harden-offline` needs no apt of its own). The proof image of section 12 predates both: on such an image `harden-offline` runs `apt-get install nftables` itself while the setup VM still has its network, and `python3-venv` must be installed by hand before the seal (`GUEST_NO_VENV` names it otherwise). Both then appear in the manifest like every other package.

### 13.2 The manifest: `packages.manifest.json`

Schema `kayfabe-propimage-linux-packages/v1`, written by the seal from the running guest (after the seal's own cleanup, before the stop), mode as the other members, `json.dumps(sort_keys=True)`. The collector runs one `sudo python3` over ssh that executes a fixed list of read-only commands and returns their raw output; all parsing happens on the host (tested without a guest).

| Field | Content |
|---|---|
| `schema`, `generated_utc` | The schema id and the host time of the seal (ignored by `diff-packages`). |
| `base` | Name, release and sha256 of the pinned Ubuntu cloud image. |
| `os`, `python3` | `/etc/os-release` id, version and codename; `python3 --version`. |
| `kernel` | `release` (`uname -r`), `headers_present`, `proc_version`, and the kernel, headers and meta packages with versions. |
| `dpkg` | Every installed package (`dpkg-query -W -f`, status `ii` and `hi`; removed packages with leftover config are not listed): `name`, `version`, `arch`, `manual` (from `apt-mark showmanual`), `held`. |
| `apt` | `sources_origin` (`guest-live`, `recorded-by-harden-offline`, or `none`), `sources` (path, sha256 of the redacted text, the text; the user-info part of any URL is replaced by `REDACTED`; `auth.conf` is never read), `preferences`, `holds`. After `harden-offline` the guest has no sources, so the record `harden-offline` wrote on the host (`D/setup/apt_sources.recorded.json`, taken before the removal) is used. |
| `snaps`, `pip`, `npm`, `gem` | System-wide packages with versions. `pip` is every distribution Python can see that is not owned by dpkg (`/usr/lib/python3/`); `npm` is `npm ls -g`; `gem` is `gem list --local`; each is empty when the tool is absent. |
| `python_envs` | Paths of every `pyvenv.cfg` under `/opt`, `/usr/local` and `/home` (rule 3: there should be none). |
| `vendor` | One object per `--vendor-entry`, free-form (13.2.1). |
| `large_dirs` | Entries over 1 GB (`du -x -s`) directly under `/opt/*`, `/usr/local/*` and each user's home (a `.cache` is not one). |
| `inventory` | Counts only (no paths) of the files created or changed after provisioning: entries, files, dirs, links, bytes, by reason and by top-level directory; `{"unavailable": ...}` when the provisioning marker is missing. |
| `hardening` | `hardened` (true only when all of 13.3's end state holds in the live guest), `problems`, the live ruleset's counts and drop hooks, the sha256 of `/etc/nftables.conf`, the state of `nftables.service` and of each masked unit. |
| `counts` | dpkg, manual, snaps, pip, npm, gem, vendor. |

The seal prints WARN lines, never silently: `LARGE_DIRS_WITHOUT_VENDOR_ENTRY` (a directory over 1 GB under `/opt`, `/usr/local` or a home, named with its size, and no `--vendor-entry`), `PYTHON_ENV_IN_IMAGE`, `NOT_HARDENED` (with the reasons), `NO_APT_SOURCES_RECORDED`.

#### 13.2.1 Vendor entries

`--vendor-entry 'name=MATLAB release=R2026b products=MATLAB,Parallel_Computing_Toolbox,Deep_Learning_Toolbox installer_sha256=<64 hex> installed_via=<installer|mpm>'`, repeatable. `name` is required; the other keys are free-form lowercase `key=value` words (a value with spaces is quoted, shell style); `products` becomes a list; `installer_sha256` must be 64 hex. A key that names a secret (`password`, `token`, `secret`, `credential`, `licen[sc]e`, `activation`, `api_key`, `key`, `*_key`) and a value that is a URL with credentials are refused (`VENDOR_ENTRY_SECRET_KEY`, `VENDOR_ENTRY_CREDENTIAL`): the manifest records what was installed and how, never anything that signs in.

`diff-packages` (and `seal --compare`) reports, per section, `+` added, `-` removed and `~` changed (`name: old -> new`) for dpkg, apt sources, snaps, pip, npm, gem, vendor entries and large directories, plus a changed kernel release or hardened flag. Identical content, whatever the timestamp, prints `no package-level difference` and exits 0.

### 13.3 Offline enforcement in the guest

`harden-offline --dir D` (ssh into the running setup VM, as root through sudo) does, in this order:

1. Reads the apt sources and preferences and writes them, redacted, to `D/setup/apt_sources.recorded.json` on the host. A second run, when the guest no longer has sources, keeps the first record.
2. Probes the network from inside the guest (TCP to `1.1.1.1:443` and `9.9.9.9:53`, one UDP DNS query, one ping, plus loopback to port 22) to know what was reachable before.
3. Runs the guest script: installs `nftables` only if `nft` is missing (the last use of apt); writes `/etc/nftables.conf` (below); checks it with `nft -c`; enables `nftables.service` so the rules load at every boot; applies it; masks `apt-daily.timer`, `apt-daily-upgrade.timer`, `apt-daily.service`, `apt-daily-upgrade.service` and `unattended-upgrades.service`; sets the periodic apt options to 0; removes `/etc/apt/sources.list`, `/etc/apt/sources.list.d/*`, `/etc/apt/auth.conf*` and `/var/lib/apt/lists/*`.
4. Reads the live ruleset back (`nft -j list ruleset`) and refuses on any rule in the drop chains that is not loopback or established/related (`HARDEN_EXCEPTION`) or when output and forward do not both have policy drop (`HARDEN_NO_DEFAULT_DROP`); checks that every masked unit reports `masked` and no apt source file is left; prints the counts (tables, chains, rules, by kind); probes again from inside the guest and refuses unless nothing outbound answers (`HARDEN_OUTBOUND_OPEN`) and loopback to port 22 still works (`HARDEN_LOOPBACK_BROKEN`). When nothing was reachable before the rules either (the guest was already offline), it says so: the check then cannot show that the rules did it. It writes `D/setup/hardening.json`.

The ruleset is exactly this (inbound is left open: the only listener is sshd, and the clone's NIC is `restrict=on`; replies to the harness's ssh are `established`):

```
flush ruleset
table inet kf_offline {
    chain input   { type filter hook input priority 0; policy accept; }
    chain forward { type filter hook forward priority 0; policy drop; }
    chain output  { type filter hook output priority 0; policy drop;
        oifname "lo" accept
        ct state established,related accept
    }
}
```

There is no exception, no mirror list and no option to add one. **There is no apt in a clone, ever.** A package that is needed must be installed in the setup VM before `harden-offline`, and it will be in the manifest.

`unharden --dir D [--clone N]` is for the online logout step on a THROWAWAY clone (start one with `run --keep --net online`): it flushes the runtime ruleset with `nft flush ruleset` and checks that no drop chain is left. It refuses unless `D/run/N` is a running clone: the name may not leave `D/run`, may not resolve into `D/sealed`, `clone.qcow2` may not be the sealed image and must be a qcow2 overlay whose backing file is the sealed image (`qemu-img info -U`), and the qemu process must be alive. The persistent `/etc/nftables.conf` is left alone (it lives in the clone's overlay), so the sealed image is never touched. To go back online on the setup VM before a seal, do not run `harden-offline` yet.

- **MEASURED** (2026-11-17, coordinator box, nft 1.1.6, `unshare -rn`, no VM): the ruleset above passes `nft -c -f`, loads, and `nft -j list ruleset` has the shape the analyser reads (committed as a test fixture). The guest's nft is Ubuntu 24.04's (1.0.9, CODE READ: same JSON schema version); nothing was run in a guest.
- **UNVERIFIED, falsifier given:** (a) DHCP. The clone gets its address by DHCP from slirp. systemd-networkd's first exchange uses a packet socket, which an `output` hook does not see (not verified here); a lease renewal after half the lease time (slirp's lease is 24 h, not verified) uses a normal UDP socket and would be dropped, so a clone that runs longer than about 12 h might lose its address. Falsifier: boot a hardened clone, check `networkctl status` after the first reboot, and let one run exceed 13 h. A static address in the netplan file would remove the question; it is not done. (b) That `harden-offline` leaves ssh usable on the real guest (inbound ssh and its replies are `established`; not run). (c) That `python3 -c` with `socket.connect` to `1.1.1.1` blocks (timeout) rather than errors on an output drop; either counts as unreachable here.

### 13.4 `inject`: run-time environments into a running clone

`inject` talks to a clone that is already running (`run --keep`, then `--name`, default `clone1`). Before it sends a byte it checks the clone exactly like `run` does: the clone resolution of 13.3 (an overlay of the sealed image, alive), then the identity rows, the network row (no internet, no ping: this is what refuses the setup VM, which has a network) and the recorded-clock window (elapsed time is taken from the clone's pid file); a mismatch refuses with `CLONE_CHECK_FAILED` naming the rows. The kernel-header, compiler and cloud-init rows do not block it.

- **Transport.** tar over the existing ssh transport (`ssh ... bash -c 'tar -x -p --no-same-owner -C DEST'`, the stream written by Python's `tarfile`); no scp. Modes are kept; owners become the guest user; setuid and setgid bits are dropped from the stream; uid, gid and names are zeroed in it. The printed `sha256` is that of the tar stream as the host sent it (with its byte count); it is reproducible for the same source and is NOT a check of what arrived (tar's own exit status is).
- **Destinations.** A relative `--dest` is under `/home/kfimage`; an absolute one must be strictly under `/home/kfimage` or `/opt/kf` (the root itself, `..`, a sibling with the same prefix and everything else are `INJECT_DEST_REFUSED`). `/opt/kf` is created and handed to the guest user through sudo. An existing destination is `INJECT_DEST_EXISTS` unless `--replace` (which removes it first). A directory source lands as the destination directory; a file takes the destination's name.
- **Source rules.** Symlinks are kept only when they are relative and resolve inside the source (`INJECT_SYMLINK_ESCAPES`; an absolute link is refused even when it points into the source, because it would not survive the move); sockets, fifos and devices are `INJECT_SPECIAL_FILE`; a symlink as the source itself is refused. The total of file sizes is capped at `--max-gb` (default 20) and refused before anything is sent (`INJECT_TOO_LARGE`); the stream itself is capped as well.
- **Python, honestly.** Copying a venv works only when the host's Python minor version, libc and CPU ABI match the guest's (Ubuntu 24.04, Python 3.12, glibc 2.39, x86-64), and a venv holds absolute paths and interpreter symlinks (`bin/python3 -> /usr/bin/python3.12`, `pyvenv.cfg`), which `inject --src` refuses by design. The robust variant is a wheelhouse: build it where the repository's scripts run (`scripts/kfml` `build-wheels` and `pip download -r <hashed lock> -d DIR`, see `scripts/kfml/README.md`), then `inject --wheelhouse DIR --lock FILE`.
- **`--wheelhouse DIR --lock FILE`** (host side, before anything is copied): parses the lock; every requirement must be `name==version` with at least one `--hash=sha256:` (`LOCK_NOT_HASHED`, `LOCK_NOT_PINNED`); a requirement that cannot come from a local directory (a URL, a VCS reference, a path, an editable, a nested `-r`) is `LOCK_NEEDS_INDEX`: the clone has no network and no index, and this is the refusal the owner asked for. Index and find-links option lines in the lock (the kfml lock names an extra index for its CUDA wheels) are NOT copied: the staged lock keeps every requirement and hash and loses those lines, and the clone installs with `--no-index` (the original lock's sha256 is printed, so the lock a result cites is still the original). Then, per requirement: a wheel with the pinned name and version must be in the directory (`WHEEL_MISSING`), one of the wheels' sha256 must be in the lock's hash list (`WHEEL_HASH_MISMATCH`), and one of those must fit the guest read from the guest itself (Python minor, machine, glibc; `WHEEL_TAG_MISMATCH`). A requirement with an environment marker and no wheel is a WARN (pip decides in the guest). Only the matching wheels and the staged lock are sent, to `DEST/wheelhouse` and `DEST/requirements.lock.txt` (default `DEST=/opt/kf/pyenv`). In the guest: `python3 -m venv DEST/venv` and `DEST/venv/bin/python -m pip install --no-index --no-input --only-binary=:all: --find-links DEST/wheelhouse --require-hashes -r DEST/requirements.lock.txt`; a failure is `PIP_INSTALL_FAILED` with pip's last lines; a guest without `venv`/`ensurepip` is `GUEST_NO_VENV` (install `python3-venv` in the setup VM). Nothing Python-related is stored in the sealed image or in the private store: the target is the clone's throwaway overlay, and the seal warns when it finds a `pyvenv.cfg`.
- **TESTED** (scripted guest; the stream is extracted by a local `tar -x -p`): destinations, symlink and special-file refusals, the size cap, modes and symlinks surviving the tar round trip, the hash, the wheelhouse checks for all named errors, the staged lock, the install command's flags, the clone guards. **NOT run** against a real clone or a real pip.

### 13.5 Open and not done

1. Nothing here ran on a guest: the first real run of `harden-offline`, a seal with a manifest and an `inject --wheelhouse` is the test of 13.3 and 13.4, and the numbers (counts of packages, rules, wheels, seconds) are not known yet.
2. A static netplan address (13.3, DHCP renewal) is not implemented.
3. `inject` does not verify content on arrival beyond tar's exit status and, for wheels, pip's `--require-hashes` in the guest.
4. The manifest lists what is installed, not how to rebuild it: reproduction means the pinned base image, the apt sources recorded here (a mirror snapshot is not pinned by this tool; package versions are), the vendor entries and the installer hashes they carry.

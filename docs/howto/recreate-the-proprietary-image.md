# Recreate the proprietary Windows image

STATUS: DESIGN-ONLY, 2026-11-13. Nobody has followed this guide from scratch yet. A from-scratch run by a second person is still to do, and until it happens every step below is only as good as its label. This page is not legal advice.

Kayfabe's Windows test image holds software that vendors do not let anyone redistribute: licensed or account-bound benchmarks and design tools. The repository therefore does not ship the image, the installers or any key. This page tells you how to build your own copy with your own licences and accounts. The maintainers' copy lives in a private bucket (see the FAQ entry *Why is there a private storage bucket?*). Yours stays wherever you put it.

How each step is known:

- **MEASURED** means it was done in the maintainers' own build, on the date given, and a file in the repository or the maintainers' notes shows it.
- **CODE READ** means a script in the repository does it, and the script was read but not run end to end for this guide.
- **NOT TESTED** means it is a plan or an assumption. Nobody has done it.

This guide never links to a protected installer or a product key. It links official download pages. You supply every licence, in your own name.

## What you need before you start

| Need | Detail | How known |
|---|---|---|
| A Linux host with KVM, QEMU (distro build for setup, the kayfabe build for the GPU modes), `swtpm`, `qemu-img`, Python 3.12+ | Setup mode uses the distro QEMU window, the later modes use the kayfabe QEMU with its display broker | MEASURED 2026-11-12 (setup and install modes) |
| **swtpm 0.10 or newer** with a matching libtpms | The image's TPM state was written by swtpm 0.10.1. Ubuntu 22.04's swtpm 0.6.3 refused to read it ("Minimum version 4 higher than implementation version 3"); a build of 0.10.1 from source worked | MEASURED 2026-11-13 (traces/hwrun_20261113/README.md) |
| An AMD host CPU | The image runs a fixed CPU model named `kayfabe-amd-v1`; the tool refuses to start on a host that cannot run it. Intel hosts are not supported by this image | CODE READ (`propimage.py`, `CPU_PROFILE_UNSUPPORTED`) |
| An NVIDIA GPU that kayfabe supports, for the install and offline modes | See [Requirements](requirements.md) | MEASURED 2026-11-12 (RTX 4070), 2026-11-13 (RTX 4090) |
| About 300 GB free disk, 12 GB free RAM for one VM | Our finished disk was 62.8 GB after trimming and about 47 GiB as the packed archive | MEASURED 2026-11-12 |
| Your own licences, accounts and a payment method where the vendor charges | See the table in step 6 | not applicable |

You also need the pinned system media. Fetch each from its official place and check the sha256 before you use it.

| File | Official place | sha256 | How known |
|---|---|---|---|
| Windows 11 Pro, multi-edition English (US) ISO, build 26200 (25H2); Pro is image index 6 | [Microsoft's Windows 11 download page](https://www.microsoft.com/software-download/windows11) | `768984706b909479417b2368438909440f2967ff05c6a9195ed2667254e465e3` (8,471,603,200 bytes) | MEASURED 2026-11-12 on the maintainers' copy. Microsoft re-issues this ISO, so yours may differ; then the index and the answer file need a check |
| virtio-win 0.1.302 ISO | [virtio-win archive](https://fedorapeople.org/groups/virt/virtio-win/direct-downloads/archive-virtio/virtio-win-0.1.302-1/virtio-win-0.1.302.iso) | `303f7ae40dad495d6ae474fdc571df58958a4dbc5c37a522d80f9a203867949d` | MEASURED 2026-11-12 |
| OpenSSH for Windows MSI v10.0.0.0 | [Win32-OpenSSH release 10.0.0.0p2-Preview](https://github.com/PowerShell/Win32-OpenSSH/releases/tag/10.0.0.0p2-Preview) | `ddec9c53864280759cf9f74791cefd387100e3946aa849a1c138a4ed1b96b7d9` | MEASURED 2026-11-12 |
| NVIDIA desktop driver 580.88 (WHQL) | [NVIDIA driver downloads](https://www.nvidia.com/Download/index.aspx) (the file `580.88-desktop-win10-win11-64bit-international-dch-whql.exe`) | `90c49f925b41ee062e02cc8094a16b6d057abfde4e85235a76d63ffe267f63a9` | MEASURED 2026-11-12 |

The Windows installation is unactivated: no product key is entered. The maintainers' build never activated Windows. MEASURED 2026-11-12.

## The steps, in order

### 1. Choose a seed and create the state file

One seed string derives every hardware identity of the virtual machine: SMBIOS UUID and serials, MAC address, disk serial, GPU UUID, a private network range, a loopback SSH port and a clock gap. Pick your own seed so your machine differs from the maintainers'. (The maintainers' seed is public, so the machine it makes has public identifiers.) Whether any vendor ties a licence to these identifiers is NOT TESTED; the point of a fixed seed is that they stay the same on every start.

```text
python3 -B -I scripts/bench/windows/propimage.py ids MYSEED
python3 -B -I scripts/bench/windows/propimage.py init MYSEED --out state.json --ssh-key-dir keys/
```

`init` writes the state file and uses the SSH key pair in `keys/` (`id_ed25519` and `id_ed25519.pub`) as the image's own key. The state file also holds the throwaway local password, which is derived from the seed (`propimage.py password state.json`). Both are meant for a loopback-only test machine. Run `init` once. Running it again replaces the key and drops anything recorded later.

How known: the derivation and the file layout are MEASURED 2026-11-12 (the tool's output for the maintainers' seed) and covered by the repository's tests. Running `init` with a fresh seed on a clean checkout is CODE READ.

### 2. Install Windows in setup mode, from the official ISO, with the answer file

Setup mode starts the machine with a standard VGA window and internet access, the real clock, and the same hardware identities as every later mode. It attaches the Windows ISO, the virtio-win ISO and the tools disc (step 3) as CD drives. The answer file (`propimage_autounattend.xml.in`, delivered on the tools disc) chooses Windows 11 Pro, partitions the disk, creates an offline local administrator account and loads the virtio storage and network drivers. It also contains the other deviations listed in the table below.

```text
PROP_DIR=/path/to/workdir scripts/bench/windows/propimage_run.sh setup
```

You first need, in the work directory: the state file, the `media/` folder with `win.iso`, `virtio-win.iso` and `tools.iso`, a thin 160 GB `image.qcow2`, and the OVMF Secure Boot variables file at `firmware/OVMF_VARS.fd` (a copy of the distro's Microsoft-keys variables file). Creating those files and the `tools.iso` image is hand work that the repository's scripts do not do; the maintainers did it once by hand. The UEFI "Press any key to boot from CD" prompt lasts a few seconds. Press a key, or reset the machine and press again.

How known: starting setup mode, reaching the Windows Setup screen and the answer file's language settings applying are MEASURED 2026-11-12. The maintainers' install itself finished (Windows 11 Pro 26200 booted, SSH worked). Whether the answer file alone completes the whole install without a click was not recorded: treat the account, edition and partition parts as CODE READ, and keep the manual fallback in mind ("I don't have a product key", pick Windows 11 Pro, local account).

### 3. OpenSSH and the key, from the tools disc

`propimage.py tools` assembles one directory for a CD labelled KFTOOLS: the answer file, the OpenSSH MSI, the NVIDIA installer and one small script, `setup_ssh.ps1`. It checks both installers against the pins above first. Turn that directory into an ISO with any ISO tool (the command is not part of the repository; NOT TESTED).

```text
python3 -B -I scripts/bench/windows/propimage.py tools state.json --msi OpenSSH-Win64-v10.0.0.0.msi --nvidia 580.88-desktop-win10-win11-64bit-international-dch-whql.exe --out tools-dir/
```

In the guest, open an administrator PowerShell and run the script from the disc (the drive letter varies):

```text
powershell -ExecutionPolicy Bypass -File D:\setup_ssh.ps1
```

The script checks the MSI hash, installs the client and server, sets the SSH service to start automatically, adds the inbound TCP 22 rule if missing, sets Windows PowerShell as the default SSH shell, and writes the image's public key into the administrators' key file. It never receives a private key.

How known: the tools directory, the hot-plugged disc, SSH login with the key, and the guest's identity checks matching the seed are MEASURED 2026-11-12. In that run the owner installed OpenSSH by hand rather than by running the script, so the script itself has never run in a guest: CODE READ. The MSI feature names inside it are marked unverified in the design notes.

### 4. The NVIDIA driver

Restart the machine in install mode: the kayfabe QEMU, the GPU device with its display broker, internet still on. Over SSH, install the 580.88 driver with the repository's `scripts/bench/windows/gsp_on.ps1` (the driver files copied across the same SSH connection). It writes the `EnableGpuFirmware` setting that kayfabe's GPU needs, once in the display class key and once in the `nvlddmkm` service key. Then set the DWM `OverlayTestMode` value to 5 (see the deviations table), reboot, and enable the device if it is left disabled.

How known: MEASURED 2026-11-12 on an RTX 4070: driver 580.88 installed through that route, both keys read back as 1, the device came up with no error after an explicit enable, and `nvidia-smi` in the guest showed the seed-derived GPU UUID. `gsp_on.ps1` without `-NoEnable` enables the device itself, which is the better call (MEASURED that the `-NoEnable` form left it disabled). The working directory the script leaves in the guest was deleted afterwards.

### 5. Install each app from its official installer, with default settings

Use the installer from the vendor's own page, run it over the desktop with the default options, and delete the installer afterwards to save disk. Do not change install paths: the check script expects the default locations.

| App | Official page | Version in the maintainers' image | Licence or account you need | How known |
|---|---|---|---|---|
| 3DMark (Advanced edition) | [UL Benchmarks](https://benchmarks.ul.com/3dmark) | 2.32.0 per the maintainers' file name; the download was a zip of about 12.9 GB with 25 files | A paid key. The maintainers bought 3DMark Advanced | MEASURED 2026-11-12 (installed, registered, started offline 2026-11-12) |
| SolidWorks Design Standard, student edition | [SolidWorks](https://www.solidworks.com/) | 2026 SP5.2 "Academic Use Only" | A student serial that comes from a request form on the vendor's site. The installer is a small downloader (217 MB, signed by the vendor, sha256 `5230b4e1dce3f23b9f6c41ccda7aad8f7096e0f832220cfbb68282cd5f8078c8` on 2026-11-12) that fetches the rest while the guest is online, so the download changes over time | MEASURED 2026-11-12 (installed, started offline 2026-11-12) |
| DaVinci Resolve (free) | [Blackmagic Design](https://www.blackmagicdesign.com/products/davinciresolve) | 21 | No licence for the free edition. The download page asks for contact details in a form | MEASURED 2026-11-12 (started offline to the Project Manager) |
| Unigine Superposition (Basic) | [Unigine benchmark page](https://benchmark.unigine.com/superposition) | not recorded | None | MEASURED 2026-11-13 (a full run completed offline on the image; results screen seen) |
| eDrawings | [SolidWorks](https://www.solidworks.com/) | not recorded | None | MEASURED 2026-11-13 (a UI tree dump was taken; it is installed). No run beyond launch |
| Black Myth: Wukong Benchmark Tool | [Steam store](https://store.steampowered.com/) | not recorded | A Steam account. Use a throwaway account, never a personal one | MEASURED 2026-11-13 for the launch dialog only: Steam's launch dialog came up and the tool itself was not run |
| Steam client | [Steam](https://store.steampowered.com/about/) | not recorded | The same throwaway account, for the install only | MEASURED 2026-11-12 |

Steam stays signed out in the finished image. Log in once to install the benchmark tool, then sign out; do not leave a saved password or cached login behind. Whether the tool still launches signed out has been seen only as the launch dialog (MEASURED 2026-11-13); that it runs signed out is NOT TESTED.

Which apps the repository's automated check starts is listed in `propimage_check.py` (CODE READ). The pass rules for each app, and what the check does not prove, are in the design notes on app automation.

### 6. Enter your licences

Do this while the machine is online, in install mode, on the console window. Type the licence into the app yourself; never put a key into a script, a file in the repository or the state file.

| App | What to do | How known |
|---|---|---|
| 3DMark | Enter your key in the app and let it register | MEASURED 2026-11-12 (the maintainers' key, not recorded here) |
| SolidWorks | Enter your student serial during the installer | MEASURED 2026-11-12 |
| Steam | Sign in, install the benchmark tool, sign out | MEASURED 2026-11-12 |
| DaVinci Resolve, Superposition, eDrawings | Nothing to enter | MEASURED 2026-11-12 |

Whether each licence keeps working with no internet is the next step's job.

### 7. Test offline, on a throwaway overlay, with a faked clock

Shut the machine down cleanly first. Then test the way the image will be used: offline mode, no internet, with the clock set from the state file. The test uses a disposable overlay of the disk, never the disk itself.

```text
python3 -B -I scripts/bench/windows/propimage_check.py --dir unpacked/ --work /path/to/fresh-workdir --out results/ --qemu /path/to/kayfabe-qemu --flags kf3_flags.sample.env --broker /path/to/broker
```

(Use `--zip` with the archive instead of `--dir` the first time; see step 10.) The script checks that `swtpm` is 0.10 or newer, creates the overlay, boots with the network restricted, SSHes in, applies the per-boot repairs (see the deviations table), checks the machine's identity against the state file, then starts each app and records a result. A launch alone is recorded as a launch only, never as a pass.

How known: the offline boot of the maintainers' copy on 2026-11-13 is MEASURED (identity, TPM, CPU, clock window matched; SolidWorks, 3DMark and DaVinci Resolve started offline). The unified script ran end to end on 2026-11-14 on the maintainers' PC (offline overlay, RTX 4070, three boots): MEASURED results are 3DMark Time Spy PASS from the saved `.3dmark-result` file (score 7400; the command-line client refuses this key with "Professional or Developer license key required", so the run goes through the UI), Superposition PASS from its saved log, Edge PASS (WebGL drew frames on the NVIDIA GPU, read through UI Automation) and Black Myth Wukong FAIL in two of five runs (game crash with a GPU crash dump and a Windows GPU watchdog event; the other runs stayed up at the menu, which proves nothing about the benchmark). eDrawings (licence dialog not dismissed), SolidWorks (COM error) and DaVinci Resolve (launch only) have no verdict yet: NOT TESTED beyond launch. A full clean pass of every app has not happened. The Storage Benchmark shows a buy card because it is not part of the Advanced edition; that says nothing about activation.

### 8. Back up before any sign-out or deregistration

While the machine is still registered and signed in, record the facts Windows and swtpm generate (the machine GUID and the TPM endorsement-key hash) plus the guest's current time, which becomes the stop time. Then shut down cleanly and compress the disk.

```text
python3 -B -I scripts/bench/windows/propimage.py verify state.json --ssh-key keys/id_ed25519 --workdir /path/to/workdir --record
qemu-img convert -c -O qcow2 image.qcow2 image.packed.qcow2
```

Record before the backup, because the stop time must be the real last use. The maintainers' first packed archive carried a stale stop time and had to be corrected (MEASURED 2026-11-12). Once an image is backed up, editing it offline means making the backup again.

How known: the `--record` and `convert` steps are MEASURED 2026-11-12 (the disk went from 117 GB to 62.8 GB after trimming; the maintainers' archive was about 47 GiB). The `--record` option on a fresh state is CODE READ.

### 9. Pack with the manifest

The package is one archive with exactly three members, stored uncompressed: `image.qcow2` (already compressed by the convert), `state.json` and `manifest.json` with the sha256 of each member. Packing also copies the TPM files and the OVMF variables from the work directory into the state file.

```text
python3 -B -I scripts/bench/windows/propimage.py pack state.json image.packed.qcow2 --out image.zip --workdir /path/to/workdir
python3 -B -I scripts/bench/windows/propimage.py unpack image.zip --dir unpacked/ --workdir /path/to/testdir
```

Unpack the archive once yourself and run the offline test of step 7 on the unpacked copy. `unpack` verifies every member and the seed, and refuses a state whose ids do not match the seed.

How known: pack, unpack and the member checks are MEASURED (2026-11-12 for the archive, 2026-11-13 for a verified unpack on another machine).

### 10. Upload to private storage

Keep the archive where only you can read it: a private bucket, an encrypted disk, an offline backup. The state file inside it holds a throwaway password, a throwaway key pair, the TPM state and the machine's identifiers. Your vendor passwords are not meant to be in it (INFERRED from the design; nobody has searched a finished archive for them), but the design keeps 3DMark's registration in the disk. Treat the archive as a licensed product. Do not post it, and do not hand it to anyone else.

How known: the maintainers' upload to their own private bucket and a download with a checksum match are MEASURED 2026-11-12. Any other storage is NOT TESTED.

### 11. Deregister, on the online copy only, after the offline image exists

Only now sign out and deregister: start a copy of the original (online) disk, sign out of Steam, deregister the 3DMark key on the vendor's options page, confirm on the vendor's website, then discard that copy. Never do this on the offline image. Deregistering earlier would unlicense the image you just built.

How known: the maintainers did this after their upload, on an overlay of the packed disk, on 2026-11-12 (MEASURED that it was started; completion was by hand and is not shown by a file). Which vendor registrations the image still needs after this is NOT TESTED.

## Using the image afterwards

- **The image is immutable.** The scripts come from the repository over SSH on every boot. Nothing is written back to the image. CODE READ (`propimage_check.py`: the PowerShell travels on SSH standard input).
- **Always offline.** Every boot of the image uses the restricted slirp network (`restrict=on`), never internet. Never boot the base disk itself; boot a throwaway overlay of it. CODE READ (the script refuses to start unless the command line has `restrict=on` and the overlay path); MEASURED 2026-11-13.
- **The clock is faked and only moves forward.** A fresh overlay starts at the state's last stop time plus the seed's gap, never earlier than the last real use. Windows keeps its own floor and clamps earlier times (MEASURED 2026-11-12: bases set to 2025 came up at the old floor). To restart the same overlay, pass `--resume`: a small sidecar file next to the overlay carries the clock forward across clean stops. CODE READ; the sidecar has not been run on hardware.
- **Die-binding.** The image is bound to the GPU it was made on in one respect. A different GPU gives the guest a new display device instance with no `EnableGpuFirmware` key, and the driver shows Code 43. The check script sets the key on each new instance every boot. MEASURED 2026-11-13: image made on an RTX 4070, Code 43 on an RTX 4090 until the key was set on the new instance; then `nvidia-smi` showed the seed GPU UUID.
- **swtpm 0.10 or newer** is required, as above.

### Known limit: the Windows evaluation timer

On 2026-11-13 the maintainers' Windows 11 evaluation test image (a different image, used for the SPECviewperf runs) shut itself down 62 minutes after boot, and in another run 60 minutes after boot. The shutdown was System event 1074 from `wlms.exe` ("The license period for this installation of Windows has expired"). The check script keeps each boot under about 55 minutes and records such a shutdown as a harness result, not an app failure. Whether your unactivated Windows 11 Pro image shows the same timer has not been measured: NOT TESTED. Treat about 55 minutes of uptime as the limit for one boot, and restart before it.

## Deviations from a default install

Every setting below differs from what you would get by clicking through Windows, the vendor installers and the NVIDIA installer with defaults. Read this table before you trust a result from the image.

| Deviation | Why | Baked in or per boot | How known |
|---|---|---|---|
| OpenSSH client and server, service set to automatic, TCP 22 inbound rule, the image's public key in the administrators' key file, Windows PowerShell as the default SSH shell | The test harness reaches the guest over SSH | Baked in | CODE READ (`propimage.py`, `setup_ssh.ps1`); the default-shell change MEASURED 2026-11-12 |
| Local administrator account with autologon, and the first-run screens hidden (online account, wireless, OEM, privacy) | Offline account creation and a desktop for app checks | Baked in | CODE READ (answer file). That autologon survives every restart is NOT TESTED |
| NVIDIA driver 580.88 installed through the INF route | The guest driver pair that kayfabe is measured with | Baked in | MEASURED 2026-11-12 |
| `EnableGpuFirmware` = 1 in the display class key and in `nvlddmkm` | Without it the guest GPU shows Code 43 | Baked in for the first instance, set again for every new display instance on each boot | MEASURED 2026-11-12 and 2026-11-13 |
| DWM `OverlayTestMode` = 5 | The Start menu reset the display driver without it | Baked in (a manual image change) | MEASURED 2026-11-12 for the effect on that image; not re-qualified since |
| Fast startup off (`HiberbootEnabled` 0), hibernation off, sleep, hybrid sleep, monitor and disk timeouts off on AC and DC, wake sign-in off, lock-screen policy off, screensaver off | A suspended or restored Windows meeting a fresh kayfabe, or a locked desktop, breaks unattended checks | Fast startup baked in by the answer file; the rest applied every boot over SSH by `propimage_check.py` (`power_fix`) | CODE READ; no hardware measurement of the repair |
| The system clock | Fixed starting time so licences and certificates behave the same | Per boot: the virtual machine's RTC is set from the state file | CODE READ; MEASURED 2026-11-12 and 2026-11-13 (guest time read back in the window) |
| US English locale, UTC, fixed computer name, Windows 11 Pro, disk 0 wiped, EFI 300 MiB, MSR 16 MiB, rest NTFS | A repeatable install | Baked in | CODE READ (answer file) |
| virtio storage driver during setup, plus the network, serial, RNG, panic and firmware-config drivers; no memory-balloon driver | Virtual disk and network; no guest-directed RAM resizing | Baked in | CODE READ |
| `BypassCPUCheck` only; the TPM and Secure Boot checks stay on | The fixed AMD CPU model is older than Windows 11 officially accepts | Setup only | CODE READ |
| BitLocker device encryption prevented | A reproducible test disk | Baked in | CODE READ |
| Windows Update dynamic update off, automatic update off, driver updates through Windows Update excluded | No unexpected traffic or driver replacement | Baked in | CODE READ |
| Virtualization-based security and memory integrity off | No nested security virtualization | Baked in | CODE READ; the running state is not measured |
| Kernel crash dump settings and visible bugcheck text | Diagnostics | Baked in | CODE READ |
| Smart App Control policy value set to 0 | The harness runs unsigned CUDA probes | Baked in | CODE READ |
| Boot display firmware signing certificate added to the Secure Boot database (OVMF variables) | The boot-display firmware is signed with kayfabe's own key | Baked in the firmware variables file in the state | MEASURED 2026-11-12 |
| A Windows firewall rule named `kayfabe-block-internet`, blocking outbound Internet | Extra layer, hand-added during cleanup, on top of the network restriction; the offline protection comes from qemu `restrict=on`, not from this rule | Baked in before packing; disabled in the online copy only | MEASURED 2026-11-12 that it was added by hand; the repository has no file for it, so NOT reproducible from this page |
| Run-time scripts and results under `C:\kfapps` and `C:\kfhw`, plus a temporary restart marker | Evidence collection in the disposable overlay | Per boot, in the overlay only | CODE READ |

The answer file may still carry choices that a future, more minimal image drops. The design notes (*Deviations from a default install* in the app-automation design document) list the same settings with the code that sets each one.

## What has and has not been shown

- The maintainers built the image once, by hand with the repository's helper scripts, between 2026-11-12 and 2026-11-13, and ran parts of the offline check on it.
- NOT TESTED: that someone else can follow this page from an empty disk to a working image, the ISO-making and the empty-disk and firmware-variable preparation (hand work), `setup_ssh.ps1` in a guest, `--resume` on hardware, an end-to-end pass of `propimage_check.py`, whether any vendor ties a licence to the seed's hardware identifiers, and the behaviour of any storage other than the maintainers' own.
- Nothing here depends on, or is stored in, a public release build.

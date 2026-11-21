# Recreate the proprietary Windows image

Nobody outside the project has followed this guide from scratch yet, so treat steps marked "not verified" with care. Note that an in-app sign-out does not revoke a login at the vendor: see *Token revocation and the online token check*. This page is not legal advice and makes no statement about the licence terms of any application.

Kayfabe's Windows tests include commercial design, video and 3D applications and benchmarks. A *proprietary test image* is a private Windows disk image that the project's maintainers build to run those applications under kayfabe. This page tells you how to build your own, with your own accounts and licences. The policy in one page is [Proprietary test images](proprietary-test-images.md).

## What this repository does and does not contain

- The repository contains **no** proprietary software, installers, licence keys, account data or disk images. It contains the scripts, the answer file and the notes that this page describes.
- The maintainers' images are stored in a private object store. They are never published, never in git and never served publicly. A rented test box receives an image only through a time-limited, authenticated download. This is the maintainers' policy: the repository's code does not show an image being fetched this way (see [Proprietary test images](proprietary-test-images.md), *How the maintainers' images reach a test box*).
- **The maintainers' images are not shared with anyone**, contributors included. Anyone who wants to reproduce the proprietary-application tests builds their own image from this page, with their own accounts and licences. The offline tests then run against that image.
- Kayfabe's open Windows lanes need none of this: they run open or freely downloadable software on a Windows guest that has internet access. See [Proprietary test images](proprietary-test-images.md).

Where a step has not been verified, the text says so in words.

This guide never links to a protected installer or a product key. It links official download pages. You supply every account and licence, in your own name.

## What you need before you start

| Need | Detail |
|---|---|
| A Linux host with KVM, QEMU (distro build for setup, the kayfabe build for the GPU modes), `swtpm`, `qemu-img`, Python 3.12+ | Setup mode uses the distro QEMU window, the later modes use the kayfabe QEMU with its display broker |
| **swtpm 0.10 or newer** with a matching libtpms | The image's TPM state was written by swtpm 0.10.1. Ubuntu 22.04's swtpm 0.6.3 refused to read it ("Minimum version 4 higher than implementation version 3"); a build of 0.10.1 from source worked |
| An AMD host CPU | The image runs a fixed CPU model named `kayfabe-amd-v1` (AMD vendor). The host CPU vendor must match the image's CPU profile, and the tool refuses to start on a host that cannot run it. Intel hosts are not supported by an image built on this profile; an Intel host would need a second image, built on an Intel host with its own profile (that profile does not exist yet). An Intel host that presented the AMD vendor to the guest hung in Windows Boot Manager |
| An NVIDIA GPU that kayfabe supports, for the install and offline modes | See [Requirements](requirements.md) | Worked on an RTX 4070 and an RTX 4090 |
| About 300 GB free disk, 12 GB free RAM for one VM | Our finished disk was 62.8 GB after trimming and about 47 GiB as the packed archive |
| Your own accounts, licences and a payment method where the vendor charges | See the table in step 6 |

You also need the pinned system media. Fetch each from its official place and check the sha256 before you use it.

| File | Official place | sha256 |
|---|---|---|
| Windows 11 Pro, multi-edition English (US) ISO, build 26200 (25H2); Pro is image index 6 | [Microsoft's Windows 11 download page](https://www.microsoft.com/software-download/windows11) | `768984706b909479417b2368438909440f2967ff05c6a9195ed2667254e465e3` (8,471,603,200 bytes). This is the maintainers' copy. Microsoft re-issues this ISO, so yours may differ; then the index and the answer file need a check |
| virtio-win 0.1.302 ISO | [virtio-win archive](https://fedorapeople.org/groups/virt/virtio-win/direct-downloads/archive-virtio/virtio-win-0.1.302-1/virtio-win-0.1.302.iso) | `303f7ae40dad495d6ae474fdc571df58958a4dbc5c37a522d80f9a203867949d` |
| OpenSSH for Windows MSI v10.0.0.0 | [Win32-OpenSSH release 10.0.0.0p2-Preview](https://github.com/PowerShell/Win32-OpenSSH/releases/tag/10.0.0.0p2-Preview) | `ddec9c53864280759cf9f74791cefd387100e3946aa849a1c138a4ed1b96b7d9` |
| NVIDIA desktop driver 580.88 (WHQL) | [NVIDIA driver downloads](https://www.nvidia.com/Download/index.aspx) (the file `580.88-desktop-win10-win11-64bit-international-dch-whql.exe`) | `90c49f925b41ee062e02cc8094a16b6d057abfde4e85235a76d63ffe267f63a9` |

The Windows installation is unactivated: no product key is entered. The maintainers' build never activated Windows.

## The steps, in order

The lifecycle of an image, in order. The numbered steps below carry the commands.

1. Install online, with the real clock (steps 2 to 6).
2. Go offline and check that every application starts and works without internet (step 7).
3. Shut down. Back up the disk and the state **before** any account revocation (step 8). This backup is the sealed image.
4. On the **original** disk (or a separate copy), online: sign out of every account, unlink the device, change the password and end all sessions at the vendor, then discard that disk (step 11).
5. Test the backup the way CI would use it: a clone, the same fixed hardware identity, offline, the deterministic clock (steps 7 and 9).
6. Pack it (step 9) and store it privately (step 10).

The order of steps 3 and 4 is the point: see *Why the sign-in state is revoked at the vendor before the image is distributed* after step 11. Steps 9 to 11 are numbered in the order the maintainers did them; only the backup of step 8 must come before step 11.

### 1. Choose a seed and create the state file

One seed string derives every hardware identity of the virtual machine: SMBIOS UUID and serials, MAC address, disk serial, GPU UUID, a private network range, a loopback SSH port and the small gap that the deterministic clock adds (step 7). The identity is fixed per image so that test runs are repeatable: the machine looks the same on every start. Pick your own seed so your machine differs from the maintainers'. (The maintainers' seed is public, so the machine it makes has public identifiers.)

```text
python3 -B -I scripts/bench/windows/propimage.py ids MYSEED
python3 -B -I scripts/bench/windows/propimage.py init MYSEED --out state.json --ssh-key-dir keys/
```

`init` writes the state file and uses the SSH key pair in `keys/` (`id_ed25519` and `id_ed25519.pub`) as the image's own key. The state file also holds the throwaway local password, which is derived from the seed (`propimage.py password state.json`). Both are meant for a loopback-only test machine. Run `init` once. Running it again replaces the key and drops anything recorded later.

The derivation and the file layout are covered by the repository's tests. Running `init` with a fresh seed on a clean checkout has not been verified.

### 2. Install Windows in setup mode, from the official ISO, with the answer file

Setup mode starts the machine with a standard VGA window and internet access, the real clock (the vendors' sites are expected to need correct dates for TLS), and the same hardware identities as every later mode. It attaches the Windows ISO, the virtio-win ISO and the tools disc (step 3) as CD drives. The answer file (`propimage_autounattend.xml.in`, delivered on the tools disc) chooses Windows 11 Pro, partitions the disk, creates an offline local administrator account and loads the virtio storage and network drivers. It also contains the other deviations listed in the table below.

```text
PROP_DIR=/path/to/workdir scripts/bench/windows/propimage_run.sh setup
```

You first need, in the work directory: the state file, the `media/` folder with `win.iso`, `virtio-win.iso` and `tools.iso`, a thin 160 GB `image.qcow2`, and the OVMF Secure Boot variables file at `firmware/OVMF_VARS.fd` (a copy of the distro's Microsoft-keys variables file). Creating those files and the `tools.iso` image is hand work that the repository's scripts do not do; the maintainers did it once by hand. The UEFI "Press any key to boot from CD" prompt lasts a few seconds. Press a key, or reset the machine and press again.

Setup mode reaches the Windows Setup screen and the answer file's language settings apply. The maintainers' install finished (Windows 11 Pro 26200 booted, SSH worked). Whether the answer file alone completes the whole install without a click is not verified: keep the manual fallback in mind ("I don't have a product key", pick Windows 11 Pro, local account).

### 3. OpenSSH and the key, from the tools disc

`propimage.py tools` assembles one directory for a CD labelled KFTOOLS: the answer file, the OpenSSH MSI, the NVIDIA installer and one small script, `setup_ssh.ps1`. It checks both installers against the pins above first. Turn that directory into an ISO with any ISO tool (the command is not part of the repository and has not been verified).

```text
python3 -B -I scripts/bench/windows/propimage.py tools state.json --msi OpenSSH-Win64-v10.0.0.0.msi --nvidia 580.88-desktop-win10-win11-64bit-international-dch-whql.exe --out tools-dir/
```

In the guest, open an administrator PowerShell and run the script from the disc (the drive letter varies):

```text
powershell -ExecutionPolicy Bypass -File D:\setup_ssh.ps1
```

The script checks the MSI hash, installs the client and server, sets the SSH service to start automatically, adds the inbound TCP 22 rule if missing, sets Windows PowerShell as the default SSH shell, and writes the image's public key into the administrators' key file. It never receives a private key.

The tools directory, the hot-plugged disc, SSH login with the key, and the guest's identity checks matching the seed worked in the maintainers' build. There OpenSSH was installed by hand rather than by running the script, so the script itself has not been run in a guest. The MSI feature names inside it are not verified.

### 4. The NVIDIA driver

Restart the machine in install mode: the kayfabe QEMU, the GPU device with its display broker, internet still on. Over SSH, install the 580.88 driver with the repository's `scripts/bench/windows/gsp_on.ps1` (the driver files copied across the same SSH connection). It writes the `EnableGpuFirmware` setting that kayfabe's GPU needs, once in the display class key and once in the `nvlddmkm` service key. Then set the DWM `OverlayTestMode` value to 5 (see the deviations table), reboot, and enable the device if it is left disabled.

On an RTX 4070, driver 580.88 installed through that route, both keys read back as 1, the device came up with no error after an explicit enable, and `nvidia-smi` in the guest showed the seed-derived GPU UUID. `gsp_on.ps1` without `-NoEnable` enables the device itself, which is the better call (the `-NoEnable` form left it disabled). The working directory the script leaves in the guest was deleted afterwards.

### 5. Install each app from its official installer, with default settings

Use the installer from the vendor's own page, run it over the desktop with the default options, and delete the installer afterwards to save disk. Do not change install paths: the check script expects the default locations.

| App | Official page | Version in the maintainers' image | Account or key you need |
|---|---|---|---|
| 3DMark (Advanced edition) | [UL Benchmarks](https://benchmarks.ul.com/3dmark) | 2.32.0 per the maintainers' file name; the download was a zip of about 12.9 GB with 25 files | A key from the vendor for the Advanced edition (a paid product) |
| SolidWorks Design Standard, student edition | [SolidWorks](https://www.solidworks.com/) | 2026 SP5.2 "Academic Use Only" | A serial that comes from a request form on the vendor's site; whether you qualify for the student edition is between you and the vendor. The installer is a small downloader (217 MB, signed by the vendor, sha256 `5230b4e1dce3f23b9f6c41ccda7aad8f7096e0f832220cfbb68282cd5f8078c8` on 2026-11-12) that fetches the rest while the guest is online, so the download changes over time |
| DaVinci Resolve (free) | [Blackmagic Design](https://www.blackmagicdesign.com/products/davinciresolve) | 21 | No key for the free edition. The download page asks for contact details in a form |
| Unigine Superposition (Basic) | [Unigine benchmark page](https://benchmark.unigine.com/superposition) | not recorded | None |
| eDrawings | [SolidWorks](https://www.solidworks.com/) | not recorded | None |
| Black Myth: Wukong Benchmark Tool | [Steam store](https://store.steampowered.com/) | not recorded | A Steam account. Use a throwaway account, never a personal one |
| Steam client | [Steam](https://store.steampowered.com/about/) | not recorded | The same throwaway account, for the install only |

Log in to Steam once to install the benchmark tool, then sign out before the backup; do not leave a saved password behind. With Steam signed out, the tool's launch dialog has been seen; that the tool runs signed out is not verified.

Which apps the repository's automated check starts is listed in `propimage_check.py`.

### 6. Sign in or register each application

Do this while the machine is online, in install mode, on the console window, with the real clock. Where an application asks for an account or a key, type your own into the application yourself; never put a key or password into a script, a file in the repository or the state file.

| App | What to do |
|---|---|
| 3DMark | Enter your own key in the app and let it register |
| SolidWorks | Enter your own serial during the installer |
| Steam | Sign in, install the benchmark tool, sign out |
| DaVinci Resolve, Superposition, eDrawings | Nothing to enter |

Whether each application starts and works with no internet is the next step's job.

### 7. Test offline, on a throwaway overlay, with the deterministic clock

Shut the machine down cleanly first. Then test the way the image will be used: offline mode (user-mode network with `restrict=on`, no internet) and the deterministic clock, which the tool sets from the state file (see *Using the image afterwards*). The question for each application is whether it starts and works without internet. The test uses a disposable overlay of the disk, never the disk itself.

```text
python3 -B -I scripts/bench/windows/propimage_check.py --dir unpacked/ --work /path/to/fresh-workdir --out results/ --qemu /path/to/kayfabe-qemu --flags kf3_flags.sample.env --broker /path/to/broker   # the flags file is optional measurement flags; Windows needs none
```

(Use `--zip` with the archive instead of `--dir` the first time; see step 10.) The script checks that `swtpm` is 0.10 or newer, creates the overlay, boots with the network restricted, SSHes in, applies the per-boot repairs (see the deviations table), checks the machine's identity against the state file, then starts each app and records a result. A launch alone is recorded as a launch only, never as a pass.

Results in the maintainers' build: the offline boot of their copy matched on identity, TPM, CPU and clock window, and SolidWorks, 3DMark and DaVinci Resolve started offline. The unified script ran end to end on an RTX 4070 (offline overlay, three boots), with these results: 3DMark Time Spy PASS from the saved `.3dmark-result` file (score 7400; the command-line client refuses this key with "Professional or Developer license key required", so the run goes through the UI), Superposition PASS from its saved log, Edge PASS (WebGL drew frames on the NVIDIA GPU, read through UI Automation) and Black Myth Wukong FAIL in two of five runs (game crash with a GPU crash dump and a Windows GPU watchdog event; the other runs stayed up at the menu, which proves nothing about the benchmark). eDrawings (licence dialog not dismissed), SolidWorks (COM error) and DaVinci Resolve (launch only) have no verdict yet; nothing beyond launch has been verified. A full clean pass of every app has not happened. The Storage Benchmark shows a buy card in the maintainers' copy because it is not part of the Advanced edition.

DaVinci Resolve row: `propimage_check.py --apps resolve` now renders five deliverables from open media (Blender CC BY clips, fetched from their sources or `https://bundles.kayfabe.dev/test-assets/20261116/`, sha256 pinned) and asserts that the GPU was used: it needs network access on the HOST only (the guest stays offline), `ffprobe`/`ffmpeg` on the host are used when present, and `--resolve-launch-only` restores the old create-a-project check. `--resolve-media DIR` chooses where the media is kept. The free edition has no NVIDIA video encoder, so the H.264 and H.265 renders encode in software and the GPU work is the colour and scaling pipeline; the row says so.

### 8. Back up before any sign-out or deregistration

While the machine is still registered and signed in, record the facts Windows and swtpm generate (the machine GUID and the TPM endorsement-key hash) plus the guest's current time, which becomes the stop time. Then shut down cleanly and compress the disk.

```text
python3 -B -I scripts/bench/windows/propimage.py verify state.json --ssh-key keys/id_ed25519 --workdir /path/to/workdir --record
qemu-img convert -c -O qcow2 image.qcow2 image.packed.qcow2
```

Record before the backup, because the stop time must be the real last use. The maintainers' first packed archive carried a stale stop time and had to be corrected. Once an image is backed up, editing it offline means making the backup again.

In the maintainers' build the disk went from 117 GB to 62.8 GB after trimming, and the archive was about 47 GiB. The `--record` option on a fresh state has not been verified.

### 9. Pack with the manifest

The package is one archive with exactly three members, stored uncompressed: `image.qcow2` (already compressed by the convert), `state.json` and `manifest.json` with the sha256 of each member. Packing also copies the TPM files and the OVMF variables from the work directory into the state file.

```text
python3 -B -I scripts/bench/windows/propimage.py pack state.json image.packed.qcow2 --out image.zip --workdir /path/to/workdir
python3 -B -I scripts/bench/windows/propimage.py unpack image.zip --dir unpacked/ --workdir /path/to/testdir
```

Unpack the archive once yourself and run the offline test of step 7 on the unpacked copy. `unpack` verifies every member and the seed, and refuses a state whose ids do not match the seed.

Pack, unpack and the member checks worked in the maintainers' build, including an unpack on another machine.

### 10. Upload to private storage

Keep the archive where only you can read it: a private bucket, an encrypted disk, an offline backup. The state file inside it holds a throwaway password, a throwaway key pair, the TPM state and the machine's identifiers. Your vendor passwords are not meant to be in it (this is the design intent; nobody has searched a finished archive for them), but the design keeps 3DMark's registration state in the disk, because the backup is taken before the sign-out of step 11. Treat the archive as private: do not post it, do not commit it and do not hand it to anyone else.

The maintainers' upload to their own private bucket and a download with a checksum match worked. Any other storage has not been verified.

### 11. Sign out and unregister, on the original online disk only, after the backup exists

**Read this first:** for some vendors an in-app sign-out is local only and does not revoke the login at the vendor. Signing out here is therefore not enough, and the revocation that protects the image is done at the vendor after the seal and then checked. See *Token revocation and the online token check* below, which takes precedence over the wording of this step.

Only now revoke the accounts: start the original (online) disk, or a separate copy of it, and sign out of every account, unlink the device, deregister keys on the vendors' own pages (for example the 3DMark key), change the password and end all sessions at each vendor, confirm on each vendor's website, then discard that disk. Never do this on the sealed image. Do it only after the backup exists: signing out or deregistering first would change the state the backup is meant to capture.

The maintainers did this by hand after their upload, on an overlay of the packed disk. Completion is not shown by any file.

#### Why the sign-in state is revoked at the vendor before the image is distributed

This is the project's procedure. The tool cannot see vendor-side state, so nothing in the repository verifies it.

The backup is taken first (step 8). Then, on a separate copy of the disk, the owner signs out of every account, unlinks the device, changes the password and ends all sessions at the vendor (step 11). The purpose is that any sign-in state still stored in the backup is revoked on the vendor's side and cannot be used to sign in anywhere.

The reasons for the arrangement are isolation (the guest is hostile and rented hosts are untrusted), reproducibility (no network dependence and a deterministic start), and a private store so that nothing proprietary is redistributed.

Use throwaway accounts where the vendor allows it, and treat every image as private (step 10).

## Token revocation and the online token check

The findings below come from one real build by the maintainers; what was not verified is listed at the end of this section.

**Why this matters.** A sealed image that must run offline has to keep its cached logins: remove them and the applications ask for a sign-in that cannot happen offline, and the image has lost its purpose. So the logins stay in the sealed image, and the protection against a leaked image is not to delete the login locally but to make it dead on the vendor's server after the seal (this is the project's design).

**What was found (on a real build).** An in-app sign-out is local only for some vendors. After signing out in a copy, two online checks on fresh overlays of the sealed image still showed Steam, Autodesk (AutoCAD) and Esri (ArcGIS Pro) signed in. The cached login stays valid on the vendor's server, so a sealed image would let anyone who has it, and is online, use the owner's account. Adobe's applications, on the other image, asked to sign in after the sign-out, so that image passed the check. One more image showed a Steam sign-in prompt, and the cause (token removed locally or expired) is not settled.

**What to do.**

1. Seal the image first, as in steps 8 to 10.
2. At each vendor, revoke after the seal: change the password, end all sessions, use the vendor's own device or session list, delete the account where it is a throwaway one. For Steam, use "Deauthorize all other devices" in the account settings (the account page, not the client). The vendor-side means differ per vendor; the list here is what was used, not a complete list.
3. Verify with an **online token check**: boot a throwaway thin overlay of the sealed image, online, with the real clock and with the sealed firewall lifted for the test only, and run an action that needs a round trip to the vendor's server. "Shows signed in" is not proof: the application may be in an offline-mode setting or hold a cached lease. Examples that were used or planned:
   - Steam: the client starts in offline mode in this image (a setting the image applies), so choose Steam > Go Online and read Steam's connection log for an "Access Denied" line. A denied connection means the login is dead; a connection that succeeds means it is still valid.
   - Autodesk: a cloud action such as AutoCAD Share creating a link. If it creates a link, the session is still valid.
   - Esri: an ArcGIS portal action (for example opening an item from the portal).
   - Adobe: whether the application asks to sign in (on the maintainers' real build it did).
4. Do not use the image on a rented machine and do not upload it anywhere until its check has passed. A failed check means the revocation was not effective: revoke again and repeat the check on a new overlay.

The check can be run with the generic tool `scripts/bench/windows/propimage_token_check.py` (`prepare`, `start`, your checklist, `end`; start from `scripts/bench/windows/token_check/checklist_template.md`; firewall lift optional with `--lift-firewall`). The checklist above is the procedure; the tool only automates the overlay, the report and the clean-up, and its guest paths are not covered by the GPU-free tests.

**Named-user licences that need "authorize to work offline".** Some named-user licences must be authorized for offline use while the machine is connected. Do it in install mode with the real clock: allow only that one program through a program-scoped firewall rule, keep the sealed firewall for everything else, authorize, remove the rules, and reseal the image. Do not open the whole network for this. This worked on the maintainers' real build for ArcGIS Pro only; other products are not verified.

**What is not verified.**

- That the revocation steps listed cover every stored login of every application in any image; the repository cannot see vendor-side state.
- Whether a vendor's server accepts a cached login after a password change for some period (a lease); the checks above only show the state at the time of the check.
- A second check later in time, and the behaviour of vendors that were not in this build.
- The cause of the Steam sign-in prompt on the one image mentioned above.
- That the Adobe result holds for an image sealed at a different time or on a different account.

## Using the image afterwards

- **The image is immutable.** The scripts come from the repository over SSH on every boot. Nothing is written back to the image.
- **Always offline, and the clock rule.** (The rule is a check on every start, `scripts/bench/windows/clocknet_assert.py`: a guest with a faked clock must have its network off, and a guest with internet must have the real clock. The image's `state.json` shape is always the first case. To run any image or disk with internet and the real clock, see [Run applications on your own Windows images](run-windows-images.md).) Every boot of the image uses the restricted slirp network (`restrict=on`), never internet. Never boot the base disk itself; boot a throwaway overlay of it. The script refuses to start unless the command line has `restrict=on` and the overlay path.
- **The clock is deterministic and never goes earlier.** A fresh overlay starts at the guest time recorded at the last clean shutdown (the state's last stop time) plus a small gap of 600 to 3599 seconds derived from the seed, so the start time is a function of the state alone. The purpose is reproducibility: boot behaviour, timestamps in results and application and certificate timers are the same on every run. The setup and install sessions use the real clock instead, because TLS to the vendors' sites is expected to need correct dates. Windows keeps its own floor and clamps earlier times (bases set to 2025 came up at the old floor). To restart the same overlay, pass `--resume`: a small sidecar file next to the overlay carries the clock forward across clean stops. The sidecar has not been run on hardware.
- **The hardware identity is fixed per image.** SMBIOS UUID and serials, MAC address, disk serial and GPU UUID all come from the image's seed and are the same on every run, so that test runs are repeatable. The host CPU vendor must match the image's CPU profile (AMD; see the table of needs above). The identity checks matched on the maintainers' boots.
- **Die-binding.** The image is bound to the GPU it was made on in one respect. A different GPU gives the guest a new display device instance with no `EnableGpuFirmware` key, and the driver shows Code 43. The check script sets the key on each new instance every boot. For example, an image made on an RTX 4070 showed Code 43 on an RTX 4090 until the key was set on the new instance; then `nvidia-smi` showed the seed GPU UUID.
- **swtpm 0.10 or newer** is required, as above.

### Known limit: the Windows evaluation timer

The maintainers' Windows 11 evaluation test image (a different image, used for the SPECviewperf runs) shut itself down 62 minutes after boot, and in another run 60 minutes after boot. The shutdown was System event 1074 from `wlms.exe` ("The license period for this installation of Windows has expired"). The check script keeps each boot under about 55 minutes and records such a shutdown as a harness result, not an app failure. Whether your unactivated Windows 11 Pro image shows the same timer has not been verified. Treat about 55 minutes of uptime as the limit for one boot, and restart before it.

## Deviations from a default install

Every setting below differs from what you would get by clicking through Windows, the vendor installers and the NVIDIA installer with defaults. Read this table before you trust a result from the image.

| Deviation | Why | Baked in or per boot |
|---|---|---|
| OpenSSH client and server, service set to automatic, TCP 22 inbound rule, the image's public key in the administrators' key file, Windows PowerShell as the default SSH shell | The test harness reaches the guest over SSH | Baked in |
| Local administrator account with autologon, and the first-run screens hidden (online account, wireless, OEM, privacy) | Offline account creation and a desktop for app checks | Baked in. That autologon survives every restart has not been tested. |
| NVIDIA driver 580.88 installed through the INF route | The guest driver version that kayfabe is tested with | Baked in |
| `EnableGpuFirmware` = 1 in the display class key and in `nvlddmkm` | Without it the guest GPU shows Code 43 | Baked in for the first instance, set again for every new display instance on each boot |
| DWM `OverlayTestMode` = 5 | The Start menu reset the display driver without it | Baked in (a manual image change) (not re-qualified since it was set) |
| Fast startup off (`HiberbootEnabled` 0), hibernation off, sleep, hybrid sleep, monitor and disk timeouts off on AC and DC, wake sign-in off, lock-screen policy off, screensaver off | A suspended or restored Windows meeting a fresh kayfabe, or a locked desktop, breaks unattended checks | Fast startup baked in by the answer file; the rest applied every boot over SSH by `propimage_check.py` (`power_fix`) (not yet run on hardware) |
| The system clock (the deterministic clock) | A fixed starting time, so that boot behaviour, timestamps in results and application and certificate timers are the same on every run | Per boot: the virtual machine's RTC is set from the state file in offline and CI modes; setup and install modes use the real clock |
| US English locale, UTC, fixed computer name, Windows 11 Pro, disk 0 wiped, EFI 300 MiB, MSR 16 MiB, rest NTFS | A repeatable install | Baked in |
| virtio storage driver during setup, plus the network, serial, RNG, panic and firmware-config drivers; no memory-balloon driver | Virtual disk and network; no guest-directed RAM resizing | Baked in |
| `BypassCPUCheck` only; the TPM and Secure Boot checks stay on | The fixed AMD CPU model is older than Windows 11 officially accepts | Setup only |
| BitLocker device encryption prevented | A reproducible test disk | Baked in |
| Windows Update dynamic update off, automatic update off, driver updates through Windows Update excluded | No unexpected traffic or driver replacement | Baked in |
| Virtualization-based security and memory integrity off | No nested security virtualization | Baked in (the running state is not verified) |
| Kernel crash dump settings and visible bugcheck text | Diagnostics | Baked in |
| Smart App Control policy value set to 0 | The harness runs unsigned CUDA probes | Baked in |
| Boot display firmware signing certificate added to the Secure Boot database (OVMF variables) | The boot-display firmware is signed with kayfabe's own key | Baked in the firmware variables file in the state |
| A Windows firewall rule named `kayfabe-block-internet`, blocking outbound Internet | Extra layer, hand-added during cleanup, on top of the network restriction; the offline protection comes from qemu `restrict=on`, not from this rule | Baked in before packing; disabled in the online copy only. Not reproducible from this page: it was added by hand and the repository has no file for it. |
| Run-time scripts and results under `C:\kfapps` and `C:\kfhw`, plus a temporary restart marker | Evidence collection in the disposable overlay | Per boot, in the overlay only |

The answer file may still carry choices that a future, more minimal image drops.

<!-- src: docs/design/V3_PROPIMAGE_APP_AUTOMATION.md (Deviations from a default install); docs/design/V3_RESOLVE_GPU_LANE.md; traces/hwrun_20261113/README.md (swtpm 0.10.1, 4090 Code 43, offline boot); dates of the maintainers' build 2026-11-12/13/14; token findings 2026-11-18 -->

## What has and has not been shown

- The maintainers built the image once, by hand with the repository's helper scripts, and ran parts of the offline check on it.
- Not verified: that someone else can follow this page from an empty disk to a working image, the ISO-making and the empty-disk and firmware-variable preparation (hand work), `setup_ssh.ps1` in a guest, `--resume` on hardware, an end-to-end pass of `propimage_check.py`, that the vendor-side revocation of step 11 covers every sign-in state stored in the backup (the tool cannot see vendor-side state; see *Token revocation and the online token check* for what was measured and what was not), and the behaviour of any storage other than the maintainers' own.
- Nothing here depends on, or is stored in, a public release build.

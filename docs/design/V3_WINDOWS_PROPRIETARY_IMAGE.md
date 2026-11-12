STATUS: DESIGN-ONLY, 2026-11-12. Owner design (binding), gap analysis against draft PRs #293/#302/#300/#303, GPU-free
tooling (`scripts/bench/windows/propimage.py`, tested by `scripts/ci/test_propimage.py`, 158 tests, 59 mutants in
`scripts/ci/propimage_mutants.py`, each with at least two killers; counts from a fresh run on 2026-11-12). No VM was started, nothing was deleted on the PC, no R2 object was written.
Labels: MEASURED = a file or command run on 2026-11-12 (named); CODE READ = read in a named file; INFERRED = reasoning,
not tested; DESIGN INTENT = owner or coordinator decision, not built; UNTESTED = no run yet.

# Reproducible proprietary Windows 11 image on the PC

## 1. The owner's design in one page (DESIGN INTENT, binding)

1. Base OS: official Windows 11 Pro ISO (not LTSC evaluation, not tiny11), edition Pro, no key, unactivated, offline local
   account through an autounattend. The owner copies the ISO to the PC under `/workspace/media/` (MEASURED 2026-11-12: that
   directory does not exist yet); its sha256 is pinned in the runbook and a private R2 copy comes first.
2. One seed string (proposed `kayfabe-prop-v1`) derives every hardware id: SMBIOS UUID and serials, MAC, disk serial, GPU
   UUID (`kf3-gpu,gpu-uuid=GPU-...`), a fixed slirp /24, a loopback SSH port and a clock gap. swtpm state cannot be derived
   (stock swtpm generates random keys), so its files and the OVMF variables are embedded in `state.json`.
3. Modes with the same ids, disk serial, MAC, subnet and swtpm state: `setup` (std VGA, internet on), `install` (kf3 GPU
   and the broker display, internet on), `offline` (slirp `restrict=on`), `ci` (clone, offline, faked clock).
4. Order: install online, then go offline and check every app starts, then stop, back up disk and state BEFORE any logout,
   then start the ORIGINAL disk online, log out and unregister everywhere, discard that disk; test the backup as CI would
   (clone, same ids, faked clock, offline). An offline edit of the backup needs a re-backup.
5. One R2 blob per image: a zip64 with STORED entries (`image.qcow2` is already `qemu-img convert -c`), `state.json`, and
   `manifest.json` (sha256 of each member). No auxiliary files outside the zip.
6. The image stays clean: official installers in official places, OpenSSH server, virtio and NVIDIA drivers. CI sends
   PowerShell over ssh at run time. The first check of every run is one command: machine id, clock, swtpm.

## 2. Gap analysis: #302/#293/#300/#303 against the owner's design

| Owner requirement | What exists (CODE READ) | Gap (this PR) |
|---|---|---|
| Machine id from a seed | #302 `machine_state.py` captures facts and applies them; `win_vm.sh load_identity` makes a random identity once (`secrets.token_hex`) | `propimage.derive_ids(seed)`: pure, golden-tested; uuid, serials, MAC, disk serial, GPU UUID, subnet, port, clock gap |
| GPU UUID | `kf3.c`: property `gpu-uuid`; unset = auto from `-uuid`, random with none (`win_vm.sh` comment, `crates/kf-rm/src/gpuuid.rs`); `machine_state.py` does not carry it | seed derives it and the argv passes `gpu-uuid=` explicitly (tests refuse `auto`) |
| Disk id / other SMBIOS | #302: disk serial and `-uuid`/type-1 serial only | adds baseboard (type 2) and chassis (type 3) serials |
| Fixed slirp subnet | none: `win_vm.sh` uses QEMU's default 10.0.2.0/24 | `net=<seed /24>,ipv6=off` in the argv; validated to `10.x.y.0/24` |
| Loopback SSH port | `win_vm.sh`: `hostfwd=tcp:$SSH_BIND:$SSH_PORT-:22`, default 2224, bind 127.0.0.1 | port derived from the seed; tests require exactly one hostfwd, on 127.0.0.1 |
| Internet on/off switch | #302 requires `restrict=on` (WINVM_NET_RESTRICT); the installer path has the opt-in knob in #293 | mode table: `setup`/`install` `restrict=off`, `offline`/`ci` `restrict=on` |
| swtpm state in the sidecar | #302 captures `tpm/tpm2-00.permall` etc. as base64 blobs bound by sha256; apply restores per logical run | same blob rule in `state.json`; plus `verify-host` (files on disk equal the state) |
| Three-part verify | none: #302 `verify` checks the manifest and image bytes only | `propimage.py verify`: part 1 host files, part 2 guest ids, clock, TPM over ssh |
| Backup/restore order | #293 `image_package.sh` flattens and compresses; no logout ordering | runbook section 5 (backup before logout; discard original) |
| R2 layout | #293 image + manifest, #302 sidecar JSON separate | one zip, `pack`/`unpack`, member and sha256 checks |
| Deterministic clock | #302 `rtc_base_utc` inside a capture window; `win_vm.sh` `-rtc base=utc` | clock rule below: never earlier than the last stop |
| Windows 11 Pro, no key | #293 uses the LTSC eval ISO pin in `win_vm.sh` | new ISO pin and autounattend (runbook section 1; file not committed here) |
| Broker display, input | `display.md`, `windows_broker.sh` (qemu-xhci, usb-tablet, usb-kbd; broker props) | argv reuses those properties unchanged; the hotkey is the broker's own (Ctrl+Alt+G releases the grab) |
| Headless/virtual display on a CI box | broker `--backend test` exists | `--display off` drops the broker properties; the same argv builder serves a later broker-on-CI run (nothing built now) |

Not reused: #302's CPU brand pin, OVMF-signing and monitor EDID checks stay in its tool; `propimage.py` is deliberately
independent of `win_vm.sh` (no source edit of a file other PRs change). Whether `propimage.py argv` replaces or feeds
`win_vm.sh` is a coordinator decision (open question Q1).

## 3. Clock: the observed anomaly and a hypothesis

MEASURED (`/workspace/bench/specvast_20261112/licence_probes/`, gate overlay of an LTSC eval image, offline, kf3 off):

| RTC base given | guest date at ssh-up | file |
|---|---|---|
| real clock (`utc`), host 08:44Z | 2026-11-12 08:44:18Z | `lic_real.txt` |
| 2026-11-12T06:18:26 (3 h back) | 06:21:11Z (honoured) | `lic_rtcm3h.txt` |
| 2026-11-14T00:00:00 (forward) | 2026-11-14 00:02:33Z (honoured) | `lic_d20261114.txt` |
| 2025-01-15T12:00:00 | **04:18:40Z on 2026-11-12** | `lic_d20250115.txt` |
| 2025-06-01T12:00:00 | **04:18:12Z on 2026-11-12** | `lic_d20250601.txt` |

INFERRED from the table: the guest honours any base that is after about 04:18Z on 2026-11-12 (the previous use of this
overlay, per README.txt) and clamps earlier bases to that moment. So the 2025 probes were not "ignored": the guest clock
has a floor at the last time the disk saw. The floor is not the RTC and not the host clock.

HYPOTHESES (none tested), each with its falsifier:
- H1 Windows keeps a persisted monotonic time floor (secure-time / last-shutdown state in the registry or the
  licensing "trusted time" anchor; the licence probes show an anchor of 11:44 on 11/12 that did not move backwards) and raises
  the system time at boot to it. Recall of the exact registry names is unverified, so the tool does not read them.
  Falsifier: boot the same overlay twice with base 2025-01-15 and stop it right after each boot. If the second boot
  shows the first boot's time, the floor follows uptime (H1 holds); if it shows 04:18 again, the floor is a value stored at
  the last clean shutdown. A freshly installed disk that was never booted, given base 2025-01-15, should show 2025-01-15;
  if it shows the real clock instead, H2 or H3 apply.
- H2 Hyper-V enlightenments (`hv-time`, arm B) feed the guest a reference time that is not the RTC. Falsifier: run arm A
  (`-cpu host,-vmx` only, no hv flags; the argv here uses A) with base 2025-01-15 on the same overlay; the clamp is
  unchanged if H2 is wrong.
- H3 the probe never passed the base (the argv was not what the log says). Falsifier: `ps`/`qemu` command line of the
  probe run, or boot the same argv with an Alpine UKI (`scripts/bench/windows/winimage/`) and read its date: Linux
  showing 2025-01-15 clears QEMU and the RTC and puts the cause inside Windows.
Design rule that holds under all three (INFERRED to be safe): `offline`/`ci` boot at `last_time_at_stop_utc + gap_s` and
never before (`boot_clock`; tests `test_fake_clock_is_stop_time_plus_gap_never_before`, `test_fake_clock_is_after_stop_in_argv`).
`last_time_at_stop_utc` is recorded from the guest's own `Get-Date` just before the clean shutdown that is backed up.
Internet modes use the real clock (TLS validation of vendor sites needs it; INFERRED), so the install clock is real and
the faked clock continues from it. The gap is 600..3599 s from the seed, so the CI boot time is a function of the state only.
`-rtc ...,clock=vm` makes the guest clock independent of the host clock after boot (CODE READ: QEMU docs; UNTESTED here).
`verify` accepts a guest date in `[base, base+1800 s]`; a guest earlier than base, which is what the floor would show if
the base were wrong, is reported MISMATCH.

## 4. State JSON and zip

`state.json` (state_version 1, `propimage.init_state`): `seed`, `ids` (smbios uuid/manufacturer/product/serial/baseboard/
chassis, `nics[].mac`, `disks[].serial`, `gpu_uuid`, `net.subnet`, `net.ssh_port`, `clock_gap_s`), `clock.last_time_at_stop_utc`,
`qemu` (machine, smp, ram, fb), `files` (name to `{bytes, sha256, data_base64}`: `tpm/*`, `firmware/OVMF_VARS.fd`; if too
big for JSON they become zip members, change the member allowlist then), `guest` (user, `machine_guid`, `ek_pub_hash`,
SSH key pair: intentionally exposed, local test VM only), `software`, `offline_checks`. `seed_matches` refuses any state whose
ids differ from what the seed derives; `pack` and `unpack` both enforce it. `MachineGuid` and the TPM EK hash are not derivable
(Windows and swtpm generate them): they are captured once from the guest before the backup and compared on every run.

Zip: `image.qcow2`, `state.json`, `manifest.json`; all STORED; exactly these three names; sha256 of each member in the manifest.

Proposed seed `kayfabe-prop-v1` derives (from `propimage.py ids kayfabe-prop-v1`): SMBIOS UUID
`6b3b9dd7-6030-5a6a-8334-3a35c5f22e73`, serial `KFA9113E2E07`, baseboard `KFB2635892EF6`, chassis `KFC3718C38858`,
MAC `52:54:00:67:20:b0`, disk serial `KFD755F9C3D05A4`, GPU UUID `GPU-57a1c1f8-d5e0-2e79-d42f-98f8f35ca554`,
slirp `10.30.170.0/24`, ssh port 33037, clock gap 1968 s. (MEASURED: output of the tool.)
Derivation is `sha256("kayfabe-propimage/v1\0<seed>\0<field>")` per field; changing the domain string or a field name changes every id.

## 5. Launch modes and the first check

`propimage.py argv STATE MODE --disk D --workdir W [--broker-sock S --gop-efi G --uid N | --display off]` prints the qemu
argv, the swtpm argv and the clone command. Differences by mode:

| mode | qemu | display and input | network | clock | disk |
|---|---|---|---|---|---|
| setup | distro `/usr/bin/qemu-system-x86_64` (gtk; MEASURED on the PC: 10.2.1 lists gtk, the kf3 build lists only none) | `-vga std -display gtk` on the PC's GNOME session, xhci tablet and keyboard | restrict=off | host real | the image |
| install | kf3 qemu | `-vga none`, kf3 GPU with `display=on,gop=on,display-broker=...`, xhci tablet and keyboard (as `windows_broker.sh`) | restrict=off | host real | the image |
| offline | kf3 qemu | as install | restrict=on | faked | the image |
| ci | kf3 qemu | broker, or `--display off` | restrict=on | faked | clone (`qemu-img create -b image -F qcow2`) |

CODE READ for the flags: `docs/howto/display.md` (the broker properties, `display-broker-uid`, Ctrl+Alt+G releases the grab,
a broker built-in) and `scripts/bench/windows/windows_broker.sh`. Step 0 of a run is a plain check that keyboard, mouse
and clipboard work; the broker's own behaviour is not re-specified here.

First check after boot, one command (the swtpm and OVMF part runs before QEMU starts):
`propimage.py verify STATE --ssh-key K --tpm-dir W` prints MATCH or MISMATCH for each of: swtpm and OVMF files, smbios uuid,
BIOS/baseboard/chassis serials, MAC, disk serial, GPU UUID (as `nvidia-smi` shows it), MachineGuid, clock window, TPM present,
TPM EK public hash. The PowerShell is sent on ssh stdin (`GUEST_PS`), never stored in the image. Exit code 0 only if all match.
UNTESTED against a real guest: the CIM property names (`Win32_BIOS.SerialNumber` etc.) and what the guest shows for the
virtio-blk serial; the first install run is the test and may change `GUEST_PS`.

Not covered by the tool (shell/ssh glue or later): starting swtpm and QEMU, restoring blobs into the work directory, VNC (the
owner has a physical desktop: no VNC path), Samba share for bulk copy (an insecure share bound to loopback via the slirp hostfwd
is the idea; not designed here), R2 upload (coordinator).

## 5b. Fixed CPU profile (image modes only)

DESIGN INTENT (owner, 2026-11-12): every propimage mode (setup, install, offline, ci) runs the same named CPU model, never
`-cpu host`, so the machine is the same during capture and in CI. This is an exception for image modes only: the owner's earlier
rule "CPU overrides rejected" (PR #302) applies to the default launcher argv, and `win_vm.sh` keeps `host,-vmx,...` untouched
(test `test_win_vm_default_argv_is_untouched`).

- Profile `kayfabe-amd-v1` (`propimage.CPU_PROFILE`): base model `EPYC-v1`, `model-id=AMD Virtual CPU`, family 23, model 1, stepping 2,
  `-avx512f,-rtm,-hle,-mpx,-sgx,-vmx,-svm`, and the Hyper-V enlightenments of arm B (`hv-relaxed, hv-vapic, hv-spinlocks=0x1fff,
  hv-time, hv-vpindex, hv-synic, hv-stimer, hv-crash`). Core count: `-smp N,sockets=1,cores=N,threads=1` with N from the state
  (default 8), never from the host.
- Derived, not guessed: QEMU 10.2.1 (the coordinator's and the PC's distro build) lists EPYC-v1 and its `query-cpu-model-expansion` (type full)
  reports vendor AuthenticAMD, family 23, model 1, stepping 2 and no AVX-512, RTM, HLE, MPX or SGX (MEASURED, 2026-11-12). The model
  carries AVX2, BMI1/2, FMA, F16C, MOVBE, SSE4.2, AES, PCLMUL, RDRAND, POPCNT and XSAVE. The explicit removals are redundant with the
  named model and are kept so the intent is in the argv. `-svm` is a decision (open item O3): the gate does not need nested
  virtualization and the model would otherwise ask KVM for nested SVM; revert it if an app needs it.
- Vendor: the guest vendor is AuthenticAMD because the image is captured on the AMD PC. No vendor spoofing on Intel hosts (the MSR
  and erratum paths in Windows and the drivers are UNTESTED under a spoof). Planner/Worker requirement for later: this image only goes
  to AMD hosts (the offer's cpu_name vendor). OPEN O1: an Intel profile is a separate image; OPEN O2: a vendor-spoof experiment.
- Host preflight: `propimage.py argv` and `verify-host` read `/proc/cpuinfo` (first processor) and refuse with `CPU_PROFILE_UNSUPPORTED`
  (argv exits 3 and prints nothing to start) when the vendor is not AuthenticAMD or a required flag is missing
  (`required_host_flags`, 32 kernel-named flags; the QEMU-to-kernel flag-name mapping is INFERRED). `--no-cpu-preflight` only prints the
  argv on a host that cannot run it. The sanity test checks the required flags exist in the two dumps we have, stored as fixtures in
  `scripts/ci/fixtures/cpuflags/`: the PC (Ryzen 9 7900, Zen 4; MEASURED) and a Vast EPYC 7B12 (Zen 2, MEASURED through a nested
  hypervisor, which hides some flags). This is a sanity check, not a strict intersection gate.
- Fingerprint in `verify` (INFORMATIONAL except two items): `propimage.py record` stores, from the guest, vendor, `Win32_Processor`
  Name, Manufacturer, Family and Description ("AMD64 Family F Model M Stepping S"), ProcessorId, core and logical counts, and a sha256
  over the sorted list of feature flags the guest reports (via `IsProcessorFeaturePresent`: a coarse list, UNTESTED; the PF numbers
  in `GUEST_PS` are from memory of winnt.h and must be checked on first run). Blocking MISMATCH: vendor and family/model/stepping. WARN
  only: brand, ProcessorId, core/logical counts, flags hash, forbidden flags seen. ProcessorId (INFERRED) derives from CPUID leaf 1
  EAX/EDX, so a fixed family/model/stepping fixes its signature half; what can still vary is the feature bits in EDX and anything the
  host adds. UNKNOWN how strictly each app (3DMark, Steam games, DaVinci Resolve, SolidWorks) binds its licence or behaviour to the CPU: no claim.
- The real strictness test is hardware validation, NOT RUN: restore the same image on the PC and on a Vast AMD host of a different Zen
  generation, run `verify`, diff the fingerprints and start every app offline. Only if an app complains do we tighten the profile.
  The flag-intersection rule the owner floated (profile flags only if every supported host has them and no successor dropped them) is
  parked: the model above is Zen 1 by construction, which is the conservative end.

## 6. Open questions for the coordinator

Q1 `propimage.py argv` versus extending `win_vm.sh`; Q2 setup-mode display with the distro QEMU (needs its OVMF paths and
the swtpm AppArmor rule the PC already has for `win_vm.sh`); Q3 whether the guest user `kfimage` and the key live in `state.json`
(per the owner: yes); Q4 DaVinci Resolve and SolidWorks licences may bind to the machine id: the seed makes that stable, which is
the point, but registering twice (CI clones) is a vendor-terms question for the owner; Q5 clock hypothesis test (section 3)
needs one boot slot after the GPU lock frees; O1 Intel profile, O2 vendor-spoof experiment, O3 `-svm` removal (section 5b).

## 7. PC disk plan (MEASURED 2026-11-12 13:30Z, read-only `du`; nothing deleted)

`/` is 1.8T, 1.7T used, 70G free (96 %). Target 300 G free needs about 230 G more. RAM 29 G (one VM). A kayfabe qemu (pid 483969,
kf3 build 0ad35b8d) was running: the SPEC agent's; untouched. Sizes are `du -xh` at depth 1 or 2.

| path | size | class | note |
|---|---|---|---|
| `/var/lib/kf-windows-20261005/win11-full-b` | 188G | needs owner (likely regenerable) | largest single item; name suggests an older full Windows disk set; confirm no run depends on it |
| `/var/lib/kf-windows-20261005/` (353 dirs, `boundary-kayfabe-34xx`, `vfio-*`, `appmatrix`, `codex-*`, `readtrace`, `wreset`, ...) | 447G total | regenerable per run, evidence in git/pulled? unverified | per-run overlays and Oct runs; keep `baseline`, `kf3-bins/<current>`, `owner-state`, `windows-desktop` until the owner says; sweep the rest per directory |
| `/data/claude-spec-20261024/lane` | 101G | evidence-already-pulled? unverified | agent lane overlays from Oct 24; verify the evidence in `traces/` first |
| `/data/codex-enscape-tdr-run-20261028` | 54G | evidence-already-pulled? unverified | Codex TDR runs; ask codex-COORD |
| `/data/codex`, `/data/codex-Q4`, `/data/codex-P1`, `/data/codex-LX1/2` | 40+35+17+14+9.6G | needs owner/codex-COORD | parked Codex lanes; `/data/codex/windows-baseline-compressed` 13G is probably an existing backup: keep |
| `/data/hw6-20261024`, `/data/hp1-20261025`, `/data/claude-hc-2026102*`, `/data/claude-ens12-20261028`, `/data/claude-caps` | 31+30+21+21+14+24+24G | regenerable overlays, evidence-already-pulled? unverified | each lane has a README or trace in git: delete after checking |
| `/data/devbox-archive-20261024`, `/data/devbox-offload` (incl. `paguro-vm` 17G) | 23G, 18G | needs owner | archives |
| `/opt/nvkvm-guest` | 69G | needs owner | Linux guest images; the broker lanes use `/opt/nvkvm-broker`, keep it |
| `/opt/nvkvm-steamos-state-codex`, `/home/ubuntu/steamos-unpriv`, `/root/steamos-nvkvm` | 22+20+9.4G | needs owner | SteamOS experiments |
| `/var/lib/kf-sdr-lut-20261006`, `kf-diesweep`, `vast-windows-4070-session`, `kf4070-resume-20261004`, `kf-vfio-b297-20261005`, `kf-code43-stubs-20261007`, `vast-windows-test*` | 12+12+21+21+21+6.6+12G | regenerable (experiments dated Oct 4-7), needs owner | 105G together |
| `/root/nested-l2`, `/root/tb-test`, `/root/venus-test`, `/root/kf-nsi-nogate-20261008` | 12+8+6.6+7.5G | regenerable (`target/` dirs, test guests) | 34G |
| `/workspace/bench/kf-verify-target`, `src-mc*`, `verify-worktrees/*`, `qemu-10.2.4`, `qemu-build-kf3` | 7.5G+~1.5G+2G+1G+0.3G | regenerable | cargo target and worktrees; keep `kf3-bins` |
| `/home/ubuntu/Downloads` | 15G | needs owner | the owner's downloads |
| `/workspace/bench/specpc_20261112` (69G, winvm 68G), `unigine_20261112`, `luanti_20261112`, `apphard_20261112` | 69+3.1+0.5G | KEEP: the SPEC agent's overlays and evidence | explicitly protected |
| `/root/.cache/pip`, `/root/.debug`, `/root/.rustup` stale toolchains | 3+1+1.6G | regenerable | small |

A first pass of about 230 G that needs no owner call on content: `/var/lib/kf-windows-20261005/win11-full-b` (188G, after one
`qemu-img info` and a grep for references in `scripts/`) plus the regenerable `/root` and `/workspace/bench` build trees (about 45 G).
Second pass (owner): Oct experiment dirs under `/var/lib` and `/data/*` lanes (several hundred GB). Windows needs: 100..150 G
sparse for the disk, two compressed backups (the SPEC backup precedent: `winimage` captured 19.1 GB compressed for a 150 GiB disk,
MEASURED in #300; a Pro disk with the apps will be larger: INFERRED 40..80 GB each), clones are thin overlays until used.
Deletion needs coordinator approval and a per-path check first.

## 8. Runbook for the owner's manual install (all sizes UNVERIFIED unless noted)

Preflight on the PC: GPU lock free and recorded in `/root/prov/PC_ACTIVE.log`, `free -g` at least 12 G, 300 G free, ISO at
`/workspace/media/` with its sha256 recorded here (`sha256sum`). Work directory outside the repo, e.g.
`/workspace/bench/propimage/<seed>/`; keys and the user password stay out of git (`/root/vast-secrets`).

0. Keyboard, mouse click and clipboard sanity line, then Ctrl+Alt+G releases the grab (broker built-in; the launcher adds no hotkey).
1. Windows 11 Pro (setup mode; internet on): autounattend: offline local account (`kfimage`), edition Pro, no key, no Microsoft account,
   no OOBE network requirement, English US. virtio-win 0.1.302 ISO (pinned in `win_vm.sh`) for virtio-blk/net drivers. Offline
   check: Windows boots to the desktop with `restrict=on`; activation stays "not activated" (expected). About 25-30 G used.
2. OpenSSH MSI `OpenSSH-Win64-v10.0.0.0.msi` (pinned, sha256 `ddec9c53...b7d9`, 6.6 MB in `win_vm.sh`): install, service Automatic, key
   login with the state's key. Offline check: `ssh` over the hostfwd works with `restrict=on`.
3. NVIDIA 580.88 desktop driver (pinned, sha256 `90c49f92...3f63a9`, 891 MB): switch to install mode (kf3 GPU + broker display);
   install; reboot. Offline check: `nvidia-smi` shows the seed's GPU UUID; display via the broker.
4. 3DMark: zip in private R2 `kayfabe-bench-enam/3dmark/v2-32-0/installer.zip` (sha256 `33c5b836...ce51e2`), key from
   `/root/vast-secrets/3dmark.key` (typed, never committed). Register while online. Offline check: starts, and a benchmark launches
   with `restrict=on`; if it demands the cloud, record it in `offline_checks` and work around or drop it.
5. Steam, Quake II RTX, Black Myth Wukong Benchmark Tool (throwaway Steam account, created by the owner): sign in online, install,
   then Steam offline mode. Offline check: Steam starts in offline mode and both launch. Sizes: Quake II RTX about 6 G, Wukong tool about
   a few G (UNVERIFIED).
6. DaVinci Resolve free: official installer, no sign-in. Offline check: starts and opens a project without network.
7. SolidWorks Student last (installer URL and serial in `/root/vast-secrets/solidworks.txt`; very large, tens of GB: UNVERIFIED). Licence
   activation is the likely offline problem: activate online, then verify offline start; record the result.
8. Offline test (offline mode, same disk): all apps from steps 4-7 start; write the results into `offline_checks`.
9. Prepare the backup: record `MachineGuid`, the TPM EK public hash and `Get-Date` (`last_time_at_stop_utc`) from the guest; clean
   shutdown; `qemu-img convert -c -O qcow2` the disk; copy swtpm and OVMF files into `state.json`; `pack`. THIS HAPPENS BEFORE ANY LOGOUT.
10. Original disk: start online (install mode), log out and unregister every app, confirm on the vendor websites, delete that disk.
11. CI-shape test: `unpack`, `clone`, `ci` mode, `verify` prints all MATCH, each app starts offline. An offline edit of the backup
    means repeating step 9 on that copy.

## 9. Setup-mode launch, measured 2026-11-12 (first real start)

- Windows ISO: `/srv/win-storage/win11x64.iso`, 8,471,603,200 B, official multi-edition en-US, build 26200 (25H2), Windows 11 Pro =
  image index 6, sha256 `768984706b909479417b2368438909440f2967ff05c6a9195ed2667254e465e3` (MEASURED by the coordinator, pinned here).
  virtio-win 0.1.302 ISO sha256 `303f7ae4...67949d`, OpenSSH MSI and NVIDIA 580.88 sha256 equal the pins in `win_vm.sh` (MEASURED on the PC).
- Disk bus: virtio-blk with serial from the seed, as in CI. No driver click: `propimage_autounattend.xml.in` lists the virtio-win ISO under
  `PnpCustomizationsWinPE` (`viostor\w11\amd64` on D..K) and stages NetKVM and friends offline. The three CDs (Windows ISO, virtio-win, KFUNATTEND)
  sit on separate q35 AHCI ports; only the Windows ISO has a boot index. (The first start used a separate KFUNATTEND CD whose answer file also ran the OpenSSH install at first logon; since the owner's rule of one way to get SSH in, that is gone: one CD, KFTOOLS, below.) UEFI shows "Press any key to boot from CD or DVD" for a few seconds:
  a key press is needed, or `system_reset` and press again (MEASURED: first boot timed out to "No bootable option").
- Setup screen reached (MEASURED, screendump through QMP): Windows 11 Setup reached the Product key page, so the answer file's language
  settings applied; whether the rest of the answer file (edition index 6, partitions, local account, OpenSSH first-logon commands) applies is
  UNTESTED at this point. Fallback: the owner clicks "I don't have a product key", picks Windows 11 Pro, and uses the documented local-account path
  below if OOBE asks for a Microsoft account.
- Windows 11 Setup's supported-CPU check would reject the Zen 1 profile (INFERRED from Microsoft's CPU list); the answer file bypasses only that check
  (`BypassCPUCheck`); TPM and Secure Boot checks stay on (swtpm CRB, OVMF secure-boot code with Microsoft keys `OVMF_VARS_4M.ms.fd`).
- Working directory on the PC: `/workspace/bench/propimage_20261112/` (state.json, image.qcow2 160 G thin, firmware/, tpm/, media/, bin/, keys/,
  run/). The swtpm AppArmor local rule gained one line for this directory. Launcher: `scripts/bench/windows/propimage_run.sh setup` as user ubuntu.


## 10. One install CD (KFTOOLS), SSH, GPU driver (2026-11-12; owner rules: minimal scripts, nothing left in the guest)

Final file list of the CD (`propimage.py tools`, label KFTOOLS): `autounattend.xml`, `OpenSSH-Win64-v10.0.0.0.msi`, `setup_ssh.ps1`,
`580.88-desktop-win10-win11-64bit-international-dch-whql.exe` (both installers sha256-checked against the `win_vm.sh` pins before the CD is built).
No README (the steps are here), no virtio-win copy (its own CD is attached separately). SSH has ONE way in: `setup_ssh.ps1` (FirstLogonCommands removed
from the answer file). The owner's single command, in an admin PowerShell, drive letter of the CD from File Explorer:
`powershell -ExecutionPolicy Bypass -File D:\setup_ssh.ps1`. It checks the MSI hash, installs it (`ADDLOCAL=Client,Server`; feature names UNVERIFIED until the
first run), sets sshd Automatic and starts it, creates the TCP 22 rule only if missing, sets `HKLM:\SOFTWARE\OpenSSH` DefaultShell to Windows PowerShell,
writes `C:\ProgramData\ssh\administrators_authorized_keys` (the VM's PUBLIC key only) with `icacls /inheritance:r /grant Administrators:F SYSTEM:F`,
restarts sshd and prints SSH_READY, the SMBIOS UUID and MachineGuid.

Hot-plug into the running setup VM (MEASURED 2026-11-12 on the PC; the QMP socket is `$D/qmp.sock`; `usb-storage` alone does not work: it needs `drive=`,
and a USB disk would hide the ISO9660 filesystem from Windows, so a USB SCSI bridge with a `scsi-cd` is used):

    {"execute":"blockdev-add","arguments":{"driver":"raw","node-name":"tools0","read-only":true,"file":{"driver":"file","filename":"<D>/media/tools.iso","read-only":true}}}
    {"execute":"device_add","arguments":{"driver":"usb-bot","id":"toolsusb","bus":"xhci.0"}}
    {"execute":"device_add","arguments":{"driver":"scsi-cd","id":"toolscd","bus":"toolsusb.0","drive":"tools0"}}

`query-block` then lists `toolscd` as removable with `tools.iso` inserted (MEASURED). That Windows lists it as a CD drive is UNTESTED until the owner is at the desktop.
Later starts attach tools.iso as the fourth q35 AHCI CD (`propimage_run.sh`), no hot-plug needed.

GPU registry settings, only these three, each from the existing gate material, none for convenience:

| key | value | why | source |
|---|---|---|---|
| `HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\<index>` `EnableGpuFirmware` | 1 | forces GSP (without it: Code 43) | `scripts/bench/windows/gsp_on.ps1:50`, `docs/howto/windows-guest.md:18` |
| `HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm` `EnableGpuFirmware` | 1 | same flag, both are needed | `scripts/bench/windows/gsp_on.ps1:55`, `docs/howto/windows-guest.md:19` |
| `HKLM\SOFTWARE\Microsoft\Windows\Dwm` `OverlayTestMode` | 5 | Start menu reset the display driver on Windows 11 Pro without it | `scripts/bench/windows/desktop_prep.ps1:53`, `docs/howto/windows-guest.md:20` |

The first two are written by the existing `gsp_on.ps1` (device disabled, INF added, keys written, device enabled), run over ssh from git with the host-extracted driver
copied over the same ssh; the third is one `reg add` over ssh. No new script. `gsp_on.ps1` leaves `C:\kf` and its log: the driver directory, `C:\kf` and the
transcript are deleted over ssh when the install is done (nothing of the harness stays in the guest). Whether the host-extracted INF route or the silent `.exe` is better
is open: the `.exe` alone cannot write the class key before nvlddmkm's first start (INFERRED from the gsp_on.ps1 header), so the INF route is used.
The setup-VM answer file keeps: language, disk layout, edition index 6, local account, BypassCPUCheck, BitLocker/Windows Update/VBS/Fast Startup/crash-dump/Smart App
Control registry lines of the existing LTSC template (CODE READ in `propimage_autounattend.xml.in`; each is a tweak from `autounattend.xml.in`, to be trimmed to the minimum
the owner approves: open item O4).

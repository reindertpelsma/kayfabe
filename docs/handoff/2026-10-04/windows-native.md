# Native Windows installer and GSP recorder

**STATUS: RESEARCH, 2026-10-04. Work in progress; no native Windows GPU success or
Windows GSP query capture yet.** The owner requested this lane after the candidate
2 handoff. It does not change the product merge requirements or claim a Windows
guest works through Kayfabe.

## Durable source

- Public standalone installer: <https://github.com/reindertpelsma/vast-windows>,
  `main`, local `/data/vast-windows`. It includes preflight, unattended nested
  Windows preparation, cold-boot/network/SSH checks, the same-disk RAM flasher,
  native NVIDIA/CUDA startup automation and an optional browser desktop helper.
- Recorder: this repository's `codex/windows-native-trace-2026-10-04` branch,
  `tools/windows-gsp-trace/`, local `/data/kayfabe-windows-native-20261004`.
  Source through `da7d9e8a` is pushed. The unsigned driver and
  collector were built locally from pinned official SDK/WDK inputs and copied
  outward to the rental; no rental executables were copied back.
- Linux captured-payload replay and bounded queue/parser tests passed. Passive
  sampling can miss startup traffic; a valid request/reply pair is required
  before treating query fields as evidence. Nothing supplies per-die constants
  to the product.

## Active rental and current stage

Owned instance **54159260**, local SSH alias **`vw-native`**, RTX 3060, about
98 GiB RAM and a 150 GiB whole boot disk. This session owns cleanup. Existing
instance **54049598** belongs to another ongoing lane; do not overwrite or
destroy it. Three failed/pending rentals from this lane were already destroyed
and their absence verified: 54157671, 54158787, 54160009.

The current Linux disk has **not** been armed or overwritten. Installation
completed, both cold boots passed all 25 checks, and an actual controller SSH
login passed with pinned Windows host keys. The recorder bundle was staged and
verified; Windows test signing was enabled and activated by a Windows reboot.
The first driver smoke attempt stopped before execution because the temporary
SSH wrapper omitted PowerShell's `-ExecutionPolicy Bypass`; that wrapper is now
fixed. No driver-load result is claimed yet. Inspect:

```sh
ssh vw-native 'tail -30 /root/vw-resume-3.log'
ssh vw-native 'ls -lh /var/lib/vast-windows/staging.qcow2'
```

Live failures were repaired and pushed: the unattended seed now uses USB optical
media, `icacls` grants and ownership are separate calls, QGA uses delimited sync
with locking/deadlines and no blind command resubmission, and private SLIRP NAT
advertises the DHCP gateway/DNS. Restricted SLIRP had suppressed those options.
The existing firstboot was continued from its exact failure point, with source
hashes and clean shutdown evidence saved. Public installer evidence records the
two later cold boots and the actual authorized-key login.

The 900-second optional hold expired during extra recorder research, so the
harness refused to seal. At 15:52:53 UTC a resume run started from the same image
with `--login-test-hold 3600`: after two cold boots it writes
`/var/lib/vast-windows/login-test.json`, then waits for `login-test-complete`.
The controller must pin those public Windows host keys and perform an actual
authorized-key SSH login through the outer Linux SSH connection. No private
client key goes to the rental. The automatic checks honestly leave
`actual_authorized_key_login=false`; controller evidence is recorded separately.

The native GPU task remains deferred and automatic research reboots are held.
The verified recorder is in `C:\ProgramData\KayfabeGsp`; the bundle SHA256 is
`a93003d85480e5dc6e9b1c16038be2e337531d11ac04643263695900de573726`.
Controller evidence and pinned host keys are under
`/data/vast-windows-runtime/54159260`. QGA large file writes timed out; staging
completed through pinned SSH/SFTP instead, without copying a private key outward.
The
generic public installer keeps ordinary NVIDIA GSP policy and automatic native
GPU installation by default; research deferral and forcing GSP are explicit
options. Consult the latest worker/session state before repeating any staging.

## Borrowed bare-metal fixture and controller storage

The owner made `root@172.22.1.20` (hostname `claude`) available through native
WireGuard. Its OS/disk may be changed, including VFIO and display restarts; no
hardware firmware changes are authorized. It is borrowed and must never hold
the only copy of valuable work. Measured hardware: Ryzen 9 7900, about 32 GiB RAM,
2 TB Samsung SATA SSD, RTX 4070 at `01:00.0` and its audio at `01:00.1`, alone in
IOMMU group 11. An AMD integrated GPU is also present. Host NVIDIA driver is
595.91.07, and `/dev/kvm` is available.

The `windows_prepare` worker owns a direct-QEMU fixture there, preserving Linux
and the host GPU binding: `/root/vast-windows-test`, work directory
`/var/lib/vast-windows-test`, log `/root/vw-test-prepare.log`. Its source matches
public installer `5b368c9`; a fresh 150 GiB image, 8 GiB RAM, eight vCPUs, loopback
SSH port 22222 and research GPU deferral are used. This bypasses the top-level
KVM preflight only as a controlled fixture, not as a claimed supported bare-metal
install. A plain OVMF compatibility failure was fixed by rendering the Windows
setup Secure Boot eligibility bypass only when the requested target state is
already disabled. The fresh installation then passed that check and reached OOBE.
No VFIO operation or physical boot-disk replacement has occurred.

On the controller, `uwgsocks-server.service` was stopped and disabled as explicitly
requested. `wg-quick@wg0` is enabled and active; direct SSH works. The borrowed
peer's conflicting `172.17.0.0/16` route was removed from `wg0.conf`, retaining
its `172.22.1.20/32` route. Other peers remain connected; the old configuration is
backed up privately beside the current file. Do not restore uwgsocks.

The owner's fresh 100 GB volume is mounted at `/mnt/windows-work` with ext4 UUID
`9593cc64-254d-444b-934a-702529ec6587`. Verified idle project/cache directories were
moved there with symlinks at their original paths; the active Kayfabe worktrees
and dirty original workspace were preserved. Root free space increased to about
28 GB. Migration details are in that volume's README and JSONL ledger.

## Next checks

1. Finish the resumed cold checks; run the staged recorder's Windows kernel/API
   smoke with execution-policy bypass. Save exact errors and CodeIntegrity events
   if it fails. The test stops its worker; restart before any valuable capture.
2. Release the final hold, require clean shutdown and exact-size/hash sealing,
   preserve the public host keys and useful text evidence off the rental.
3. Arm only this owned instance, then perform the Linux-to-RAM and RAM-to-native
   Windows reboots. The flasher was rehearsed under KVM/UEFI, including full
   readback and a 512-byte oversized-image prewrite refusal; actual rental GRUB
   and firmware boot remain to be exercised.
4. Reconnect to Windows as `vast`, start the recorder and detached collector,
   then resume NVIDIA/CUDA setup with `-HoldReboots`. Drain and save the trace
   before manually performing a requested reboot. Validate a real
   CUDA kernel result and decode captured query traffic. Absence of a sample is
   not evidence that Windows omitted the query.
5. Use the borrowed RTX 4070 fixture for a second-die VFIO capture and later
   non-nested Kayfabe doorbell measurements, preserving evidence on GitHub.
6. Save useful text evidence to GitHub after each run. Keep the rental only while
   actively used; destroy by its owned ID or explicitly hand off ownership.

The public native package profile currently targets supported Turing-and-newer
GeForce devices with driver 580.88 and CUDA 13.0.0. It does not yet cover all
datacenter GPUs. Broader die/driver validation and any derived Kayfabe fix remain
separate work after the capture mechanism is demonstrated.

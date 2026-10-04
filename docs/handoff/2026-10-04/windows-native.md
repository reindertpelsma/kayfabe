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
  Source through `4663d332` is pushed; the runtime evidence below follows that revision. The unsigned driver and
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

The original Linux disk was armed and rebooted at **17:07:02 UTC**. Provider
serial output confirms the RAM flasher unmounted it and began writing Windows;
final verification/native SSH are still pending. Installation
completed, both cold boots passed all 25 checks, and an actual controller SSH
login passed with pinned Windows host keys. The recorder bundle was staged and
verified; Windows test signing was enabled and activated by a Windows reboot.
The first driver smoke attempt stopped before execution because the temporary
SSH wrapper omitted PowerShell's `-ExecutionPolicy Bypass`; that wrapper is now
fixed. Actual load testing then exposed an absent TrustedPublisher store.
Installer source `0e627c48` fixes that through X509Store.Open(ReadWrite), and
signing/verification now pass. The loader failure is resolved: the combined
`/tsaware:no /section:.retplne,R` full diagnostic loaded, passed the Windows API
suite and unloaded cleanly. An independent MSVC build from source `351d5b7f`
then passed SCM install/load, API tests (zero failures), restart/status and stop.
That MSVC normal observer is the selected capture artifact; its unsigned SHA256
is `c32adf94bde55c19caa493b9ccc460bd39926900acfc4ca93297a6787bb1c3ca`.
Text evidence and build provenance are in `traces/windows_gsp/20261004-loader`.
The service is demand-start and stopped. The final post-recorder Windows
readiness check passed all 25 checks, the controller released the hold, and
Windows shut down cleanly at 16:50:17 UTC. Image sealing completed at 17:01:28 UTC, with check/identical comparison/exit 0.
The 6,242,498,048-byte image SHA256 is
`1fe10630f73d36f01387b51ed73db86833a13f7a4fd867e854d83c6c6580aac7`.
The cutover evidence is in `traces/windows_gsp/20261004-cutover`; full private
controller manifests/keys are in `/data/vast-cutover-54159260-20261004`.
No Windows GSP traffic has been captured. Inspect:

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

A direct-QEMU preparation fixture preserved Linux and initially kept the host
GPU binding: `/root/vast-windows-test`, work directory
`/var/lib/vast-windows-test`, log `/root/vw-test-prepare.log`. This is a controlled
fixture, not a claimed supported top-level bare-metal install. The fresh
installation passed both 26-check cold boots and a real pinned-key SSH login.
It sealed at 16:49:27 UTC with exit 0 and identical source/final comparison:
`windows.qcow2`, 6,940,128,768 bytes, 150 GiB virtual, mode 0400, SHA256
`4bce7fcd14724e4f3cf0c7f425b5800811161e1fd64376eac8e4c917edd07447`.
Controller evidence is `/data/vast-windows-runtime/claude-20261004/`.

The reviewed public installer `537d96e` helper then cloned that sealed image,
stopped the desktop/persistence services, drained the GPU/audio clients and
assigned both RTX 4070 functions through temporary VFIO binding. Its journal
is `/var/lib/vast-windows-4070-session/vfio-state.json`; Windows SSH is forwarded
only on host loopback 22225. The AMD GPU remains with amdgpu. No physical firmware,
boot disk or persistent binding changes were made. Preserve the supervisor and
journal: normal Windows shutdown restores the host's original bindings/services;
recovery refuses to rebind a live VFIO guest.

Pinned-key SSH and the MSVC recorder install/API/restart/stop tests passed with
the GPU assigned. A guest reboot activated test signing. Both NVIDIA PCI functions
then reported PnP OK, but NVIDIA's own driver is not installed yet; this is not
GPU workload success. The `windows_prepare` worker owns the running VFIO guest
and is retrying the collector-before-NVIDIA installation attempt with native
reboots held. The first attempt stopped after extraction and before NVIDIA
installation because Start-Process lost the exit code. Its empty trace exported
and independently decoded correctly. The process helper and both controller
launchers now own the native process handle; actual Windows 5.1 tests cover
0/1/7/3010, 20 fast children, error/timeout refusal and two concurrently drained
128 KiB output pipes. Public installer fix is `a9ecdfa`. It must drain and export before a manual reboot. Root owns
the Vast native-cutover lane; do not race either owner. Borrowed-host evidence,
including the recorder transcript and recovery journal, has been copied to the
controller. No Windows GSP query pair has been captured yet.

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

1. Recorder kernel/API smoke passed. Use the verified MSVC build for capture;
   the revised Linux linker build has static validation but its exact runtime
   parity remains untested. Restart the observer before capture.
2. Sealing, arming and the Linux-to-RAM reboot passed. Watch provider serial
   output for full readback verification/reboot; root polls pinned Windows SSH.
   Do not restart or interfere with the instance during the disk write.
3. Before native GPU setup on the Vast Windows image, stage public installer
   `a9ecdfa` native-gpu.ps1 (SHA256
   `04c09449c358707fcc36775c8f359f144f83f48efe3ac098743ed9eace8233c6`).
   It fixes lost process exit status observed on the first 4070 attempt. The
   sealed image predates this fix but native setup remains deferred.
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

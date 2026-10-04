# Native Windows installer and GSP recorder

**STATUS: RESEARCH, 2026-10-04 17:42 UTC. Native Windows cutover and recorder API
tests passed. NVIDIA 580.88 works on the VFIO RTX 4070, but GSP reports N/A and
no Windows GSP query has been captured.** The owner requested this lane after the candidate
2 handoff. It does not change the product merge requirements or claim a Windows
guest works through Kayfabe.

## Durable source

- Public standalone installer: <https://github.com/reindertpelsma/vast-windows>,
  `main`, local `/data/vast-windows`. It includes preflight, unattended nested
  Windows preparation, cold-boot/network/SSH checks, the same-disk RAM flasher,
  native NVIDIA/CUDA startup automation and an optional browser desktop helper.
- Recorder: this repository's `codex/windows-native-trace-2026-10-04` branch,
  `tools/windows-gsp-trace/`, local `/data/kayfabe-windows-native-20261004`.
  The selected MSVC driver is built from `351d5b7f`; candidate-stat collector
  source is `0797f6ae` (integrated as `4cddb27a`). Artifacts were built by the
  repository's Windows CI from pinned official SDK/WDK inputs and copied
  outward to the rental; no rental executables were copied back.
- Linux captured-payload replay and bounded queue/parser tests passed. Passive
  sampling can miss startup traffic; a valid request/reply pair is required
  before treating query fields as evidence. Nothing supplies per-die constants
  to the product.

## Active rental and current stage

Owned instance **54159260**, RTX 3060, about
98 GiB RAM and a 150 GiB whole boot disk. This session owns cleanup. Existing
instance **54049598** belongs to another ongoing lane; do not overwrite or
destroy it. Three failed/pending rentals from this lane were already destroyed
and their absence verified: 54157671, 54158787, 54160009.

The original Linux disk was armed and rebooted at **17:07:02 UTC**. Provider
serial output confirms the RAM flasher unmounted it, wrote the image and fully
compared the result (`Images are identical`). It rebooted at kernel uptime
654.823 seconds. Windows booted at **17:18:17.500 UTC** and pinned public-key SSH
succeeded at **17:18:41.349 UTC** through the original public endpoint.
The former Linux root is gone; do not use the old `vw-native` root alias or
Linux commands. Controller wrapper `/tmp/vw-native-ssh.py` connects as `vast`
with the previously pinned Windows host key. It reports Windows 11 Enterprise
LTSC Evaluation build 26100 and 105,636,900 KiB visible RAM. NVIDIA PCI functions
10de:2504/228e are present. The Basic Display Adapter's pre-install problem 43
does not describe NVIDIA driver operation. Root started the collector before
resuming the first NVIDIA installation at 17:40 UTC; automatic reboots are held.
Earlier nested installation
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
No Windows GSP traffic has been captured. The selected MSVC recorder also passed
its API/process smoke after native Windows boot. Its collector is now active in
`C:\ProgramData\KayfabeGsp\captures\rtx3060-first-install`; the live run must be
drained before any reboot. `/tmp/vw-native-ssh.py` with the controller's
`capture-status.ps1` reads progress without competing for the exclusive device.

Live failures were repaired and pushed: the unattended seed now uses USB optical
media, `icacls` grants and ownership are separate calls, QGA uses delimited sync
with locking/deadlines and no blind command resubmission, and private SLIRP NAT
advertises the DHCP gateway/DNS. Restricted SLIRP had suppressed those options.
The existing firstboot was continued from its exact failure point, with source
hashes and clean shutdown evidence saved. Public installer evidence records the
two later cold boots and the actual authorized-key login.

The earlier optional login-test hold expired during recorder research; the
harness correctly refused to seal. The resumed preparation then passed another
pair of cold boots and a real controller authorized-key login before its hold
was released. No private client key went to the rental. The harness's automatic
checks leave `actual_authorized_key_login=false`; the actual controller login
evidence is recorded separately.

The native GPU task was deferred through boot and recorder staging, then resumed
with the recorder active; automatic research reboots remain held.
The verified recorder is in `C:\ProgramData\KayfabeGsp`; the bundle SHA256 is
`cfaa75950c2cd7ad226a01060cc91e8db816437e4bd9e178aef612eb198daaf0`.
Its 46 files were verified after native boot (scripts bfa9263b, build 351d5b7f).
Public installer `4ed6a8f` native-gpu.ps1 is staged and verified with SHA256
`beab25e243b596440fdae7d79bd161ad573835dfcbee8eb5abd5a570cef718fc`.
It fixes native process exit handling, verifies an already-started pinned
NVIDIA driver before skipping redundant enable, and requests one held research
reboot after writing the active display-class GSP policy.
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
the GPU assigned; a guest reboot activated test signing. NVIDIA 580.88 is now
installed, nvlddmkm is running, PnP reports Started/problem 0 and nvidia-smi
succeeds. Its full report nevertheless says **GSP Firmware Version: N/A**.
Service and active display-class EnableGpuFirmware values are both 1, but the
class value was written after setup auto-started the GPU. Their presence alone
does not establish firmware activation.

Two captures exported and independently decoded correctly but contain zero
records/tables. The first attempt stopped before NVIDIA setup due to a lost
process exit code. The second installed NVIDIA but redundant pnputil enable
returned 50; public `4ed6a8f` now skips that operation only after live PnP and
pinned-version verification. Actual Windows tests reject wrong driver pins and
unrelated exit50, and cover exits 0/1/7/3010, fast children, timeout and concurrent
stdout/stderr draining. CUDA has not been validated.

The `windows_prepare` worker exclusively owns the running VFIO guest. It is
preparing a controlled guest reboot with the tested observer set to system-start
and a protected SYSTEM AtStartup collector, native installation deferred and
reboots held. Collector `0797f6ae` adds the existing candidate count to stats;
it does not change the kernel driver or ABI. Its trusted MSVC executable SHA256
is `9f1a69942dc52e9e8fded62069b1078a59db06fd2c522e1b1e90596d6fc97bb9`.
Root exclusively owns the native RTX3060 lane; do not race either owner.
Borrowed-host text evidence, including the recorder transcript, empty exports,
NVIDIA mode report and recovery journal, is preserved on the controller under
`/data/vast-windows-runtime/claude-20261004`. No query pair is captured yet.

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
2. Complete the controlled RTX4070 Windows reboot and check actual GSP firmware
   mode before diagnosing queue discovery. Startup ordering is not guaranteed;
   the observer remains passive and never claims a complete capture.
3. On the native RTX3060, the corrected helper and candidate-stat collector
   hashes were verified; first installation and collection are running. Monitor
   the actual task/state before retrying anything. The collector owns the
   observer device exclusively; a competing --status open is expected to fail.
4. Drain and save each trace
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

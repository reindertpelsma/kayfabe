# Native Windows installer and GSP recorder

**STATUS: RESEARCH, 2026-10-04. Native Linux ordinary-user capture and Windows
source-policy/refusal audits complete; no Windows-through-Kayfabe success. Native Windows cutover and recorder API
tests passed. NVIDIA 580.88 works on both GPUs. RTX4070 GSP firmware 580.65.05
produced 4,535 validated records, but no target query pair yet.** The owner requested this lane after the candidate
2 handoff. It does not change the product merge requirements or claim a Windows
guest works through Kayfabe.

## PC returned; native D3D11 reference, 20:55 UTC

**This supersedes the later historical unavailable-PC notes.** The owner
reported the PC back online. SSH recovered after the initial timeout. A fresh
independent clone of the preserved Windows working disk passed disk check and
comparison, then booted through the pinned VFIO helper. Native NVIDIA 580.88 /
GSP 580.65.05 was healthy. The probe from `572411c1` passed four GPU
clear/copy/readbacks and device-health checks. [Evidence and reproduction](../../../tools/windows-gsp-trace/evidence/2026-10-04-rtx4070-d3d11/README.md)
retain 1,074 validated records and zero pool-query records. This is still a
partial passive reference recording, not Windows running through Kayfabe.

The capture drained and the observer was stopped/demand-start. Windows shut
down normally and the supervisor restored Linux NVIDIA/audio/display services
at 20:55:26 UTC, with zero recovery errors. New regeneratable session image:
`/var/lib/kf4070-resume-20261004/staging.qcow2`; original session image remains
unchanged. Resume source is `tools/windows-gsp-trace/tests/resume-native4070.py`
at `219b4ee7`. The controller holds all useful evidence; nothing unique relies
on the borrowed PC staying online. No new rental was needed.

The owner's stub question prompted a [more concrete pool-source review](../../../tools/windows-gsp-trace/evidence/2026-10-04-rtx4070-580.88/command-catalogue/gfx-pool-source.md):
OGKM publishes control-block offsets as well as the exact query/lifecycle APIs,
and Linux firmware-client context promotion includes the pool/control block.
A bounded local-pool experiment need not wait for another capture. Exact
hardware sizing and Windows compatibility remain unproven; no product stub
was added in this research update. Preserve the host-twin policy and real
preemption/completion requirements.

Repository root instructions now live in `AGENTS.md`; `CLAUDE.md` and
`GEMINI.md` are relative symlinks, committed as `bb07d20a` and also applied in
the original `/workspace/kayfabe` checkout without touching its CI edits.

## Ordinary-user proof and scope corrections, 2026-10-04

The owner requested positive confirmation of which observed RPCs can originate
from ordinary-user ioctls. Separate direct user control permission from internal
RPCs caused by an unprivileged allocation or workload. A NON_PRIVILEGED metadata
flag or an earlier root-run trace alone is not an observed nonroot GSP result.

The [new evidence and per-ID table](../../../tools/windows-gsp-trace/evidence/2026-10-04-ga106-unprivileged-575.51.03/README.md)
retains 3,781 native open-575.51.03 GSP records on GA106. Credential/send/ioctl
correlation proves 48 of the 129 Windows IDs can reach GSP through matching
ordinary-user controls, and eight through internal work during ordinary-user
ioctls (55 unique combined). The guard checks UID/GID 65534, no groups, all
capability sets zero, NoNewPrivs=1. Closed 575 also accepts 37 IDs at its direct
user-ioctl boundary, but no closed GSP capture exists. Do not conflate these
boundaries or turn reachability into an unreviewed forwarding allowlist.

Temporary rental **54195016**, label `kf-unpriv-rpc-20261004`, was created for
this test. Text evidence is now saved on the controller and in this branch;
it was destroyed after evidence commit `089559c1` was pushed, and provider
absence was verified at 20:36 UTC ([receipt](../../../tools/windows-gsp-trace/evidence/2026-10-04-ga106-unprivileged-575.51.03/retirement.json)). Other lane 54049598 and
the Windows rental 54159260 remain separate; do not destroy them for this test.

The [scope and existing-policy correction](../../../tools/windows-gsp-trace/evidence/2026-10-04-rtx4070-580.88/command-catalogue/v3-context-policy.md)
applies the owner's ZBC/golden-context ruling: local ZBC already exists and
promotion is satisfied by the real host twin. Eleven direct Windows promotes
use descriptor arrays; four deferred promotes use a legacy GR VA/size form
that the current video-only fallback rejects. Do not implement a golden-image
generator or passthrough UMD method interpreter. Deferred resource/protocol
handling must respect the actual channel route, not infer it from missing
allocation parameters. Source-understood behavior takes priority over gathering
more traces; any compatible OGKM release can supply the implementation contract.

The [refusal-consequence table](../../../tools/windows-gsp-trace/evidence/2026-10-04-rtx4070-580.88/command-catalogue/linux-refusals/README.md)
checks the original 68 Linux-observed IDs: 35 were actually refused with 0x56
in the saved b98bdbec/580.65.06 Linux boot, 15 served, 18 native-only in those
samples. The refused boot reaches SMI_RC=0 and console handoff; its display
probe build failed, so do not call it a complete display pass. The separate
saved CUDA ladder on that build passes. This is not proof of harmlessness for
every feature or Windows. No new failure injection or product change was made.

## Earlier source / OS comparison update, 2026-10-04

The [complete catalogue](../../../tools/windows-gsp-trace/evidence/2026-10-04-rtx4070-580.88/command-catalogue/README.md)
has descriptions/source links for all 129 direct controls, both deferred IDs,
20 allocation classes and five RPCs. Unresolved semantics remain explicitly
unresolved. Whole-tree OGKM searches include 580.65.06, 580.159.04, 595.84 and
610, then Nouveau/Nova and public Envytools/gVisor sources.

- **No Windows-only command is established.** 68 IDs also occur in saved Linux
  driver traffic: 59 native ioctl/GSP and nine additional Linux-to-Kayfabe GSP
  IDs. Seven are confirmed at the native Linux GSP boundary. These are different
  observation planes, not 68 native GSP matches. The closed Linux kernel module
  has no comparable GSP census here. Driver version and die are not controlled.
- The pool-query CPU-stub inference was wrong: flag 0x40 routes to physical/GSP;
  it is not an OS exclusion. A native kernel-caller query probe remains needed.
- Newer OGKM's 0x00730282 structure is 2608 bytes versus Windows 580's 2600;
  Nouveau's historical 0x00730122 meaning uses 16 bytes versus observed 8.
  Neither is a valid direct layout substitution.
- All 472 allocation frames retain headers only; 179 per direction declare
  parameter bytes that are absent. Channel privilege/ownership is not decoded.
  GSP channel lifecycle calls are captured, GPU pushbuffers/doorbells are not.

Callability follow-up: source can prove rejection for an enumerated release,
resource and authorized caller context, but missing callers/CPU bodies do not.
580.65.06 has generic GSS and BinAPI forwarding paths as well as ordinary
exported-method dispatch. The catalogue now links all three and distinguishes
OGKM interface availability from proprietary firmware implementation. No
all-version OGKM rejection or proprietary-CPU-driver-only claim is established.

**Historical, superseded by the PC-return section above:** the borrowed PC
timed out and the owner said it was probably handed over. That offline pass
treated it as unavailable and made no further reconnect/reboot attempts. Unique
capture/code is already saved. That earlier offline comparison started no
rental; the later Linux test above did. No product behavior changed and no
Windows-through-Kayfabe success is claimed. The earlier
runtime state below is historical; do not interpret it as current reachability.

## Command audit, 18:37 UTC

The owner requested checking everything captured against actual command
support, excluding generic passthrough. The
[saved report](../../../tools/windows-gsp-trace/evidence/2026-10-04-rtx4070-580.88/command-audit/README.md)
and `tools/windows-gsp-trace/audit/` reproduce the result against clean
`v3-windows` **c50fad9ac485f53d45d4ea77a21cb7206267c65a**. This recorder branch's
product ancestry lacks that branch's Windows handshake; do not substitute it
for the Windows implementation when assessing coverage.

- All five observed RPC function IDs are known. Of 129 direct control IDs,
  22 have display/channel/init handlers, three have conditional authored
  host-fact queries, two have empirical replies, and **102 have no specific
  handler**. Of those 102, **92 succeed at least once on native hardware**.
- Two of the three host-fact query IDs reject every captured Windows request
  at their existing input gates (0x2080a026 and 0x2080a028); 0x2080a084 matches.
- All 20 allocation classes have names; 17 have decoders. Deferred API class
  0x5080 is absent, 0x90e7 has no decoder, and physical I2C class 0x402c is
  intentionally denied.
- The deferred wrapper 0x50800101 contains four INITIALIZE_CTX and four
  PROMOTE_CTX requests; implementing direct promotion does not implement
  deferred execution. GR_CTXSW_PREEMPTION_BIND and latency-buffer/channel
  properties are further successful native commands without handlers.
- Native errors occur for 12 command IDs. Coverage gaps are not all fatal
  blockers. No GFX_POOL_QUERY_SIZE observation exists, the early prefix is
  still unknown, and no per-die sizing values have been derived.

The audit checks every retained record with the strict decoder. Independent
RPC/control/class aggregation matches; eleven decoder tests pass. No product
code changed and no new borrowed-PC or Vast runtime actions were performed
during this audit. The previously documented reconnect uncertainty remains.

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
resuming the first NVIDIA installation at 17:40 UTC; NVIDIA 580.88 completed at
17:47:53 with zero installer failures and a healthy started GPU. The corrected
helper requested a held reboot after setting the active GSP class key.
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
The selected MSVC recorder passed its API/process smoke after native Windows
boot. Its first pre-reboot capture in
`C:\ProgramData\KayfabeGsp\captures\rtx3060-first-install` drained and exported
at 17:49:53: zero records/tables, exit4, FIFO0. Native setup was deferred again.
Root armed a new system-start/AtStartup capture in `rtx3060-gsp-policy-boot`
using the same tested driver and issued the GSP-policy reboot at 17:51 UTC.
The first empty export is still on the Windows disk; its controller SFTP copy
timed out before connection and remains pending. The controller does retain its
export hash/stats and successful drain transcript. Do not substitute Linux SSH
commands or repeat the reboot blindly.

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

The native GPU task was resumed only with the recorder active. It is now deferred
again after successful driver installation; research reboots remain held.
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
succeeds. Its first full report said **GSP Firmware Version: N/A** because the
active display-class setting was written after setup auto-started the GPU.
After a controlled Windows reboot, the same driver reports **580.65.05** and the
observer attaches to a real GSP table. This verifies the policy timing fix.

Two captures exported and independently decoded correctly but contain zero
records/tables. The first attempt stopped before NVIDIA setup due to a lost
process exit code. The second installed NVIDIA but redundant pnputil enable
returned 50; public `4ed6a8f` now skips that operation only after live PnP and
pinned-version verification. Actual Windows tests reject wrong driver pins and
unrelated exit50, and cover exits 0/1/7/3010, fast children, timeout and concurrent
stdout/stderr draining. CUDA has not been validated.

The controlled boot capture drained successfully at 17:45:47: **4,535 records**,
one table, zero recorder FIFO drops, one observed sequence gap, zero remaining
FIFO bytes. Full text export hash/checksum/framing validation passed. The first
retained request/reply sequences are 2707/2710 with unknown prefixes; the missing
early traffic prevents any claim that the target query was absent. No
0x2080121f observations are in this capture. Complete compressed text data and
provenance are in `tools/windows-gsp-trace/evidence/2026-10-04-rtx4070-580.88/`.

The observer was stopped, restored to demand-start, and its one-shot startup
task disabled after export. The `windows_prepare` worker again exclusively owns
the running VFIO guest for a bounded capture around a trusted NVIDIA-only D3D11
probe being built by the recorder worker. Native installation remains deferred
and reboots held; no new PnP restart is underway. Collector `0797f6ae` adds the existing candidate count to stats;
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

Latest operational note, **17:58 UTC**: the requested RTX3060 reboot was accepted
by shutdown.exe, but at 17:54 its boot time still matched 17:18 and the boot
capture task had not run. Subsequent SSH banner/reconnect probes timed out.
Its observer was stopped/system-start and the fresh boot trace path contained
only its arm journal/task; do not claim a completed GSP boot or blindly repeat
the restart. Check actual boot/task state when SSH returns. The borrowed host's
new direct SSH connections also timed out, despite an active controller WG
interface. No cause is established yet.

The owner reiterated that the borrowed PC is opportunistic and may become
unavailable, while its SSD remains intact. Code, selected trusted build inputs,
the complete first useful trace and completed test evidence are already on the
controller/GitHub. Do not depend on access to its rebuildable Windows images.
The D3D11 probe source is committed (`572411c1`, integrated `7067f79d`) but its
runtime test has not completed. Both helper agents hit an account usage limit;
root now owns any further runtime actions until ownership is explicitly handed
off again. Their last completed 4070 runtime action left the observer stopped,
demand-start restored, boot task disabled, and native setup deferred/held.

1. Recorder kernel/API smoke passed. Use the verified MSVC build for capture;
   the revised Linux linker build has static validation but its exact runtime
   parity remains untested. Restart the observer before capture.
2. Capture explicit NVIDIA graphics initialization on the RTX4070 and look for
   the target query. Its boot trace has an unknown early prefix, so a later
   controlled device initialization may still be needed.
3. Reconnect after the native RTX3060's GSP-policy reboot and inspect actual
   firmware mode/boot capture. Save both exports before further work. Monitor
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

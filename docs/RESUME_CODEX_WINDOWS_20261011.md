# Resume the Windows matrix and GravityMark Vulkan investigation

**STATUS: LIVE, 2026-10-11 21:07 UTC.** Compact entry point for a new chat.
Historical chronology is in [HANDOFF_FOR_CODEX_20261011.md](HANDOFF_FOR_CODEX_20261011.md).
Read the current checkpoint here before treating an older pending/result paragraph
in that chronology as current state.

## Branches and checkout

Public repository: <https://github.com/reindertpelsma/kayfabe>.
Branch: **`codex/windows-matrix-20261011`**. Last confirmed push before this
audit: `249e4e248`; the branch tip includes this later offline H1 checkpoint.
Use the branch tip to get this document and the complete reachable source history.

Controller worktree: **`/data/kf-inval-absent`**. `/workspace/kayfabe` is another
checkout; do not accidentally edit it or Claude's worktrees. Once this session
has ended, reuse the Codex worktree or create an isolated worktree from the branch:

```sh
git -C /workspace/kayfabe fetch origin '+refs/heads/*:refs/remotes/origin/*' --prune
git -C /workspace/kayfabe worktree add -b codex/windows-resume-20261011 \
  /data/kf-windows-resume origin/codex/windows-matrix-20261011
```

Private diagnostic repository: <https://github.com/reindertpelsma/kayfabe-private>.
Branch **`codex/gravity-vulkan-20261011`**, pushed checkpoint **`1dc9c0e`**.
Controller checkout `/tmp/kayfabe-private-codex`; entry point
`codex/gravity-vulkan-20261011/README.md`. It contains debugger scripts, matching
public-symbol/module hashes, bounded stack candidates, native fault records and
the public-schema offline decoder. Access requires the owner's private-repo
authorization, already given for this task. Keep that material private.

Read `AGENTS.md`, `docs/OWNER_RULINGS.md` (especially §§AD–AH),
`docs/design/V3_CONSTANT_STRATEGY.md`, `docs/design/V3_PROVENANCE.md`, and
`docs/design/V3_FEATURE_GAPS.md`. The owner just reaffirmed that the fix must
follow v3. Host actions must be authored and unprivileged; preserve ownership,
lifetime, actual GPU ordering/completion and the nonblocking vCPU contract.
Private diagnosis supplies no implementation interfaces/constants.

## Live hardware state: protect the owner's session

Trusted host: **`root@172.22.1.20`**, one RTX4070/AD104. Host driver595.91.07,
Windows driver580.88. `W=/var/lib/kf-windows-20261005`.

**Manual Windows run1380 is READY**, since20:17:45UTC, exact product3252a087.
Window title: `kayfabe Windows manual 1380 — no automation` on ubuntu's desktop.
At readiness NVIDIA Code0, autologon disabled, no input or apps automated.
Owner was notified and given credentials. Login `vast`; password only in
`$W/windows-desktop/secrets/win_password`, never in Git. No expiration, restart,
auto-click, sign-in or app loop is scheduled for this VM.

The original parent PID209237 holds `flock -o /tmp/kayfabe-fastguest.lock` for
the entire manual VM. QEMU PID398484 was live at20:23UTC; recheck processes and
the campaign log because PIDs/state are a dated observation.
Do not launch another GPU job, another QGA client or stop/reboot this VM merely
to resume work. The owner's stop marker, only if requested, is
`/tmp/kf-stop-codex-manual-1380`. Campaign log:
`$W/codex-prod-defaults-campaign.log`.

The allocator job `/tmp/kf-capacity-after-manual.sh` is already waiting for this
parent's actual exit and both manual READY/EXIT markers. It then acquires the
same lock, validates65530e6a on Linux and runs Windows1480+ controls. Do not
duplicate it. Log: `$W/codex-capacity-after-manual.log`.

Kayfabe and VFIO cannot run simultaneously. After any authorized stop, require
no QEMU, IOMMU group11 `DMA-FQ`, and remove `/tmp/kf-stop-winprod`.
Keep `/var/lib` above100GiB free (156GiB observed20:23UTC). No host compilation
during guest timing. No Vast instances remain rented by this session.
Vast is untrusted: no secrets; any future rental requires `vms_enabled=true`
and a KVM template, with evidence backed up before teardown.

## What passes, and what remains open

The [combined checkpoint](../traces/windows_matrix_20261011/candidate_combined_checkpoint.tsv)
has104 executed application rows: **79 cleanPASS, 8 PASS_WITH_RECOVERY,
11 FAIL, 6 TIMEOUT**. Applicable Linux counterpart parity is **59/65** in
[the parity table](../traces/windows_matrix_20261011/linux_parity_checkpoint_current.tsv).
These combine explicitly recorded product/harness revisions; they are not a
single-revision full-green claim. Rebuild with
`traces/windows_matrix_20261011/rebuild_candidate_checkpoint.py`.

OpenGL gears passes. Exact product**3252a087**, which makes native PTIMER
mapping the default without KF3_TIMER_MAP, passes Linux gates9/9+gate10 and
fast30/30. Both Windows controls1390 and1460 pass gears/vectorAdd/CUP8 with
zero recorded guest recovery/GSP cycles and precise-window hostXid0.
[Production/default evidence](../traces/windows_matrix_20261011/prod-defaults/README.md).

Manual1380 still uses19 remaining behavior flags. There is **no zero-flag stock
Windows claim**. Removing KF3_DISPLAY_CAPS_PROBE (the captured PDISP MMIO caps
page) causes run1400 setup BOOT_FAIL/Code43/no NVIDIA adapter. Run1401 was
explicitly interrupted after the first falsifier; no app verdict is claimed
for it, and its unclean stop is recorded. This flag is distinct from system
GET_CAPS_V2 feature bits. Derive the caps from public sources/our implementation;
do not promote a captured table into a default merely to boot Windows.

Product**65530e6a** adds actual-need allocator fallback (421 GPU-free tests
pass,3 ignored); hardware validation is pending the owner's manual-session exit.
Most matrix rows used**72741e2b** or explicitly named earlier revisions.
Current branch HEAD includes later source/evidence commits; do not attribute
the frozen3252 hardware results to arbitrary HEAD changes.

## GravityMark Vulkan: measured facts and the next decision

**GravityMark1.89 Vulkan is still unfixed on kayfabe.** Same RTX4070 under VFIO,
same Windows580.88 and normal600s workload/oracle, **PASS183s** with output image,
timing data and measured NVIDIA activity; no guest153/4101/WER/reboot.
Reference has physical12GiB VRAM versus kayfabe's declared4GiB; measured native
peak2247MiB. GSP recovery count is unmeasured, not assumed zero.
[Reference and full observer capture](../traces/windows_matrix_20261011/gravity-vulkan/vfio-reference/README.md).
Earlier Vast4090 TIMEOUT is a separate baseline, superseded for same-GPU comparison.

Kayfabe1361 times out603s, hostXid32/channel4e, three guest153 and one GSPcycle.
A screenshot shows asteroid rendering before failure. Guest153 alone is not
proof of a Windows TDR; distinguish display4101/live-kernel141.
The faulting twin has a real5080 object with no authored host registrations;
the guest registers FIFO_UPDATE_CHANNEL_INFO. VFIO records two successful
50800104 INTERNAL registrations for inner20801116. **The exact GPU triggering
method and common initiating cause have not been measured.**

Offline H1 audit21:07UTC: fault channel4e is **Passthrough/user-work**, not
Translated. It is scheduled and forwards over4096 submissions before the one
FIFO-update registration. The two Translated registering channels have no FIFO
updates. VFIO also schedules before its first update registration (9.213s).
Registration-time pre-token/pre-schedule conversion does not fit this sequence.
Actual faulting0x200 remains UNMEASURED, so H1 is **inconclusive**, not falsified
by registration alone. Do not implement a Translated-only migration as this
channel's fix. Next capture needs a bounded peek armed after registration;
earlier512-entry initialization peeks cannot cover this late window. Public
source says clear the PBDMA interrupt *before* executing FIFO update, correcting
the proposed plan's opposite wording. [Reproducible channel audit](../traces/windows_matrix_20261011/gravity-vulkan/h1-channel-audit/README.md).
Owner is now trying1380 and will say when finished; leave it untouched meanwhile.

FIFO_UPDATE_CHANNEL_INFO is a deferred FIFO/USERD metadata update, distinct
from batching mapping/unmapping with a final invalidate. Keeping an authored
host API registration and its stream trigger is a candidate, not an implemented
fix. GPGA containment alone is insufficient: complete spans, alignment,
ownership, mapping/backing lifetime, handle namespace and relay transition
must be correct. Current Windows USERD is guest RAM relayed to a private host
USERD, so guest relocation also changes the relay endpoint/count/ring metadata.

The native USER/uid65534 test registers successfully and writes a GPU tail,
but update and update-hold each cause Xid32; NOP is clean. The hold fault occurs
300us after HOLD_BEGIN, while all resources remain alive for2s. **Early cleanup
is falsified for this native failure.** Public source retains registration
privilege for the inner control, which remains privileged in595.91.07;
permission failure is a hypothesis, not a decoded inner status. No product
caller or privilege escalation was added.
[Native probe](../traces/windows_matrix_20261011/gravity-vulkan/native-deferred-update/README.md)
and [hold/RC evidence](../traces/windows_matrix_20261011/gravity-vulkan/rc-journal/README.md).

Root read-only diagnostic journal queries succeed, counts0/1/2 and97-byte
fault records. Raw records/decoder are private. The exact public schema leaves
returned nested fields opaque; no method, fault VA or inner error is claimed.
Private1362 breakpoint capture reaches a later Windows hardware-hang path,
after hostXid. Guest CPUs paused about33s for a dump; GPU was not frozen.
Stack words are candidates, not a validated unwind or initiating-cause proof.

Existing PT stall snapshots were inconclusive: exhausted global/free capture
budget and no decoded segments for the needed fault window. Zero mapping
refusal/HELD counters do not prove PT/PD/TLB correctness. Boot-wide invalidate
maxima around76ms breach the12ms requirement but are not fault-time correlation.
Before another run, state a falsifier, change one variable, and measure the
actual software trigger or a specific mapping/completion failure. Further
hardware work must wait for the protected owner session to end.

## Backup and restart artifacts

All current source and reviewed evidence are on the public branch; private
diagnosis is on its separate branch. The
[resume archive](../traces/windows_matrix_20261011/resume-20261011/artifact-manifest.json)
backs up the exact campaign/manual/native runner scripts and stopped-run
1400/1401/1460 logs/launch metadata, with original and artifact SHA256 hashes.
It is an archival snapshot, not an instruction to replay completed runs or
overwrite scripts currently in use. Older scripts with hardcoded PID/path/run
guards must be adapted deliberately for a future fresh run.

Credentials, password-input traces, full guest disks and the8GiB RAM dump are
excluded. RAM is regeneratable, trusted-host only; its private scripts and
bounded findings are backed up. Original VFIO benchmark output image/timing
files remain in the retained host overlay; their recorded file facts, lane
oracle and observer trace are published. No claim that a VM/disk image is
published or that every raw capture is public.

Continue committing/pushing at least every30min. Keep master untouched without
its review and exact-revision hardware merge bar. End each work block by
updating this resume point and the main handoff with measured results and open work.

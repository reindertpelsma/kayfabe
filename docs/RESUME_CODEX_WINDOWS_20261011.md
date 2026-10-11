# Resume the Windows matrix and GravityMark Vulkan investigation

**STATUS: LIVE, 2026-10-11 21:43 UTC.** Compact entry point for a new chat.
Historical chronology is in [HANDOFF_FOR_CODEX_20261011.md](HANDOFF_FOR_CODEX_20261011.md).
Read the current checkpoint here before treating an older pending/result paragraph
in that chronology as current state.

## Branches and checkout

Public repository: <https://github.com/reindertpelsma/kayfabe>.
Branch: **`codex/windows-matrix-20261011`**. Last confirmed push **89e8ddf8b**,
reviewed diagnostic source with161 GPU-free tests. Newer documentation checkpoints
must not inherit exact-revision hardware claims automatically.
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

## Live hardware state: owner released the GPU; personal image saved

Trusted host: **`root@172.22.1.20`**, one RTX4070/AD104. Host595.91.07,
Windows580.88. `W=/var/lib/kf-windows-20261005`.

Owner finished manual1380 and explicitly authorized stop and all further runs.
VM stopped21:33:57UTC **UNCLEAN-kill** after ACPI/QGA failed to finish;
parent209237 exited21:34UTC. HostXid total remained30. Before new tests:
no QEMU, group11DMA-FQ, stop-winprod removed. Owner reports GPU/Minecraft freeze;
see [manual evidence](../traces/windows_matrix_20261011/manual-1380/README.md).
No matching Minecraft Windows reference/edition/version measured.

**Keep the convenience image:** `$W/boundary-kayfabe-1380/windows.qcow2` and
its `$W/baseline/windows.qcow2` parent. Original1380 is read-only. Separate
writable personal child **`$W/owner-state/manual1380/resume.qcow2`**, copied
OVMF_VARS.fd and KEEP.json are ready for future personal use. qemu-img check0;
this does not prove NTFS clean shutdown. Do not test on/delete the personal
chain. User says all contents are regeneratable/redownloadable/re-signin.
Disks/credentials stay solely on the trusted host, not Git/Vast.

Old queued allocator65530 campaign was cancelled **while waiting**, not run;
its exit0 trap is not a validation result. Validation is combined into exact
**89e8ddf8** campaign, including allocator source and default-off diagnostic.
Worktree `/data/kf-late-fifo-host-89e8ddf8`, detached exact89e8ddf8b.
Run root `$W/codex-late-fifo-20261011-89e8ddf8`; shared GPUlock held by resumed
campaign PID492472 (dated21:38UTC; recheck). Log `$W/codex-late-fifo-campaign.log`.
**Do not duplicate it or edit its running scripts.**

Linux gates9/9+gate10 PASS21:34UTC, binary/client/fastguest built. Initial
fast suite never started because runner omitted BENCH_DIR mkdir; campaign
exit1 preserved, no product failure inferred. New `resume_campaign.sh` fixes
only runner preconditions, preserves fast-notrun.log and starts fast30 at21:38; completed30/30 PASS21:41:47UTC.
Fast suite30/30 PASS21:41:47UTC, zero FAIL/CRASH/NOTRUN. Automatic fresh Windows1490 smoke (gears/vectorAdd/CUP8/torch),1495
Gravity control,1510 Gravity probe. Only probe changes measurement value
KF3_RELAY_PB_PEEK=after-fifo-update. Original600s benchmark/oracle retained.
[Exact archived runners](../traces/windows_matrix_20261011/gravity-vulkan/h1-channel-audit/).

Kayfabe/VFIO cannot run simultaneously. After stop require noQEMU, group11
DMA-FQ, remove /tmp/kf-stop-winprod; keep /var/lib above100GiB (131GiB at21:38).
No host compilation during guest timing. No Vast instance remains rented by
this session. Vast is untrusted: no secrets; rentals need vms_enabled=true/KVM.

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
pass,3 ignored); hardware validation is part of the active exact89e8ddf8 campaign.
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
Owner has released1380; it is stopped and the installed personal chain retained.

Linux hardware validated on exact89e8ddf8 (Windows probe pending): `KF3_RELAY_PB_PEEK=after-fifo-update` arms the
existing bounded sample after the first update registration on a relay's
associated5080 allocation lifetime. Default remains off; `=1` starts at birth.
No migration or host registration is added. Three failing-first diagnostic
packet tests fix missed multi-datum triggers; final kf-qemu lib tests161/161.
Independent review complete, no blockers; binding/first-object/Legacy/arming
and post-forward race limits explicitly remain. A sample is not GPU execution
proof and cannot alone formally falsify H1. Hardware gates9+10/fast30 still
completed for exact89e8ddf8: gates9/9+gate10 and fast30/30 PASS; fresh matched control/probe
runs in the active combined campaign. See audit README.

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
actual software trigger or a specific mapping/completion failure. Hardware work now runs under the released shared lock in the combined campaign.

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

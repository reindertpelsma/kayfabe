# Windows investigation stop/resume record

**STATUS: PAUSED, 2026-10-05 14:37 UTC, owner requested saving work at 1% weekly allowance.**
Do not start new work until the owner resumes it. No working Windows-on-Kayfabe claim.

## Saved code and evidence

- Main investigation: `codex/windows-boundary-comparison-20261005`, trusted worktree
  `/data/kayfabe-boundary-20261005`. Public comparison, recovery tools and handoff
  are on GitHub. Original dirty `/workspace/kayfabe` checkout remains untouched.
- L baseline product `b431aeaf`; M ILUT probe `d2c7ca1b` on
  `codex/windows-ilut-constructor-probe-20261005`; N product `9312854d` on
  `codex/windows-tmo-surface-constructor-probe-20261005`. Both probes are default off,
  source-derived, and force blanket display-method refusal. Nothing promoted to master.
- Observer/artifact `a8845e69`; reviewed native comparison `6b13590f` on
  `codex/windows-native-boundary-20261005`; source dataflow `d36f41ec`.
- Raw journal evidence/tooling is on `codex/vfio-gsp-observer-20261005`, worktree
  `/data/kayfabe-vfio-gsp-observer-20261005`. M evidence `f7ad0585`, labelled
  comparator `507da434`; final stop commits may add N evidence and privacy guards.
  Read its latest Git log rather than treating these earlier hashes as its head.
- Production LUT source audit is separate on `codex/windows-lut-production-audit-20261005`,
  worktree `/tmp/kayfabe-lut-production-audit-20261005`. This is a proposal, no implementation.

## Latest established result

Native VFIO8/9/10 are healthy. Kayfabe5/6/7 use the same immutable QEMU binary,
Windows580.88, baseline and firmware, and all fail with Code43 / nvidia-smi9 and
identical normalized365-RPC order. Native coverage remains partial; command-ID
matches do not establish equivalent parameters, objects, topology or causality.

M, boundary-kayfabe-8, passes the first ILUT constructor check and fails at the
second TMO constructor. Of24 saved assertions only ordinal17 changes:
`16e97f4` → `16e97c6`. KD failed with pipe1450 messages; its output was not a
completed analysis. A bounded offline reader recovered the raw journal, matched
L against its independent successful KD journal, and validated module identity
against the pinned driver. Outer NVCD is one byte short; that integrity limit remains.

N, boundary-kayfabe-9, adds only generated TMO_SFCLOAD relative to M.96 unit tests,
QEMU compile check and independent review passed before the PC build. It still
fails with365RPCs/Code43, but raw-journal analysis advances the sole assertion to
`16e9967`. This follows constructor call `16e98d9` in a later four-entry loop;
Windows passed the earlier window ILUT/TMO loop and subsequent CPU allocation.
No debugger ran inside N; the fresh dump was recovered after orderly guest
shutdown via read-only NBD/NTFS. Supervisor exit0, host GPU healthy, cleanup verified.

The next static lead is the per-head output LUT descriptor from public
POSTCOMP_HEAD_HDR_CAPB at `0x684+head*32`. **Corrected:** input+4 is an entry/count
field, not buffer count. OLUT buffer count input+0x18 is1, so the absent-SFCLOAD
buffer-count guard passes. The candidate mismatch is populated pointer slots
versus the zero-pointer requirement with SFCLOAD absent, analogous to TMO.
No O implementation or boot was authorized/executed before this pause. Recheck
this source note and exact caller path before creating any further probe.

## Private evidence and hardware state

All unique runtime evidence is on the controller under
`/data/kayfabe-runtime/windows-boundary-20261005/`, not solely on the borrowed PC.
M original/copy dump SHA256:
`d1fd05107c458a985cd0dc1f3e341aceded78a1fccca3b0360d2e058ad14928e`.
N `boundary-kayfabe-9/watchdog-offline/watchdog.dmp` SHA256:
`4daf4bebe29c6fcf657e0b62cb8816b101fdfabded5ab75f6e3136fee13c7e23`.
Both343710bytes. Original dumps/journals/driver bytes stay private.

N binary hash `7d6b9266182581ec95ad5dae1de0b46c7d07d80d3744d982982727e1f90ee975`;
its build receipt and script are in the same controller runtime root. PC immutable
binaries are under `/var/lib/kf-windows-20261005/kf3-bins/`; the shared build now
contains N, so always choose an immutable revision directory. On pause, no QEMU
VM runs, NBD0 is detached, and RTX4070 host driver595.91.07 is healthy. The PC's
baseline/builds are regeneratable; recovered evidence is duplicated locally.

## V3 and remaining product work

These constructor probes deliberately declare operational capabilities while
refusing their methods. They are not production fixes. The observer also traps
BAR0 reads and copies queues synchronously for diagnosis; that is explicitly
outside v3 product trap rules. No captured per-die tables, privileged forwarding,
passthrough UMD parsing or forged GPU completion were added. Hardware evidence
is one Windows580.88/AD104 fixture, not driver/family-wide qualification.

The operational audit finds real default ILUT use in public Linux NVKMS too.
Current composition lacks typed ILUT/TMO/CSC processing. Honest production
support needs generated method decoding, bounded owned LUT storage/lifetime,
actual GPU transformations and real completion. Silent method acceptance or a
CPU pixel-transform fallback is not an acceptable promotion of these probes.

## Vast/reinstall side task

The successful baseline is reinstall-based rental54307259, not the old nested
vast-windows workflow. It remained reachable; its actual bootstrap.ps1 exactly
matches the preserved repaired wrapper, and native-gpu.ps1/cuda-smoke.cs match
trusted controller sources. Saved task/log/state evidence reports first-run
success, NVIDIA580.88, CUDA13.0.48 and an actual CUDA smoke result42 on RTX5090.
The rewritten standalone bootstrap is the unvalidated delta; no exact cause of
the failed rental54316622 has been established.

Private evidence: `/data/vast-windows-runtime/54307259/` including
`preserved-successful-baseline-20261005/` and the bounded read-only capture JSON.
Docs/audit branch: `reindertpelsma/windows-on-vast`,
`investigate/bootstrap-20261005`; scripts/main remain unchanged.
54316622 was retired earlier.54307259 is being retired after preserving the
working baseline and recorded CUDA result, under the owner's idle-rental cleanup
instruction; consult retirement-20261005.json for the verified account inventory.

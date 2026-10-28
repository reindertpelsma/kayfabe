# Proposed Enscape 06 TdrDelay intervention

**STATUS: PREREGISTERED ONLY, 2026-10-28. NOT RUN; execution awaits the owner's go.**
This is a diagnostic threshold intervention, not a fix or a recommended deployment
setting. [plan.json](plan.json) freezes the proposed arms, order, limits and flags.
No registry, VM, image, host driver or display configuration was changed in preparing it.

**Question and falsifier, fixed before running.** The owner's requested prediction
is: if a long paging-channel MEM_OP split causes the 153, raising TdrDelay lets
Enscape 06 finish; if 153/hang persists, walk length is not the cause. Test the
narrow version: a finite split/preemption delay exceeding the default two-second
threshold, but shorter than the proposed ten-second threshold, is sufficient to
explain the selected default-arm failure. Its falsifier is a raised-arm 153/hang
despite verified registry settings and complete timing evidence that every
relevant request, including unreleased requests, stayed below the raised threshold.
That rejects this **simple threshold explanation** for that occurrence. Persistence
with a request exceeding ten seconds, lost request history, or an unverified
scheduler dependency does not rule out every walk-related mechanism. Completion
under the raised setting is consistent with the prediction, not unique causal proof.

The repeated/default control must reproduce the relevant failure and long-request
exposure. If defaults complete or the long split is not reproduced in isolation,
the intervention is inconclusive about the ENS12 failure; do not pool in old
failures to manufacture a matched comparison. Report every scheduled arm, not
only successful starts. No failed, hung or infrastructure-refused arm is replaced.

**Prior evidence is not discarded.** REPORTED by the coordinator's 2026-10-28
read-only ENS12 analysis: in 7418 a 2665 ms token-0x80c split was requested about
2.03 s before TdrPayloadEngineTimeout and released about 0.64 s afterwards; the
all-node completion gap began within about 6 ms of its request. REPORTED for 7417:
no preceding long split was observed, and its 3.49 s all-node gap began at the TDR,
consistent with recovery rather than its initiating cause. These are relayed
timing findings, not an independently calibrated millisecond causal measurement
by this preparation. Independently re-read QEMU timestamps establish 7418's
2665.007 ms request-to-resume interval, but do not establish cross-clock offsets.
7417 already defeats a universal assertion that every 153 follows a long split.
Absence in retained history does not prove that an unreleased request was absent.

CODE READ: archived `h1c.py` requires both apply-end and release timestamps and
selects by release time. It excludes requests never released; its rows are not an
all-request census. The earlier `h1.py` additionally optimizes an offset against
completion proximity; that optimized alignment cannot independently prove the
same proximity. Neither method is adopted as the primary causal oracle here.

**Intervention.** Arm A removes the `TdrDelay` value and leaves `TdrDdiDelay` absent.
Arm B sets only `TdrDelay` to REG_DWORD decimal 10 and leaves `TdrDdiDelay` absent.
Do not change TdrLevel, TdrDdiDelay, TdrLimitCount, TdrLimitTime or other watchdogs.
Microsoft defines TdrDelay as the scheduler's preemption timeout, default two
seconds; TdrDdiDelay is a separate driver-exit timeout, default five seconds.
Its documentation restricts these keys to targeted driver testing/debugging.
Ten seconds is a proposed finite intervention, not a Microsoft universal
recommendation. SPEC's guide independently suggests a TdrDelay of at least ten
for some TDR-sensitive workloads; detection remains enabled in both arms.
[Microsoft TDR keys](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/tdr-registry-keys),
[SPECviewperf 15 user guide](https://gwpg.spec.org/wp-content/uploads/2025/12/SPECviewperf15-User-Guide.pdf).

**Fixed runtime/configuration.** Use ENS12 A runtime
`bab3c0882b49c76f8a71d47fc672ba6330bc1313`, binary SHA256
`8636abb126a0cd7cdd706737b6d79f0abb0165688901a916e336df055267789c`,
host driver 595.91.07, installed Windows driver 580.88/32.0.15.8088, RTX 4070,
12 GiB guest RAM, eight vCPUs, 8 GiB guest VRAM and FullHD. The launch property's
580.65.06 is nominal emulation configuration, not the installed driver identity.
`plan.json` copies all 23 recorded A7418 environment flags; sparse twin and latency
diagnostics remain on, IRQ storm remains off. Preserve CPU/disk/cache/network,
display, driver options and observer configuration. Only TdrDelay differs between
arms. The intentionally isolated workload differs from ENS12's earlier full-suite
history and is not silently treated as the same exposure.

Use a fresh owned CoW plus fresh owned UEFI variables for **every** arm, all from
one frozen parent. Seal the parent content identity and backing-chain metadata
before execution and after cleanup, without altering the parent; archived ENS12
size/mtime alone are not a content identity. Keep raw local paths and machine
identifiers private. Refuse a mismatching binary, installed driver, image chain,
flag set, workload definition or target selector. Changes to this preregistration
require a new dated amendment before any affected run, not a post-failure adjustment.

**Reboot/readback gate.** In each fresh arm, record the entire relevant registry
baseline, make the one assigned change inside the owned CoW, reboot both arms,
prove a new boot identity and re-read the actual keys before the workload. Arm A
must show both values absent; B must show TdrDelay=10 and TdrDdiDelay absent.
Do not reuse ENS12's no-reboot deviation. A reboot hang is an infrastructure
outcome, not a benchmark pass; repeated infrastructure wall twice stops the
campaign. Preserve the refusal and do not skip the reboot or reuse that overlay.

**Target and invocation gate.** Target only
`enscape-01/06_SanMiguelFullUltraHQ`, using the archived vendor invocation
`run_vset.bat enscape 6` recorded in [reference.json](reference.json). Launch in
the standard interactive user session, from the installed SPECviewperf directory,
under its verified FullHD configuration. First verify the current script's digest,
working-directory/configuration prerequisites and selector semantics from permitted
text metadata. Do not reverse engineer executables or copy models. Do not run
the full Enscape workload as hidden preparation. If required setup cannot be
established without another workload, stop and amend before execution. An output
showing another subtest or missing the target name is invalid selection evidence,
not a replacement sample. Preserve normal setup/calibration, not just a timed loop.

**Replication and bounds.** Five planned independent boots per arm, fixed balanced
order `A1 B1 B2 A2 A3 B3 B4 A4 A5 B5`. One subtest execution per boot; no best-of,
rerun, selective extension or replacement of failures. Workload bound 1800 s;
whole-arm bound 2700 s including reboot, collection and restoration; ten-arm
absolute maximum 27000 s. Reserve the complete host window before starting;
unfinished arms remain unfinished. Stop at a new host Xid, boot/image identity
mismatch, unsafe restoration condition, or twice-repeated infrastructure wall.
A timeout is an incomplete/hang result and never a pass. The raised arm receives
the same wall bound as the default arm, not a longer workload budget.

**Observations and request accounting.** Arm the same DXG/NT/event/process observers
before the isolated workload in both arms, and record their identities and loss
counters. Record first new nvlddmkm 153, Display 4101, new-origin WER 141/117,
bugcheck, boot change, application exit/hang, and target Started/Completed/Failed.
Never count replayed old WER reports as new failures. Record the paging-channel
identity from birth/retirement evidence rather than assuming token 0x80c survives
a reset. Preserve request, queue, start, apply, ready, release and resume separately.

Enumerate **all observed requests**, including release=None and apply=None.
Deduplicate by process lifetime/ticket; record each request's observation horizon,
last stage and any cancellation. A request unreleased at horizon is right-censored:
its known wait is a lower bound, never zero, excluded, or assumed completed. Report
requests issued versus settled, pending gauges and overflow/missing-hook/dump-loss
deltas. Fixed bab3's 64-history/512-pending journal cannot promise an all-request
census. No MAPLOG flag or product change is introduced to disguise that limitation.
If a negative timing conclusion needs omitted requests, label it **inconclusive**;
do not say no long split occurred. Exact unreleased population counts remain
unidentifiable where the fixed telemetry loses requests.

**Clock/causality discipline.** Capture host monotonic/unix pairs, guest UTC/QPC
frequency pairs and independent marker exchanges bracketing the workload. Preserve
before/after bounds and drift; map intervals, not fabricated point timestamps.
The old remote command pairs span roughly three seconds and cannot alone prove
a 6 ms relationship. Never select clock offsets by maximizing completion matches.
Report all-node and per-node submit/completion/preemption intervals and the TDR
event; split the timeline into pre-TDR and recovery. A gap beginning at the TDR is
not evidence that the gap triggered it. A GPU observation is distinct from CPU
observation and from guest scheduler recognition. Alignment wider than the tested
relationship, missing events or recorder overflow precludes that causal claim.

**Outcomes and controls.** Publish the complete ten-slot disposition table, exact
settings, target selection, timing distributions with denominators, censored/lost
requests, and first failure per arm. Success requires the target's own completed
subtest/result record, valid nonempty benchmark output and checked assertions, no new
TDR/bugcheck/app failure, stable boot, and completed cleanup checks. A zero process exit
alone is insufficient; a null exit is UNKNOWN. The isolated batch path must await
its actual child processes and preserve the target's frame/result record; a
wrapper-authored Completed line is not a substitute. SPEC completion is a diagnostic
status, not a byte-correct GPU oracle. Keep any companion raw-client byte/assert/
completion oracle from the owner's separate B(3) work explicit; do not claim it
was exercised by this preparation or by a completed rendering benchmark.

VF1's historical native pass is context only: TDR overrides and byte-identical
image matching are unknown, and it used one boot, different capacity/desktop.
A genuinely matched default-TDR native control requires separate future owner
authorization and sealed identities; do not perform VFIO handoff as part of this
registry-only kayfabe experiment without it. If no new control is authorized,
retain that attribution limit in the final report.

**Ownership/cleanup gate for a future authorized run.** Use the shared host lock
only after authorization, strictly serial, never disturb a foreign VM/process.
Preserve the active host desktop and restore the pre-run state after every arm.
Stop only owned QEMU with QMP powerdown; a bounded fallback to QMP quit is a
recorded unclean-shutdown outcome, never fabricated cleanup success. Preserve
bounded text evidence and hashes before deleting only the owned CoW/UEFI copies;
verify owned PIDs exited, the parent unchanged, desktop healthy and lock released.
An owner-approved run does not authorize a driver fix, host rebind, firmware change,
longer timeout, new flag, new host campaign or provider rental.

No scores are required. Configuration is SPECviewperf 15.x on a Windows guest,
kayfabe/QEMU with RTX 4070; these diagnostic conditions are non-compliant.

(nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

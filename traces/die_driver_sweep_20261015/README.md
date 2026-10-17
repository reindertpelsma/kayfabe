# Die × driver campaign, 2026-10-15

**STATUS: LIVE, 2026-10-17 00:44UTC.** Current steering is
`/root/codex_prompt_dies_20261016.md`; earlier campaign prompts remain binding.
No own rental active. Latest56302930 retired/APIabsent00:40:19UTC aftertextpush;
originaltimer canceledonlyafterabsence. Campaigncomputeceiling10.50USD+transfers.

Latest [AD102wave](ad102_second_builds_20261016/linux590_apps/RESULT.md): exact9d
Linux590raw30/CUDA4/graphics5+10nativehashes/CinnamonX11SHM PASS;595raw30PASS,
1606startupHB0warnings remain. AD102Linux580/590/595 noweachhas functionalraw/CUDA/
graphics/broker passes atitsrecordedrevision;590retains18STALL/10nvAssert limitations.
Windows580P19 referencePASS; threeone-rowablations: caps00cabsenceCode0butdesktopFAIL,
2801or2806 individualremovaleachPASS. Nojoint-removal/otherbuild/die conclusion.
[Row ablations](ad102_second_builds_20261016/windows580_ablations/RESULT.md).

Windows581.51 actual8151/GSP1/P19/ABI580.94.02 guest FAILCode43. SameAD102
[native/VFIO581reference](ad102_second_builds_20261016/native_windows58151/RESULT.md)
passesCUDA3/hardwareDX11+12 beforeafter420.59s/14rounds/0TDR/GSP580.94.01.
Losslesssequence0 nativeGSPmetadata confirmsB297alloc12bytes/status0, whilevirtual
refusesitsunsupporteddrivercell atfirst_seq216. Thisisaleadingcompatibilitycandidate,
notaprovenrootcause; publicheaders/nativefacts mustsupportanynewcell, no guessedack.
CapturewrapperfailedBlackwell-specificmethod3gate; actualAD102method1streamsindependently
passdecoder/pairing/prefix/losschecks, withthewrapperfailurepreserved. Nativecaps99words/
all4096equalWin580/Linux595. Nativeheadlessisnotaconnected-displayorderingreference.
PR31DRAFT vocabulary aloneinsufficient;610.88exactpublicABI stillUNSUPPORTED/nooverride.

GB206 e1bdLinux595raw30/CUDA4/headlessgraphics10nativehashesPASS;595brokerUNRUN.
Linux580onegroupfixesmodesbutdesktopFAILblackframe/fliptimeout; Windows580Code43after
boundeddisplay/CMCprobes. GB205native580and595rawpass, guestWindowsUNRUN. GA104Windows
UNVERIFIEDQGA/binding; TU106LinuxR34timeout180remains, Windows580holdPASS. No familycomplete.
Theseupdates supersede stale ownership/queued-work wording below; codePRsremainDRAFT
againstowner topic andmaster/ownerGPU/VM/checkout remainuntouched.
Initial base: origin/claude/gpu-types-20261015 plus cursor fix2f99e673d;
initial combined product revision23585a319.

## Active wave 2026-10-17 — owned GA104 Linux595 completion

Only own rental **56326220** is active (machine56506), created01:30:28UTC.
Timer2714225 was armed immediately for **04:25:28UTC / 06:25:28CEST**;
manual retirement target03:55UTC, with API absence required before05:00UTC/07:00CEST.
Own receipt and queued hypotheses are in `ga104_595_completion_20261017/`.
Preflight verified KVM,138GBfree root,49GiBRAM and9logicalCPU; actual die/595/native
workloads remain pending. Source9d6f210ba is built on the rental, with a fresh595 fat
guest from public packages. No shared-host access. Root build PID1595; serial queue
V2 PID15243. The first queue was stopped before any GPU workload to correct its
native scoreboard path; no workload/installer replay. Official pinned Windows media
fetch is GPU-free optional preparation; no Windows binding/result is claimed.
Current M1/M3 falsifiers and limits: [plan](ga104_595_completion_20261017/PLAN.md).
Previous no-active statements below are historical and superseded by this ownership.


## Update 2026-10-17 01:26 UTC — coordinator addendum applied; no active rental

MEASURED: live API reconfirms own56302930 absent. Retirement was **02:40 CEST /
00:40 UTC**, after the coordinator's02:30CEST observation. New rentals tonight must
be destroyed and verified absent by **07:00 CEST / 05:00 UTC on October17**, as well
as obeying the existing three-hour cap. All shared-host access waits until the owner
is awake. D-1..D-6 remain open; no master merge. `git ls-remote` exactly matches the
clean main evidence and PR46 local heads; this update is pushed and rechecked too.
No new rental/GPU run or compatibility claim. [Reconciliation evidence](addendum_20261017/RESULT.md).


## Update 2026-10-17 01:03 UTC — Windows 581 allocation diagnostic prepared

MEASURED: [draft PR46](https://github.com/reindertpelsma/kayfabe/pull/46), code
`90364d9d8` after failing-first `da63e54eb`, passes 1,382 CPU tests. The new allocation
entry uses the exact public 580.94.02 layout and our native class/size measurement,
restricted to AD102 and Windows 581.51. It adds no scheduling acknowledgement or
normal class capability. Regeneration is byte-identical; unsupported dies/builds
and malformed inputs refuse. [CPU evidence](ad102_second_builds_20261016/runlist581_cpu/RESULT.md).

INFERRED: this should remove the first unsupported-cell refusal, but its hardware
falsifier remains UNRUN. The measured guest result is still **Windows 581.51 Code 43**;
native 581.51 on the same board passed. New-build scheduling reply-body semantics
remain **NOT DERIVABLE** from the retained metadata/public headers. No family is complete.
No new rental or GPU use; the last rental remains retired and all work is pushed.


## Hypotheses stated before runs

| ID | hypothesis | falsifier | result |
|---|---|---|---|
| B0 | The combined product revision passes all GPU-free `kf-*` tests. | Any failed test or failed compilation, with all crates attempted (`--no-fail-fast`). | Falsified at `23585a319`: 3230 passed, one aggregate-RSS test failed. The exact test passed alone. Fix `59ea4dbaa` isolates both process-RSS oracles; full rerun 3231 passed, zero failed, 14 ignored. `gpu_free/{before,after,rss-isolated}.txt`. |
| A1 | GA104 (RTX 3070) on open host 595.91.07 runs the gates, raw bare-metal suite and fast guest on the combined revision. | A failed gate, non-USER birth, or any failed/crashed/not-run arm; distinguish provisioning failure from a product failure. | The first suite falsified the all-arms-pass prediction (A3); a fresh-seed rerun passed: gates 9/9 + gate10 PASS, every birth USER, native raw 30/30 and fast guest 595.91.07 30/30 at `ba864a55`. The initial native 29/30 is retained (A3). |
| A2 | The same GA104 host runs the native CUDA ladder and the stock Linux 580.159.04 guest ladder correctly. | Any rung's content oracle fails; a native failure prevents attributing that workload's guest failure to kayfabe. | Four native rungs passed. Initial guest run failed before work due to the wrong profile (A2-profile); after the harness fix, all four guest rungs passed, same QEMU binary. `ga104_3070/cl_codex_ampere_580_receipt_guest.out`. |
| A2-profile | The fat guest's GSP boot failure comes from selecting host ABI 595 for guest driver 580.159.04. | With only the explicit guest-driver property corrected, cup2 still fails at the same queue-header check. | Not falsified: cup2 passed on the same binary `ba864a55`; full ladder rerun passed 4/4 after the receipt fix. The first four failed rungs are retained as a harness failure. |
| A3 | Native concurrent-fuzz's initial seed generated no engine work in its positive control, rather than detecting a GPU correctness failure. | Replaying seed 1792058516649537499 performs engine work and still fails its control, or shows a correctness violation. | Not falsified: the original seed replay again produced zero control engine operations and no violations. Seed 1 performed 17 control engine operations and passed. A complete native suite rerun (new generated seed) passed 30/30; no grader code changed. Both seeds and the first failed run are retained. |
| G1 | GA104 renders the graphics suite correctly with native and 580.159.04 guest userspace. | A native failure (environment/workload finding), or a guest step failure / render hash mismatch after the native oracle passes. | Native and guest steps passed; ten render hashes matched at `ba864a55`, box 56076314. |
| W1 | GA104 with the Windows 580.88 baseline reaches Code 0, desktop and DWM under the production profile. | Any nonzero adapter problem code, absent desktop/DWM, boot failure or TDR. A passing desktop additionally needs a D3D app before the cell passes. | Pending: two-boot native-image INF/GSP setup, then a fresh production-profile QEMU process. Unknown GA104/580 caps remain an explicit measurement gap. |

Evidence is measured only when a completed command's text output is present here.
Every cell must name its actual QEMU revision, host/guest drivers, box and evidence.
Boot, CUDA, graphics and display are separate checks; partial checks do not make a passing cell.
The owner's running Windows VM is untouched. No GPU workload runs on the shared owner host.
GPU-free builds use `/data/codex/` with two build jobs.


## Family acceptance (owner clarification, 2026-10-15)

A die/family is covered only after both Linux and Windows guest validation suites
pass across several actual driver builds. Prioritize Turing, Ampere, Ada and
Blackwell before adding many boards inside one family. Plan at least three
supported driver builds per OS/family, selected by the public API boundaries
and native driver support; never label an unsupported physical driver/die pair
as a kayfabe regression. Every result remains keyed to the exact tested die,
OS, host and guest driver, source revision and evidence. One passing die does
not prove the remaining dies in its family.

| Family | This campaign | Full family acceptance |
|---|---|---|
| Turing | TU106 Windows580 Code0/DWM/CUDA3/D3D11+12/zeroTDR and Linux580 Cinnamon/OpenGL/X11broker PASS at8e3a9d32; Linux580 CUDA4 beforefix PASS/raw29 of30 before/afterfix (R34timeout180s) | incomplete: required Linux raw pass and several actual driver builds remain |
| Ampere | GA104 Linux580 broad pass; Linux595 raw pass; Windows580 UNVERIFIED after QGA timeouts; actual binding not established | incomplete |
| Ada | AD102 Linux580 raw30/CUDA4/graphics10+broker and Windows580 Code0/DWM/CUDA3/D3D11+12/zeroTDR pass; Linux595 raw30/CUDA4/graphics10+broker pass; Linux590raw30/CUDA4/gfx10/brokerPASS; Windows580421secondhold/BlenderCUDA/EdgePASS; Windows581guestCode43/nativesameboardPASS | incomplete: Windows581Code43; furtherWindowsbuilds and otherdies remain unverified |
| Blackwell | GB206 Linux580 raw30/CUDA4/headlessgfx5+10hashes PASS atc2092cf7; desktopFAIL (one derived group now validates modes, but blackframe/fliptimeout); GB206 Linux595 raw30/30/CUDA4/graphics5+10hashes exacte1bd; thisrerun0HBwarnings, prior10startupHB0 remain; GB205 fixed-clock595raw30 with startup warnings; nativeWindows580 PASS; GB206kayfabeWindows580 Code43, GB205guestWindowsUNRUN | incomplete: initialization fix, broker/graphics and several actual builds remain |

Rental56076314 retired12:40:19UTC; native Windows installation completion
was never verified there. AD10256096336 retired/absence verified15:39:44UTC.
Turing56117789 retired by its revised timer, absence verified18:39:01UTC below
the authorized three-hour cap. Windows580 and Linux580 broker pass, raw29/30
before/afterfix leaves full Linux acceptance open. No own rental is active at this
checkpoint. Exact results: turing_2060super/README.md and ad102_4090/README.md.

CPU verification audit: the bench README requires one Cargo target directory
per checkout. Some CPU-only runs reused target-gpu-free across own worktrees.
Fresh GPU-free runs now use target-host-gid and target-census-linux. Fresh checks completed at12:35UTC: product42483a475 (GID+C57E+exact595
vocabulary) passes3234 tests,0 failed,14 ignored; Windows-census topic passes51,
0 failed,1 ignored. Logs in fresh_verification/. The audit is resolved by these
checkout-specific runs. Rental
hardware builds use one checkout and its own target; their binary provenance
is unaffected. Record the fresh results either way.

## Next work

Linux first: GA104, then available Ada, Turing and Blackwell; driver endpoints selected
from the committed API intervals, with the GSP boundaries considered separately from controls.
Windows acceptance requires desktop, Code 0, DWM, a D3D app and zero TDR, under the production profile.
For each failure: first failing operation, stock oracle, falsifier, failing regression test,
derived fix, rebuilt cell and regression runs. One wall is bounded to about three hours.

Open owner decisions D-1/D-2/D-3 remain as recorded in
`docs/design/V3_GPU_TYPES_20261015.md` and `V3_CONSTANT_MATRIX_20261014.md`.
F-6 board-specific fallback removal is already in the base; it is not new campaign work.

Budget and instance ownership: [COST.md](COST.md).

## GA104 wave, measured results

Box 56076314, RTX 3070 (10de:2484), stock open host 595.91.07.
All completed hardware runs used `kf3-bins/ba864a55/qemu-system-x86_64`.

| Guest | Raw | CUDA | Graphics | Desktop / broker | Cell |
|---|---|---|---|---|---|
| Linux 595.91.07, kernel 6.8.0-59 | 30/30 | not tried | not tried | not tried | incomplete |
| Linux 580.159.04, kernel 6.8.0-142 | 30/30 with automatic receipt | 4/4 | native + guest steps pass, 10/10 hashes match | Cinnamon + NVIDIA gears about 60 FPS; actual X11 SHM presentation on Xvfb | pass for these checks; diagnostics below |
| Windows 580.88 | not tried | not tried | not tried | realize refused before boot | fail; missing host GID ABI inventory row at 595.91.07 |

The original thin initrd also passes its timer arm with only an explicit
`guest-driver=580.159.04` added; no rebuilt-initrd confound in that comparison.
Before and after logs are retained. The full rebuilt thin suite passed 30/30.

The broker test measured zero guest Xid and zero flip timeouts, with 12 NV assertions.
The display drainer emitted idle-window/head STALL diagnostic dumps; the reported
36 is a count of diagnostic lines, not 36 separate stalls. Presentation used X11
SHM on Xvfb, not a measured DMA-buf path. Teardown ultimately forced the QEMU down
(exit 137) after the harness's ACPI shutdown wait; graceful shutdown was not proved.

Windows first refusal: host ABI rejects `GPU_GET_GID_INFO` (0x2080014a), then
`gpu-uuid=auto` correctly refuses without the host UUID. Hypothesis H-GID: the
published control is absent from the host ABI inventory, so a generated layout row
and control registration will make the default identity work. Falsifier: the
correctly carried unprivileged control still fails or produces no valid UUID.
Next: failing test, compile the public SDK layouts at the committed measured tags,
register the control, rebuild, retry the same default Windows profile. No random
UUID bypass is used to label this production cell passing.

Image transfer hypothesis (stated before the comparison): interrupted source or
sink pipelines can leave false completion markers. Falsifier: all21 chunk hashes
match the trusted source. Measured: exactly02/03 differ; the other19 match.
Two regression tests reproduce hidden receiver failure and premature marking on
sender failure, fail before and pass after. The two repaired chunks and full
source/destination SHA256 match are in image_transfer/repair.txt. Windows now
reaches QGA on the same GID-only binary f6d96614; GPU acceptance still pending.

Combined candidate 8e45edc8a (GID + C57E): full GPU-free3234/0/14 ignored,
GA104 fast30/30 and actual X11 SHM broker presentation PASS, guest Xid0,
flip timeouts0,12 NV assertions. SSH poweroff gives QEMU exit0. Detailed evidence
is on codex/hardware-regression-20261015 in hardware_regression/. This does not
claim Turing hardware or Windows acceptance.

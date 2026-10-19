# Trap-cost experiments for the Steel Nomad gap (2026-10-19)

**STATUS: RESULT, 2026-10-19. MEASUREMENT ONLY, branch `claude/trap-experiments-20261019`, never merged, never shipped.**
Labels: MEASURED = a run or test named here. CODE READ = source. INFERRED = reasoning. Base `origin/claude/tsg-mirror-20261018` (c2685412e, binary 0d7ba743).
Everything is behind Cargo feature `diag-trap-exp` (not in any `default`) plus default-off env flags; none is needed to boot.

## Answer
**Trapped writes are not what limits Steel Nomad.** MEASURED: adding +10 / +20 / +100 us to EVERY trapped interrupt-tree write (about 14.4 per tiny submit, so +145 / +290 / +1445 us per submit,
tiny DIRECT probe 266 -> 427 / 576 / 1711 us) leaves Steel Nomad where it was: control 2936 [2885,2963] and 3006 [2904,3034] (fps = score/100 = 29.4 / 30.1), +10: 2954 [2947,2979], +20: 2967 [2910,3024],
+100: 2951 [2913,2977]. Regression slope over 0/10/20/100: 0.0004 fps per added us (indistinguishable from 0; the two controls differ by 2.4 %, more than any delay arm differs from them). Doorbell +10 us: 2967 [2923,2995].
So the share of the 25 % gap (29.4 vs 38.5 fps) that is trap cost is **0 within the run spread (<= about 2-3 % of fps, i.e. at most about 1 of the 9 fps)**, even though traps ARE a large part of the tiny-submit latency
(MEASURED: one trapped store costs 6.2 us in the Linux guest, so about 14.4 traps = 90 us of the 205 us submit excess over VFIO). Latency of a tiny submit is not what the benchmark is bound by.
The interrupt-page spinner never produced a working guest (see below), so there is no spinner "ceiling" number.

## Source revisions, binaries (host /data/claude-trapexp-20261019/kf3-bins, ROM paths symlinked to qemu-build-kf3)
| dir | commit | content |
|---|---|---|
| 02911cbb-ctl | 02911cbb (stage 3) | feature OFF (control); md5 7c7eb268... |
| 02911cbb | 02911cbb | feature ON: delays, variant A spinner (`SPIN=1`); md5 eed1439e... |
| 1bd33400 | 1bd334007 | + variant B (`SPIN=2`, model-driven, hold 300 us, EN_CLEAR reads 0) |
| cacfcc62 | cacfcc629 | + EN_CLEAR reads the mask, `HOLD_US` |
Feature-off proof: `FEATURE_OFF_PROOF.txt` (control archive contains none of the flag/thread strings: 0 matches vs 3; the only non-cfg change in production code is `memory_map` -> `memory_map_base` + a feature-gated wrapper).

## Tests (GPU-free), expected-RED first for stage 1 and 2 (`67d888192`, then GREEN); the variant-B additions were written with their tests in one step (not RED-first)
Default features, before/after: kf-trap 111 / 111, kf-qemu 408 / 408 (re-run until stage 3; the later commits are cfg-gated, default not re-run after them). With `diag-trap-exp`: kf-trap 162, kf-qemu 445 at `1bd33400`, +3 tests later (39 trapexp lib tests green at head; full-crate count not re-run).
Mutants (each killed by >= 2 tests): `MUTATIONS_STAGE1.json` (24), `MUTATIONS_MEMMAP.json` (4), `MUTATIONS_KFQEMU_TRAPEXP.json` (12), `MUTATIONS_VARIANT_B.json` (14).

## Runs (all: mirror ON, fresh qcow2 overlay of the owner's disk win11-full-b, 8 vCPU, flags `KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5` + the arm's; rev/flags printed from /proc/<qpid>/environ in `evidence/arms/*/result.json`)
Steel Nomad n=5 per arm unless stated (3DMark result-file score; fps = score/100, P1 measured score 2967 = 29.68 fps). Tiny probe `sl_probe.ps1` medians of 2000 submits, n probe repetitions per queue. Xid: host `dmesg` count 21 before and after the whole session; per-arm journal delta 0 in every arm; guest System-4101 / 153 / app crashes 0 in every arm that reached the desktop agent.
| arm | rev | flag | SN score median [min,max] | DIRECT / COMPUTE / COPY us (n) |
|---|---|---|---|---|
| control1 (first) | 02911cbb-ctl | - | 2936 [2885,2963] | 266 / 229 / 192 (10) |
| intr +10 | 02911cbb | `KF3_DIAG_TRAP_DELAY_INTR_US=10` | 2954 [2947,2979] | 427 / 378 / 338 (10) |
| intr +20 | 02911cbb | `..INTR_US=20` | 2967 [2910,3024] | 576 / 524 / 482 (10) |
| intr +100 (positive control, added) | 02911cbb | `..INTR_US=100` | 2951 [2913,2977] | 1711 / 1652 / 1611 (5) |
| db +10 | 02911cbb | `KF3_DIAG_TRAP_DELAY_DB_US=10` | 2967 [2923,2995] | 259 / 253 / 205 (10) |
| db +20 | 02911cbb | `..DB_US=20` | 3027 (n=1; run 2 produced no result file, no TDR/Xid, cause unknown, not repeated) | 267 / 269 / 217 (10) |
| control2 (last) | 02911cbb-ctl | - | 3006 [2904,3034] | 277 / 272 / 195 (10) |
| spinner A | 02911cbb | `KF3_DIAG_INTR_SPIN=1 ..SPIN_CPU=23` | none | agent never came up (10 min) |
| spinner B | 1bd33400 | `SPIN=2` | none | agent up, probe: "no NVIDIA adapter" |
| spinner B3 | cacfcc62 | `SPIN=2 SPIN_HOLD_US=2000` | none | agent never came up (10 min) |
Order was control, spinner A, intr20, db20, intr10, db10, control2, spinner B, intr100, spinner B3 (reordered to put the risky arm first; control bracketing shows a +2.4 % upward drift over the session, so scores are not order-corrected). One boot per arm.
Slopes (`evidence/summary.md`): tiny probe vs interrupt-tree delay 14.35 / 14.19 / 14.16 us per added us (DIRECT / COMPUTE / COPY) = **about 14.4 trapped interrupt-tree writes on the critical path of a tiny submit** (matches the ~14 inferred by P1). Tiny probe vs doorbell delay: 0.04 / 2.0 / 1.2 us per added us, i.e. at most about 0-2 doorbell traps on the path, not 3.
Counters (`evidence/qemu_counters.txt`): irqfd writes (`raised`) about 1.17-1.26 M in every full arm; interrupt-tree traps per raised interrupt about 4.2-5.4 (control 6.47 M writes / 1.235 M raised); delayed-write counters equal the trap counts.

## Experiment 1, delay injection
Hypothesis: trapped interrupt-tree / doorbell writes cost fps. Falsifier (stated before): +20 us per interrupt-tree trap changes fps by less than the run spread (n=5). **Result: falsifier met (traps do not matter for Steel Nomad).** MEASURED. The method is sensitive (probe slope 14.4 us/us; +100 arm moves the probe by +1445 us).

## Experiment 2, interrupt-page spinner (BAR0 0xB81000 as plain RAM; the page backs a BAR0 shadow piece, mapped by the existing PlainRam path, no QEMU C change)
Hypothesis: with the interrupt page untrapped Steel Nomad fps moves beyond the spread. Falsifier: it does not. **UNDECIDED: the guest never ran a benchmark with a spinner.** MEASURED outcomes:
* A (forward everything, brief's design): Windows boots, GSP comes up, then the driver fails and the device is reset 3 times in 12 minutes (`phase` Cold -> Running -> Cold), the agent never starts. Counters: 2 triggers raised (the loopback self-test did fire), 284 worker raises, learned masks leaf0 0x3f, leaf2 0x101, leaf4 0xc10007a, 67 M page stores, 2225 lost CAS. INFERRED cause: pending bits that never clear (ISR sees "still pending").
* **Fundamental, CODE READ + unit-tested:** a W1C ack that stores the very value the word holds (the normal case: the ISR writes back what it read) changes nothing in plain RAM, so a poller cannot see it. Pending can only be cleared by a timer (variant B/B3 "hold") -> a short hold misses interrupts, a long hold gives phantom pending. Variant B (model-driven: guest stores applied to the real `CpuIntr`, answers published by compare-and-swap, workers publish before raising; hold 300 us, EN_CLEAR read-back 0): the driver loaded but no adapter was usable, no trigger raise reached the guest (`trigger_raises=0`), GSP Suspending -> Halted. B3 (EN_CLEAR reads the mask, hold 2000 us): loopback raised (2), GSP stayed Running 15 min, 2294 raised / 2343 held, agent still never came up. Cause of B/B3 not isolated (time box); the brief's fallback B (atomic OR by workers) has the same invisible-ack problem.
* A spinner that would work needs W1C visible: only a write trap (or KVM dirty-bit write tracking, which is a trap on AMD without PML) gives that. No spinner "ceiling" is claimed.

## Owner addition: WB for the interrupt page
(a) MEASURED (Linux fast guest, 3 vCPU, kf3-gpu, `evidence/fgprobe_*.out`, `trapprobe.c`): WB RAM page read 4.0 ns / store 11.5 ns (with mfence); BAR0 through sysfs resource0 (UC-): read 98-101 ns (shadow ROMD page, same on the RAM-backed interrupt page); store to the RAM-backed interrupt page 175 ns (spinner build); **the same store trapped (control) 6234 ns**. So untrapped UC stores are 35x cheaper than traps and a UC floor of ~100 ns per read remains.
(b) CODE READ (Linux master fetched from github, not the host's 7.0.0-34 tree): `arch/x86/kvm/svm/svm.c` defines no `get_mt_mask` (grep: no match) while `vmx.c` has `vmx_get_mt_mask` with `VMX_EPT_IPAT_BIT`; `mmu/spte.c` calls the hook only if present. So on AMD NPT KVM cannot force WB per page; the guest PAT/PCD/PWT chosen by the guest driver decides (Linux sysfs: UC-; Windows: closed, not examined). BAR0 is advertised 32-bit non-prefetchable (`pci_register_bar(.. SPACE_MEMORY)`, resource flags 0x40200: no `resource0_wc`); BAR1/BAR3 are 64-bit prefetchable. Making BAR0 prefetchable would at best allow WC, not WB, and changes the driver's view: not done.
(c) The guest stays unmodified; UC is a floor on any spinner gain. No host-side WB override implemented (none exists on this CPU).

## Not done / limits
db +20 has n=1 SN; spinner arms have no fps; arm order not balanced (control bracket only); fps is score/100 not a broker capture; ISR/frame not measured (only irqfd and trap counts); the Linux fast-guest init runs (`fginit_*`) saw zero interrupt-page writes in 3 s and say nothing; Phase-1 build rev differs from head only by scripts/docs for 02911cbb, but 1bd33400 and cacfcc62 are the heads of their commits.

## Decisions for the owner
1. Drop trap elimination (spinner, coalesced MMIO, write-back page) as a Steel Nomad lever: measured gain 0 within 2-3 %. It remains a tiny-latency lever (about 90 of 205 us per submit).
2. The 25 % gap is elsewhere; do not spend more on interrupt/doorbell trap cost for fps. Candidate next measurement: GPU-side per-frame timeline (queue idle gaps) rather than submit latency.
3. If the polling idea is still wanted: it needs a W1C-visible mechanism; plain RAM cannot provide one (see above). I recommend not pursuing.
4. Keep this branch unmerged; the four binaries stay in /data/claude-trapexp-20261019/kf3-bins until you say delete.
5. Optionally repeat db +20 and a control triple to tighten the doorbell arm (about 40 min of the VM).

## Final host state
Owner's VM restored exactly as found: run 3018, launcher /data/claude-manual-owner-perf6.sh, binary 0d7ba743, `KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5 KF3_TSG_MIRROR=0`, OWNER_SESSION_READY 21:49:41 (it was run 3017 before; stopped 19:12 via the stop file, OWNER_SESSION_EXIT 19:12:58). Lock holder released 21:49:35. Two of my QEMUs (spinner B, B3) did not exit by themselves; each was stopped with SIGTERM to its exact pid (no kill -9). Sampler and the tdropus queues untouched. Host Xid (dmesg) 21 -> 21. Big build dirs, overlays and initrds deleted; logs, results and binaries kept under /data/claude-trapexp-20261019. Local target dir deleted.

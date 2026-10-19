# R1 report — 2026-10-19

## 1. Identity and request

Lane R1, branch `codex/trap-research-20261019`, base `0d7ba7438`. Request: GPU/VM-free interrupt-tree oracle/replay tests, trace mining and feasibility research for reducing BAR0 write exits; include vCPU blocking and host-mitigation audits. Every stage was pushed without force; the private handoff records the final origin-verified head. No merge or integration-branch change.

## 2. Result (DONE / PARTIAL / NOT DONE)

- DONE — MEASURED: independent all-family differential/read/shadow oracle and exhaustive single-edge schedules; no existing serial CpuIntr defect found.
- DONE — MEASURED: naive W1C/TRIGGER counterexamples fail in separate EXPECTED RED commit; bounded serial replay turns the gate green, retaining negative controls.
- DONE — MEASURED: no_std capability gates; 19 restored-source mutants killed by >=2 tests each.
- DONE — CODE READ / MEASURED: pure replay, coherent-read and final-unmask counterexamples, detailed per-thread/lock audit; no device coalescing wired.
- DONE — MEASURED: 2,994 retained submit intervals: median 12 masks/2 acks/3 doorbells; older Windows diagnostic has 65,197 interrupt-page reads.
- DONE — CODE READ / MEASURED: read-only host/KVM/QEMU/mitigation inventory; actual host export check; no kernel configuration change.
- DONE — CODE READ / MEASURED: missing state diagnostic prepared, compile/runtime off by default, fixed atomics on producers, drainer-only control/output, locally tested.
- PARTIAL — CODE READ: exact patched-host callback is a sketch; no complete lifetime/readback/injection proof or supported stock external-module attachment.
- NOT DONE as ordered — MEASURED n=0 new hardware runs: real exit cost, mitigation share, active AVIC, masked-pending frequency and performance gains remain unknown.
- DONE — CODE READ: hardware plan includes commands, falsifiers, risk/effect scenarios and slot times; rejected designs are not scheduled as performance arms.
- NOT DONE — MEASURED: target-directory deletion was refused by automatic approval review; STOP rule applied, no alternate deletion attempted.

## 3. Verification and mutation table

MEASURED: `kf-trap` 111→140 passed, 0 failed/ignored. `kf-qemu` baseline/default 408 passed; optional `diag-intr-audit` 411 passed. Combined final default548/0/0; both optional features551/0/0. All release, -j2, nice19. Restored final trap run140/0. Oracle120,000 random operations +86,016 exhaustive schedules; replay150,000 random operations, zero extra messages at serial barriers. These do not prove concurrent linearizability.

MEASURED: Clippy before/after full multisets identical:89 inherited errors/440 warnings, added0/removed0, changed-line diagnostics0. Both Clippy exits101 are explicit. Changed Rust rustfmt and git diff checks pass. `VALIDATION.json`, `CLIPPY_FINAL.json` and the mutation JSONs hold counts, commands, raw-log hashes and exact killing tests.

| Mutant | Killing tests |
|---|---:|
| fire_only_on_latch | 3 |
| fire_only_on_enable | 3 |
| double_fire (replay message count) | 7 |
| ignore_top_enable | 3 |
| ignore_leaf_enable | 3 |
| drop_w1c | 3 |
| reorder_set_clear | 3 |
| lose_owed_edge | 7 |
| apply_takes_a_lock | 2 |
| remove_replay_bound | 2 |
| eager_masks | 5 |
| reset_retains_state | 3 |
| audit_default_armed | 2 |
| audit_overrun | 3 |
| audit_publish_incomplete | 3 |
| audit_drop_pending_snapshot | 2 |
| audit_drop_message_result | 2 |
| audit_ignore_stop | 2 |
| audit_runtime_default_on | 2 |

MEASURED: diagnostic API EXPECTED RED was E0432 for an absent new API, not a claim of an existing production defect. The earlier replay EXPECTED RED contains two assertion failures on an explicit naive fixture, not changed production code. No unsafe Rust was added. Capability checks exclude std locks/allocation/OS calls in the compiled source closure, not custom atomic spinlocks or all external device wiring.

## 4. Owner answers and coordinator verdicts

CODE READ / MEASURED: (a) ordinary RAM cannot implement SET/CLEAR/W1C/shared aliases or retain write order; page permissions cannot isolate offsets inside the same writable page. (b) memory-backed untrapped reads already exist; default owner logs3014/3015 have1,089/1,699 status lines, all read_exits0. (c) coalesced MMIO still has a kernel VM exit, usually avoids userspace return, takes VM ring_lock and has finite-ring fallback. Generic QEMU flush can run on the vCPU, conditionally taking BQL on non-lockless targets; kf3 itself is already lockless. It is not “every exit/any MMIO”. No hardware-free emulated SET/CLEAR device was established.

| Coordinator statement | Verdict |
|---|---|
| 1. Plain RAM loses semantics/order | CONFIRMED — model counterexamples and register readback contract. |
| 2. Kernel FIFO, latency, automatic flushing | REFUTED as written — FIFO/ranges/170 slots confirmed; universal flushing false;1–2us kernel estimate UNKNOWN. |
| 3. Mask-only safe split plus owed timer | REFUTED — stale alias reads, latch/barrier order and guaranteed final drain unresolved; no timer introduced. |
| 4. Doorbell ioeventfd opportunity | CONFIRMED available already, default off; DATAMATCH uses normal MMIO bus, not length-zero FAST_MMIO. Kernel synchronization/BAR-remap waits remain audit gates. |
| 5. AVIC acceleration | UNKNOWN effective per-VM state; hardware/module support confirmed, launcher lacks hv-avic, AutoEOI can inhibit. |
| 6. Exact kernel device / external module | UNKNOWN complete design; stock-module shortcut REFUTED by missing registration export on actual host. |

MEASURED trace counts: whole P1 capture68,460 mask +11,401 ack +9,962 doorbell writes, zero trigger; masks85.724% of interrupt writes. n2,994 submit intervals medians12/2/3, interrupt14 (8..70), cycles2 (1..10). This syntactic candidate fraction is not proven coalescible. Windows read-trace count65,197 includes EN_SET/TOP_EN_SET readbacks. NSI traces expose subscription arming, not leaf/top masks; pending-at-unmask frequency UNKNOWN. Ordered Linux submit traffic remains unavailable in the scoped corpus. Existing hardware VFIO summary is deduplicated; the retained VFIO display trace has no interrupt-page records.

CODE READ / INFERRED: 9–10us/write in prior README is spacing/chain attribution, not isolated exit latency. R1 observes median11.15us consecutive same-thread spacing (n68,369, min7.03/max5095.256), still including guest/IRQ/scheduler/probe work. Apply+shadow is atomic memory work; some writes also signal an irqfd syscall. Host VMSCAPE is conditional IBPB before userspace return; SRSO is Safe RET, Retbleed Not affected. No measured mitigation share or safe guest→host relaxation follows from a trusted single-user host when guests remain hostile.

## 5. Decisions, ranked plan and hardware slots

1. Schedule exit census and scratch loop before selecting implementation; recommendation measure actual kernel/user/dispatch costs with protections retained.
2. Reject the simple mask split and RAM-page proposal; recommendation preserve synchronous apply/readback and use the new oracle for any replacement.
3. Resolve the scope of existing kernel wake spinlocks and hostile BAR relocation before globally approving doorbell fast-path use; recommendation keep defaults unchanged meanwhile.
4. If measured benefit warrants it, review the optional exact KVM callback tier; recommendation an explicit host patch with fault-free atomics/lifetime/IRQ handoff proof, not an unsupported external module.
5. Keep mitigations; recommendation reduce userspace returns rather than weakening the hostile-guest boundary.
6. Resolve cleanup outside this stopped lane; recommendation remove the remaining own target with an explicitly approved mechanism, without bypassing the refusal.

INFERRED sensitivity, not performance measurements: existing doorbell transport3×(9..10−1..3)≈18..27us/submit (0.52..0.78ms/29-submit frame), conditional AVIC0..30us (0..0.87ms), patched-host tree14×delta≈84..126us (2.44..3.65ms). Generic mask coalescing72..108us would be attractive but is REJECTED by correctness/blocking, so no gain is claimed. Estimates are not additive across overlapping work.

| Future experiment | Reserved VM time | Gate |
|---|---|---|
| perf/trace-cmd exit reasons and kernel/user timestamps | 5min after boot | Verify process/event identities and trace perturbation. |
| Scratch/RDTSC N-write transport loop, no GPU commands | 10–15min separate diagnostic guest/microguest | RDTSC companion/coalesced scratch fixture still needs preparation; existing clock_gettime helper identified. |
| One tiny submit with ordered BAR0 reads and bounded state audit | 5–10min after diagnostic boot | New flags prepared, never run; no performance claims from trace arm. |
| Interrupt coalescing A/B | 0 now;15–20min after gates | REJECTED until readback/order/liveness/locking solved; pure replay is not a device flag. |
| Doorbell OFF/ON/OFF | 0 until audit resolved; then20–30min | Existing code/ledger, wake latency and hostile BAR relocation. |
| Active AVIC state / hv-avic A/B | 12–20min | Preserve other enlightenments, prove actual active state/health. |
| Mitigation-aware decomposition | 0 inventory;2–5min piggyback | Security retained; mitigation-off arm0/not authorized. |

CODE READ: perf kvm exit/entry pairing includes combined servicing/scheduling; kernel/userspace subdivisions need kvm_userspace_exit, KVM_RUN ioctl and QEMU probes. Silicon VMEXIT/VMENTRY latency is not separately measured by that pairing. Full commands/falsifiers/risks are in `docs/design/V3_TRAP_REDUCTION_RESEARCH_20261019.md`.

## 6. Limits and cleanup

CODE READ / MEASURED: no hardware prototype, active AVIC verdict, exact host mitigation timing, loss-free masked-edge frequency, full distro-kernel source audit or concurrent real-device equivalence. Hardware performance n=0 for this lane. New trace snapshots are nontransactional and can drop bounded diagnostic samples; losses/overlap remain explicit.

MEASURED: no GPU/VM/guest-agent/host-lock/configuration action was taken. All host commands were read-only evidence/version queries. Lane build directory remains392MiB after automatic approval review rejected its deletion: “rm -f style commands are not permitted. Use a safer approach”. The lane STOP-on-refusal instruction was followed; no reworded or alternative deletion was attempted. No lane build/test/collector process or acquired lock remains. Final origin equality and this cleanup blocker are recorded in the private handoff. No master merge, force push, other worktree write or message to another session/agent.

Protocol incident: an early filename-only discovery used an overly broad directory scope and enumerated a filename in an excluded directory. No file contents there were opened; subsequent discovery was confined to explicit allowed source roots. This scope error is disclosed rather than represented as full compliance.

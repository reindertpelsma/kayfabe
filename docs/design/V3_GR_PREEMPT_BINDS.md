# GR preemption and zcull binds: source audit and research plan

**STATUS: RESEARCH / DESIGN-ONLY, 2026-10-27.** The ENS9 premise update supersedes the proposed preemption explanation for the current stall. The branch tip contains this document only relative to base `d2ea6d26be6f746e579e6bfc8dbfe39847ddb990`; implementation, tests, capability additions and the proposed runtime switch have been withdrawn. No host experiment was performed by PF1.

## Conclusion after ENS9

**MEASURED, coordinator-supplied ENS9 boots 7370/7371 and the PR169 probe, 2026-10-27:** at the stall, all eight guest vCPUs are in HLT, the host/GPU are idle, every observed host group reports GET_CTXSW_MODES `[1,0,0,0,0]`, and the guest trace reports no ctxsw-mode setting. The probe reports `GUEST-SILENT-AFTER-COMPLETION`: the host completed all work it knew about. PF1 has not independently inspected these new host captures.

**INFERRED from those ENS9 observations:** H-PREEMPT, a non-preemptible long GPU packet preventing progress, is **not supported for this stall**. Its required outstanding GPU work is absent from the supplied stalled-state evidence. The next investigation should follow work discovery and completion/wakeup delivery to the halted guest. That evidence does not identify which event or guest state is wrong, and completion of all known work does not establish that every guest-intended submission reached the host.

The five-field readback order is zcull, PM, SMPC, compute preemption, graphics preemption. **CODE READ, base `d2ea6d26`, `kf-qemu/tsgfacts.rs`, and OGKM 580/595 `ctrl2080gr.h`:** zcull 1 is NO_CTXSW; the last two zeros denote WFI modes. A WFI mode is a configuration fact, not evidence of a currently blocked packet. NO_CTXSW resets zcull tables at a context switch without saving/restoring their buffer; it does not disable all depth culling.

**INFERRED:** a late idle/HLT snapshot alone cannot exclude an earlier transient. The cheap remaining check is the interval around the *first* loss of guest progress in the same boot. Earlier ENS7 walker wall-time peaks cannot be joined causally to ENS9 snapshots from another boot without that alignment. No TDR timer change follows from either observation.

## Source-supported conclusions

**CODE READ, base `d2ea6d26`:** deferred zcull/preemption/PM binds lack a default USER adapter. Direct same-client zcull and ctxsw setters have existing owned-object adapters. The legacy Translated/T-space classifier refuses bind VAs that are not mapped in its address space; it is not an existing working bind path. A missing adapter establishes a coverage gap, not that the stalled guest exercised it or needed a different mode.

**MEASURED in retained native census C at commit `6564dd597` and WC tables at commit `1eedda417`:** successful direct preemption-bind counts C/B202/B205/B206 are 9/12/12/12, and zcull-bind counts are 3/5/5/5; direct PM-bind count is 0 in these tables. These are startup populations, not current Enscape trigger/order/value evidence. They do not inventory nested deferred execution. A bind can itself carry mode intent, so a future inventory must inspect bind flags and triggered bundles as well as standalone SET calls.

**CODE READ, NVIDIA generated `g_subdevice_nvoc.c` in OGKM 580.159.04 and 595.84:**

| Control | ID | Export flags / access mask | Host RM access class |
|---|---|---|---|
| GR_CTXSW_PREEMPTION_BIND | `20801211` | `0x10248` /0 | NON_PRIVILEGED |
| GR_CTXSW_ZCULL_BIND | `20801208` | `0x90348` /0 | NON_PRIVILEGED; ordinary caller names its own client |
| GR_CTXSW_PM_BIND | `20801209` | `0x10048` /0 | NON_PRIVILEGED |
| GR_SET_CTXSW_PREEMPTION_MODE | `20801210` | `0x10348` /0 | NON_PRIVILEGED |
| GR_GET_CTXSW_MODES | `20801235` | `0x48` /0 | NON_PRIVILEGED |
| GR_CTXSW_PM_MODE | `20801207` | `0x118` /0 | NON_PRIVILEGED |
| GR_GET_ENGINE_CONTEXT_PROPERTIES | `2080122d` | `0x8` /0 | NON_PRIVILEGED |

These exports provide an ordinary owned-object API surface. Supported modes, runtime access checks and exact refusals remain native RM decisions. They do not authorize a guessed context identity, pointer domain or buffer size. No privileged host channel or extra capability is justified by this research.

**CODE READ, OGKM `kernel_graphics_context.c`:** allocation/map paths supply main/patch/global buffers and may provide PM state; allocation depends on skip-buffer flags, context/subcontext, platform and driver policy. Public GSP sources do not include the relevant bind/set implementation bodies. They do not establish whether a deployed mode setter without explicit buffers allocates defaults, refuses, or selects another mode. Therefore missing guest bind forwarding does not establish missing native buffers or a WFI defect. GET_CTXSW_MODES reports modes, not bound buffer addresses.

**CODE READ, OGKM `kernel_graphics.c` and the SDK:** GR_GET_ENGINE_CONTEXT_PROPERTIES returns native per-buffer size/alignment and `bInfoPopulated`; a query must zero route/prepopulated inputs and reject unavailable output. GET_CTX_BUFFER_SIZE is an aggregate of existing context buffers, not a substitute for each buffer's extent. SDK layouts contain 8/9 preemption pointers across versions; slot 7 is a CPU pool-control pointer, distinct from GPU VAs. Those layout/domain facts are independent of the stall hypothesis.

## What remains open

- Whether any affected workload requests and triggers a bind whose intended effect differs from its host context. ENS9's supplied no-setting observation does not support inventing a preemption mode on the guest's behalf.
- Native automatic buffer allocation and actual bind/set behavior on the deployed driver. Public export metadata and a successful setter are not a buffer-presence audit.
- Context scope across a channel, TSG, ctxshare and subcontext. Several members can share state, but choosing one member from a TSG does not by itself identify the requested shared context or prove all relevant VASes can address a buffer.
- Whether an earlier, genuinely GPU-busy interval preceded the idle guest state. That requires same-boot timing, not an inference from WFI readbacks.
- **Separate zcull performance lead:** NO_CTXSW can discard save/restore state at switches. Whether that contributes to the reported performance gap needs its own request/buffer/mode and context-switch evidence. It neither establishes disabled depth culling nor explains an idle/HLT stall.

## Conditional design constraints for later work

This is a design boundary for a separately justified task, not an implementation proposal to enable now.

An ordinary adapter could author a bind using the guest's own VA only where that entire unchanged span is already mapped writable in the target's owned twin. Resolve logical targets within the VM, replace every host handle, decode guest and encode host layouts independently, query native extents/alignment, and reject holes, overflow, unsupported pointer domains and unknown sizes. Logical kernel/process clients inside one VM need not equal the RPC header client. CPU pointers require a separate translation/ownership contract; a GPU-leaf proof cannot validate them.

Alternatively, host-owned buffers are permitted in principle, but need native sizing, initialization, reserved-range exclusion, mapping, shared-context ownership and successful native Free ordering before release. Neither path may forward opaque guest control bytes or make a private writable mapping without those contracts. New work must remain off vCPUs and outside locks they take. A native operation may block its issuing worker; that latency also needs evidence.

The existing TSG mirror owns a host TSG and explicit ctxshares. A later adapter must resolve actual sharing rather than broadcast a guessed per-channel bind. Queued work must retain allocation identity, exclude native Free, reject canceled/recycled targets and keep the stream suffix parked until a real native result. Ordinary native refusal remains a refusal. Resource bounds are policy limits, never captured per-die sizes. None of these conditions demonstrates relevance to ENS9 or supplies a reason to change its configuration.

## Cheapest discriminating test

1. **Use existing evidence first.** In 7370/7371, align the last guest PUT/doorbell, host submission/completion witness, completion notification, first loss of guest progress and first all-vCPU HLT interval. Inspect the short preceding window as well as the final snapshot. Keep the PR169 qualification: it accounts for work the host knew about.
2. **Falsifier for this stall:** all relevant submitted work completes promptly through the first loss of progress, no GPU work is overdue, and the host/GPU are idle while the guest remains halted. Preempting a GPU packet cannot repair work never discovered or a missing wake after completion. The supplied ENS9 result already points this way; the alignment checks the remaining earlier-transient ambiguity.
3. **Only if that window is missing, collect one unchanged baseline trace.** Reuse the existing completion probe. If necessary, enqueue one short probe in an already warmed, ordinary owned host CUDA context and record enqueue/completion wall times plus GPU-event duration. Prompt independent completion during the halt falsifies ongoing GPU starvation in that interval. Context creation/JIT time must not be confused with GPU queue delay. This is a proposed owner-authorized host test, not work performed here; it needs no new VMM adapter or switch.
4. **Positive evidence needed to revive preemption:** at the onset of lost progress, real GPU work remains outstanding, the engine is busy, and an independent ready context is held up behind it. WFI readback alone is insufficient. If that prerequisite exists, a small native cap-free A/B on owned USER contexts can compare WFI with a supported preemptible configuration and valid driver-provided buffers, using identical long packets and the same short probe. Exact native statuses and actual configuration must be retained. A repeatable queue-wait reduction establishes preemption relevance to that loaded case; restoration of guest progress in a matched reproducer is additionally needed to connect it to the stall. A synthetic loaded-context result alone does not explain ENS9's idle/HLT state.

For zcull, use a separate future matched experiment with valid buffers and known NO_CTXSW versus SEPARATE_BUFFER state, identical workload and context-switch load. Record GPU work and frame timings independently of stall diagnosis. No such experiment, runtime change or new switch is part of this head.

Source provenance: NVIDIA official 580.159.04 commit `b81d58ee0224d1d290bef1c080592b619e184042`,595.84 tag commit `722ae84526a09ed672fbe75448e2909834ba4cce`; `ctrl2080gr.h`, `ctrl0080fifo.h`, `nvos.h`, `kernel_channel.c`, `kernel_graphics_context.c`, `kernel_graphics.c` and generated export tables. Prior parity audit: `codex/pf1-tsgparity-20261026` commit `5fa2bc567`. ENS9 observations above are attributed to the coordinator's 2026-10-27 update, not an independent PF1 measurement.

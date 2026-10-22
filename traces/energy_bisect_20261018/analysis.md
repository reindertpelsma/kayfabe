# Q2 Energy refusal analysis

STATUS: LIVE investigation, 2026-10-22. CODE READ and experiment limits below; no product fix.

## Immediate refusal and classification

MEASURED at rev afe00b3c, run4262, and rev 40583bc2, runs4263/4266: guest client/channel `0xc1d00019:0xff040004` was born Translated on COPY2 (`engine=0xb`), guest token `0x806`, host `0x1002d`, privilege level2, internalFlags0x16, UVM-owned=false. It dies at GP34 on subchannel4/method576 (0x240). Retirement records forwarded35/submissions1. The bounded context and guest153 times are in each run's result.json/excerpts.txt. Guest timestamps retain their original guest clock; do not equate them with host wall time.

CODE READ: `kf-rm/src/chanlink.rs::kernel_channel` accepts this birth under both old and SA-02 predicates: 0xc1d00019 is outside both internal-client predicates, but the stamp is KERNEL. `ChannelPolicy` emits kernel_client=true; Windows user-work classification keeps the kernel driver's own process Translated. The recorded classification is not a direct SA-02 reclassification.

CODE READ: `kf-qemu/src/chan.rs` builds a successful HostRing and then `TranslatedRing::new_tmode(..., 0)`, followed by `set_gr`. `kf-chan/src/host.rs::on_engine_layout` calls `alloc_ce_object` and schedules the COPY ring before returning success. Therefore the software error name does not establish that host RM failed to allocate a physical CE object.

CODE READ: `ring.rs::new_tmode` creates default TState (tier=None). `set_gr` installs GR/video/software configuration, not a CE tier. `tmode.rs::host(SetObject)` sets tier=Some only for a recognized CE class. `tmode.rs::one` routes methods below0x100 to host methods, then checks software/inert/GR/video alternatives. COPY2/subchannel4/method0x240 reaches the `tier.ok_or(NoCeObject)` check. It refuses before decoding that CE address register. The missing fact is decoder binding state, not an evaluated invalid semaphore address.

CODE READ: the ring reports a Rewrite error; the channel is marked DEAD. The device's operand path returns No for a dead channel and the kernel-owner refusal path poisons it. The NoCeObject check is a named refusal predating these topic branches. `source-identities.json` shows identical birth/decoder/ring/host/refusal source blobs on master, descriptor-only, A, B and candidate.

## Same-input comparison and its limit

CODE READ: `rpc_characterization.rs` reconstructs a minimal root+channel allocation using public ABI fields and recorded birth facts; it is not a complete replay of the original Windows RPC ordering. `ce_characterization.rs` uses synthetic public-ABI packets, never captured guest PB words. It contrasts an initially unbound CE semaphore with an explicit recognized SET_OBJECT followed by a semaphore in a later segment. It characterizes existing behavior and does not assert an unproven desired default binding.

CODE READ applied to the birth records measured at rev 468a35c9, rev afe00b3c and rev 40583bc2 (runs3454/4260/4262/4263/4266/4267): the old and new SA-02 predicates agree on every captured CHAN-FACTS entry; the corpus and per-run counts are in channel-comparison.json. These are evaluations of source predicates over logged facts, not measurements of two executions of an identical complete RPC stream. Non-channel inputs and unrecorded ordering remain outside this comparison.

MEASURED at rev 468a35c9, K1 run3454: the matching host0x1002d channel retired with forwarded0/submissions0. MEASURED at rev afe00b3c, candidate run4267: Energy Completed, score43.64, guest153/4101=0, poison0, fresh host Xids0; this channel also retired forwarded0/submissions0. These clean runs do not exercise the failing GP34 submission on that channel. The candidate Energy-only samples so far are run4262 FAIL and run4267 PASS.

## Causal boundary

INFERRED: VA batch is not necessary for this failure, because A (without it) reproduces twice. An A-only failure pattern alone would not establish SA-02 causality: source predicates agree for the logged births and clean runs omit the failing channel's submission. B run4264 subsequently completed; its channel-submission coverage and the matched descriptor-only control will be incorporated before the final topic verdict.

INFERRED: the immediate NoCeObject cause is an unset software CE tier. Why a required binding is absent is not established: never submitted, lost/skipped parser history, or a shared/implicit engine-context contract are distinct hypotheses. No product fix is proposed. A native same-stream reference and bounded binding-history evidence are needed before deciding whether a birth-time seed, context sharing, or submission-order correction is legitimate. Never infer a binding merely to suppress this refusal.

INFERRED future binding hypotheses, falsifiers stated before further inspection: (1) never submitted — falsified by a recognized CE SET_OBJECT in this channel's complete consumed history before GP34; (2) lost decoder history — falsified by complete history with no such bind plus an explicit-per-channel binding contract; (3) implicit/shared context — falsified by public-source/native controls showing no inherited binding for the same context topology. None is established by the current logs.

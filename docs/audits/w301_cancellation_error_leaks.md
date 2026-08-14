# w301 — cancellation, error reporting, and lifetime: a read-only audit

> ## ★★★★★ TWO OF THIS AUDIT'S FINDINGS WERE ACTED ON — w303, 2026-08-14. Read this before §1.4 or §3.1.
>
> Branch `w303-reap-and-preempt`. Both findings were **re-verified still true at master
> `8658f0f0`** before anything was changed (`git log --all -S reap_retired`, and the
> `INPUT_ONLY_CONTROLS` row read at source) — the currency check this tree keeps paying for.
>
> ### §3.1 — THE REAP IS NOW ARMED
> `Regs::write` (`crates/kayfabe-qemu-raw/src/shim.rs`) calls `SharedDevice::reap_retired()`
> after `materialize_pending` and the engine-forward drain, on the frame where the plane's
> rank-0 guard is a dropped local. That frame is the shim's **GSP re-handshake edge**: the
> guest's free chain, the idle release and the status-queue re-publish all arrive as
> register writes, so it is the L10 quiesce point the core's docs ask the *adapter* to
> declare. Because the edge recurs, a proc deferred for not being quiesced is retried at the
> guest's next MMIO write; no deadline and no thread are needed.
> ★ The missing gate is `crates/kayfabe-qemu-raw/tests/reap_composition_root.rs` — not
> *"the reap works"* (every teardown test already asserted that, and every one of them was
> green while a real boot reaped nothing) but ***"the reap is REACHED from the production
> path"***. Severing the one line turns it red and leaves the rest of the workspace green,
> which is exactly the state master was in.
>
> ### §1.4 — PREEMPT NO LONGER LIES, ⊘ **AND §1.4 PUT IT IN THE WRONG PLACE**
> `0xa06c0105` has **left `INPUT_ONLY_CONTROLS`** (this audit's own recommendation) and is
> answered by `ObjectPolicy::respond_preempt`, which **decides**: `NV_OK` when the named
> group provably has no host twin — nothing has ever reached the GPU, so the preemption is
> vacuously complete and the ack is *true* — and `NV_ERR_INVALID_STATE`
> (`PREEMPT_UNPERFORMED_STATUS`, inside `ctrla06c.h`'s own documented status set, unlike
> `0x56`) when it does.
>
> ⊘⊘ **The correction that matters most: §1.4 says record 457 of `ctx_r1` is *"inside
> `cuCtxCreate`, not teardown"*. It is TEARDOWN.** `[measured 2026-08-14, w303]` decoding
> `../nvidia-gpu-passthrough/traces/host_reference_ga106/ctx_r1.jsonl.zst` puts 457 inside an
> unbroken `RM_FREE` cascade — 450, 451, 453, 456, **457**, 459, 463, 465 — i.e.
> `cuCtxDestroy`; `init_r1`/`dev_r1`, which create no context, never issue it at all. ⇒ the
> severity scoping in §1.4 (*"not a user-driven cancel … the lie is latent"*) is **wrong in
> the unsafe direction**: this is the one control that asks *"is the engine still running out
> of the pages I am about to free?"*, which ties it directly to §3.3's free-after-ring rather
> than to context setup.
>
> ⊘ **And the brief's premise — *"one of four controls I served to get cup2 past its wall"* —
> is refuted for this id.** `[measured 2026-08-14, w303]` `0xa06c0105` appears **nowhere** in
> either crossing boot: not in `traces/w294_cudalimit/run_w294cup2_qemu.log`'s served-control
> census nor its 40-id unserviced ledger, and not in
> `traces/w297_cup3/run_w297cup3_qemu.log.gz`'s either. ★ Known-positive for that zero — the
> siblings `0xa06c010a` (×5), `0xa06c0101` (×3) and `0xa06c0103` (×1) **are** named in both,
> so the census is live. The cause is in the workload: `scripts/bench/cup3.c` calls
> `cuCtxCreate` and never `cuCtxDestroy`. ⇒ **changing this answer cannot move `CUP2_RC` or
> `CUP3_RC`.**
>
> ⚠ **What is still unmeasured, stated rather than buried:** every committed boot in which we
> answered `0xa06c0105` non-`NV_OK` (~15, `run_s45`…`run_w216`, all `0x56`, all followed by a
> clean `FREE` unwind) was a boot where `cuCtxCreate` had already **failed**. A refusal on the
> *successful* `cuCtxDestroy` path has never been observed. The instrument that would settle
> it in one run exists — `scripts/rpctrace/inject_matrix_ctxcreate.sh`, the ladder w275 used
> to prove two other `0x56`s inert on real GA106 — and needs a bench.
>
> ⊘ **Neither change performs a preemption.** The successor is to forward it: the host TSG is
> already reachable (`HostRmBackend::schedule` finds it as `channel_parts(raw).tsg` and issues
> a group control on it), so the verb is that function with a different command id and the
> guest's own params. Not built here; it needs a boot to prove.

**STATUS: LIVE — 2026-08-14.** Read-only. Nothing was built, booted or measured on hardware
for this document; every claim is read off source at `91f8b34b` (kayfabe), the vendored
`ogkm-580.159.04`, the vendored gVisor `nvproxy`, and committed traces. Where a claim needs a
boot to settle, it says so and is not asserted.

Scope: `/workspace/nvkvm-rs` (kayfabe, the product). The C artifact
(`/workspace/nvidia-gpu-passthrough`) appears only as an oracle — and §0.2 records that on this
subject it is **silent**, which is the first thing to know.

⚠ **Driver-version note.** Every `ogkm` citation is **580.159.04** (the pinned host driver).
The 610.43.02 tree was diffed on every load-bearing body and HAL table used here
(`kchannelCtrlCmdStopChannel_IMPL`, `kchannelCtrlCmdResetChannel_IMPL`,
`subdeviceCtrlCmdFifoDisableChannels_IMPL`, the `kfifoStartChannelHalt`/`kfifoCompleteChannelHalt`
chip conditions, `krcErrorSendEventNotificationsCtxDma → _FWCLIENT`,
`kgmmuServiceChannelMmuFault → _92bfc3`): **byte-identical or semantically identical**. Only
key-rotation plumbing was refactored. No finding here has a version seam.

Verdict vocabulary, used exactly, never blurred:

- **BUILT + REACHABLE** — exists, and a guest action reaches it (call site cited).
- **BUILT + ORPHANED** — exists, nothing in production calls it.
- **NOT BUILT** — no such code.

★ **"Built" is not "reachable" and "reachable" is not "correct."** Three of this audit's top
findings are orphans, and one is a verb that is reachable, returns `NV_OK`, and performs nothing.

---

## 0. Two framing facts, both of which bound everything below

### 0.1 We forward doorbells to a real GPU, and the forward is unretractable

A guest doorbell becomes a real store into the host GPU's usermode window on the vCPU thread,
synchronously:

`crates/kayfabe-qemu-raw/src/shim.rs:4555` (`SharedDoorbell::ring`, the one production
`DoorbellPort`) → `crates/kayfabe-rt/src/device.rs:2359` → `crates/kayfabe-isolate/src/lib.rs:2850`
(`VerbPlan::Doorbell`) → `crates/kayfabe-isolate-host/src/rm.rs:4741` → `:1600-1603`, a literal
`store_u32`.

It is not queued: the executor inbox (`crates/kayfabe-rt/src/executor.rs:79-108`) carries
`SourceSignal` / `Deferred` / `IsolateComplete` and has no doorbell variant. **Once that store
executes there is no handle by which it can be retracted**, and no `RmBackend` verb exists that
could ask the GPU to stop (full trait surface: `crates/kayfabe-isolate/src/lib.rs:764-1219` —
`alloc*`, `schedule`, `free`, `control`, `map/unmap_gpu_va`, `ring_doorbell`, `ce_copy`, `fb_*`,
`*_guest_ram`; no stop, no preempt, no idle).

The word "cancel" in this tree means something else, and the suite says so at
`tests/tests/cancellation.rs:19-30`: *"'cancel' can never mean 'interrupt the host ioctl' — it
means deliver a break signal and find out."* The cancellation seam
(`crates/kayfabe-isolate/src/lib.rs:1347-1439`, `CancelReason`/`CancelSink`/`CancelHandle`) is a
well-built mechanism for **abandoning our own in-flight host ioctls**. It has nothing to do with
GPU work, and conflating the two is the single easiest mistake to make in this area.

### 0.2 ⊘ There is NO C oracle for any of this

The C research artifact runs **exactly one CUDA process per QEMU lifetime**
(`docs/reference/mode2_bench_lifecycle.md` §1, measured, reproduced across three boots). It
therefore never exercised a second teardown, and it handles none of the cancellation verbs:
`grep -i 'STOP_CHANNEL|RESET_CHANNEL|DISABLE_CHANNELS|PREEMPT'` over
`C: src/qemu/nvkvm_gpu_emul.c` and `src/stub/*.c` returns only `NVA06C_CTRL_CMD_BIND` and one
prose use of the word "preempts" about the BQL.

⇒ **Every finding below is unoracled by the C.** Where this audit says "we do X and NVIDIA does
Y", Y comes from `ogkm` with its HAL binding resolved, never from the C.

What the C *does* supply is the shape of guest teardown, and it is load-bearing for §3: on a
CUDA process's death the guest kernel issued **178 `fn=10` RM-FREE RPCs, then fn-47**, with no
application cooperation (`mode2_bench_lifecycle.md` §5). **The guest kernel is the garbage
collector, and it tells us.** Everything in §3 is about what we do with that message.

---

## 1. Part A — the cancellation census

### 1.1 The default arm, because it decides most rows

There is no single control `match`. There is a three-stage funnel:

1. **Chain assembly** — `crates/kayfabe-device/src/lib.rs:1101` `served_chain()`, a
   `Vec<Box<dyn CommandPolicy>>` tried in order (`find_map`, first `Some` wins —
   `crates/kayfabe-gsp/src/boot.rs:640`). `ObjectPolicy` is seated at `lib.rs:1246`; the terminal
   link is `UnservicedLedger` at `lib.rs:1252`.
2. **The control gate** — `crates/kayfabe-rmrpc/src/policy.rs:2037`:
   `if !OBJECT_CONTROLS.contains(&req.cmd) { return None }`. The list is
   `policy.rs:1731-1823`. Then a `match` with seven explicit arms, one table-lookup arm
   (`policy.rs:2068`), and `_ => None` at `policy.rs:2071`.
3. **The terminal default** — `crates/kayfabe-gsp/src/boot.rs:1526-1532`: record in the ledger,
   reply `NV_ERR_NOT_SUPPORTED` (`0x56`) with an empty body.

**The default is a named refusal with a ledger entry — not a silent `NV_OK`, not an echo.** That
is the right default and it is why most rows below are honest failures rather than lies.

⚠ **`capability.rs`'s `CONTROLS` allowlist is NOT on this path.** The capability gate lives in
`crates/kayfabe-rmrpc/src/lib.rs:1544-1550` (`translate_control`), which `ObjectPolicy::respond`
never reaches for `RmControl` (early return at `policy.rs:2943`). So an allowlist row for
`RESET_CHANNEL` changes nothing about the answer. This is the tree's own named class —
*"ADMITTED and SERVED are different gates"* — and three cancellation verbs sit in that gap today.

### 1.2 The table

| verb (id, ogkm header:line) | guest issues it? | kayfabe today | what happens now | verdict |
|---|---|---|---|---|
| **`NVA06F_CTRL_CMD_STOP_CHANNEL`** `0xa06f0112` (`ctrla06fgpfifo.h:237`) | **Yes, on every UVM channel teardown.** `nvGpuOpsStopChannel` → `pRmApi->Control(..., STOP_CHANNEL)` at `ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:11194`, reached from `nvUvmInterfaceStopChannel` ← `kernel-open/nvidia-uvm/uvm_user_channel.c:788`. **Measured arriving at us: present in the distinct-unserviced ledger of all 25 captured boots** under `traces/boots/` (see §1.3 for why "25 boots", not an arrival count) | zero mentions anywhere in `crates/` (0 grep hits for the id or the name) | `policy.rs:2037` → `None` → ledger → **`0x56`, empty body**. Our channel state is untouched: the channel stays in `exec.requested` until a `FREE` arrives | **NOT BUILT** |
| **`NV906F_CTRL_CMD_RESET_CHANNEL`** `0x906f0102` (`ctrl906f.h:138`) | Yes, on error paths: MMU-fault recovery (`kern_gmmu_gv100.c:2082`, reason `_MMU_FLT`) and the RC watchdog (`kernel_rc_watchdog_callback.c:91`). Never observed reaching us | allowlist row only — `crates/kayfabe-abi/src/capability.rs:859` | **`0x56`** (the allowlist row is on a path this control never takes) | **NOT BUILT** (admitted ≠ served) |
| **`NVA06C_CTRL_CMD_PREEMPT`** `0xa06c0105` (`ctrla06c.h:203`) | **Yes, from libcuda.** Verified first-hand: exactly **1 occurrence per run** in `../nvidia-gpu-passthrough/traces/host_reference_ga106/{ctx,ce,launch,alloc}_r1`, **0** in `init_r1`/`dev_r1`; in `ctx_r1` it is record **457 of 608** — i.e. inside `cuCtxCreate`, not teardown. Also 1× in each of `traces/nvdiff_w292/{both,drain,serve}_r1` | **SERVED as an input-only echo.** Row at `crates/kayfabe-abi/src/submit.rs:4515-4524`; claimed at `policy.rs:1808`; dispatched at `policy.rs:2068` → `respond_input_only` (`policy.rs:2115`) | size/serialization check, then **`NV_OK` with the guest's own payload echoed back** (`policy.rs:2135-2138`). **Nothing is preempted.** Malformed → `0x47` | **BUILT + REACHABLE** — see §1.4, this is the one that lies |
| **`NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`** `0xa06f0103`, `bEnable=false` | enable form issued by the CeUtils scrubber; disable form has no in-tree kernel caller but a guest may send it | **SERVED** — `policy.rs:2047` → `respond_gpfifo_schedule` (`:2585`) → `Gpu::schedule_channel` (`gpu.rs:4865`) → `apply_schedule_channel` (`gpu.rs:667`), where `enable=false` is **`proc.exec.requested.remove(&route.chan)`** (`gpu.rs:671`) | guest-plane deschedule is **real**: `plan_doorbell` (`crates/kayfabe-fwd/src/lib.rs:3432-3437`) then refuses future doorbells `FwdFault::NotScheduled`. ⚠ **The withdrawal never reaches the host**: `RmBackend::schedule` has no enable flag (`crates/kayfabe-isolate/src/lib.rs:887`) and hardcodes `b_enable: 1` (`crates/kayfabe-isolate-host/src/rm.rs:4593`) | guest plane **BUILT + REACHABLE**; host plane **NOT BUILT** |
| **`NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`** `0xa06c0101`, `bEnable=false` (TSG) | Yes — `cuCtxCreate` issues the enable form 3×/run in every nvdiff trace | **SERVED** — `policy.rs:2050` → `respond_gpfifo_schedule_group` (`:2664`) → `apply_schedule_group` (`gpu.rs:870`), `remove` at `gpu.rs:881` per member | as above, fanned over the group | guest plane **BUILT + REACHABLE**; host plane **NOT BUILT** |
| **`NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS`** `0x2080110b` (`ctrl2080fifo.h:339`) | **Yes (UVM).** `nv_gpu_ops.c:1071` (`bDisable=TRUE`) and `:1129` (`FALSE`). Never observed reaching us | allowlist row only — `capability.rs:784` | **`0x56`**; `nvGpuOpsDisableChannels` fails; nothing in our `ExecPlane` moves | **NOT BUILT** |
| **`NV2080_CTRL_CMD_FIFO_DISABLE_USERMODE_CHANNELS`** `0x20801117` | Yes, on suspend/resume — `rm_disable_user_channels` / `rm_restart_user_channels` (`osapi.c:2792`, `:2821`) | zero mentions; not even allowlisted | **`0x56`**; guest suspend path fails | **NOT BUILT** |
| **`NV0080_CTRL_CMD_INTERNAL_FIFO_RC_AND_PERMANENTLY_DISABLE_CHANNELS`** `0x00802008` (`ctrl0080internal.h:106`) | RM-internal; not observed | zero mentions. ⚠ Its immediate neighbours `0x00802004`/`0x00802009` **are** served (`submit.rs:4529`, `:4547`) — the miss is a hole in an otherwise-covered range | **`0x56`** | **NOT BUILT** |
| **`NV2080_CTRL_CMD_RC_*` family** (`ctrl2080rc.h`: `0x20802206`, `…07`, `…0a`, `…0b`, `…0c`, `0x20802210`) | Partly: `0x2080220c` and `0x20802210` appear 1× each, `status=0x0`, in all three nvdiff runs — answered **inside the guest's own `nvidia.ko`**, never RPC'd to us | three allowlist rows (`capability.rs:811-813`); no `match` arm for any | today they do not reach us at all; if one ever did → **`0x56`** | **NOT BUILT** |
| **`RC_TRIGGERED` outbound event** (`rpc_rc_triggered_v17_02` — the actual RC mechanism under GSP-RM) | n/a, we would be the sender | `crates/kayfabe-rmrpc/src/fault.rs:165` `rc_triggered_for` + `:102` `FaultEmission::deliver`, re-exported `lib.rs:197` | **verified first-hand: every caller is `tests/tests/simulated_fault.rs` (167, 368, 408, 483, 670, 767, 888).** We can never tell a guest its channel died — except through the w288 notifier (§2.1) | **BUILT + ORPHANED** |
| **`NV01_FREE` (fn 10)** on a channel / TSG / VASpace | **Yes, constantly** — it is the entire teardown path | **SERVED** — `policy.rs:1689` (`OBJECT_VERBS`) → `translate_free` (`crates/kayfabe-rmrpc/src/lib.rs:1943`) → `RmEvent::Free` → `RmGraph::free_subtree` (`crates/kayfabe-core/src/rmgraph.rs:2008`) → `Spine::refresh` | `NV_OK` + echo. Cancellation is modelled *indirectly*: `gpu.rs:3186-3199` drops the freed channel from `exec.scheduled`/`requested`/`forwarded`, so it stops being forwardable. Host objects are **staged**, not freed synchronously — and §3.1 is about what happens to that staging | **BUILT + REACHABLE** |

Not orphaned, checked explicitly: `decode/encode_gpfifo_schedule` (`submit.rs:691`, `:739`),
`decode/encode_ctxsw_preemption_mode` (`:2743`, `:2781`), `input_only_control` (`:4601`) all have
production callers in `impl ObjectPolicy` (`policy.rs:2605, 2624, 2684, 2705, 2463, 2499, 2068,
2120`), no `#[cfg(test)]`.

### 1.3 ⊘ A correction to the instrument, made while checking this

An earlier draft of this census reported *"95 arrivals of `0xa06f0112` across 25 boot logs"*.
**That number is not supported and is withdrawn.** The unserviced ledger prints a **distinct set,
once, at teardown**, capped at `UNSERVICED_SAMPLE_MAX`
(`crates/kayfabe-device/src/unserviced.rs:171-189`); `grep -c` over the logs counts one line per
distinct id per boot. The defensible statement is: **`0xa06f0112` is in the distinct-unserviced
set of all 25 captured boots** — the guest asks on every boot and is refused on every boot. The
aggregate arrival count exists per boot but not per id (e.g.
`traces/boots/w268/run_w268_refuse_qemu.log:1285` — *"843 decoded, 111 UNSERVICED …, 41
distinct"*).

★ Same class the tree already names: a census that reports *distinct* cannot answer *how many*,
and reading one as the other invents a measurement.

### 1.4 ★★★★ The one verb that lies: `NVA06C_CTRL_CMD_PREEMPT`

Everything else unserved is refused loudly. This one is answered `NV_OK` and does nothing, and
the justification in the source is:

> *"`SET_TIMESLICE` and `PREEMPT` are scheduler hints to a runlist we do not schedule."*
> — `crates/kayfabe-rmrpc/src/policy.rs:2109-2110`

⊘ **That premise is no longer true of this port.** We do materialize a host TSG and a host
channel for the guest's channel, and we *do* schedule it: the isolate issues
`NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` on `parts.tsg` (`crates/kayfabe-isolate-host/src/rm.rs:4593`,
`b_enable: 1`), and `docs/design/gpfifo_schedule.md` states the split in its own words —
*"guest asks `a06f` on a channel; host is told `a06c` on a group."* So a guest `PREEMPT` names a
TSG that has a live, scheduled host twin executing on a real GA106, and we tell the guest the
preemption succeeded.

★★★ **And the HAL resolution makes this worse than "we ignore a hint we could have ignored."**
`NVA06C_CTRL_CMD_PREEMPT`'s export entry is `ogkm-580:
src/nvidia/generated/g_kernel_channel_group_api_nvoc.c:267-280`, `flags = 0x10248` — which
**includes `ROUTE_TO_PHYSICAL` (0x40)** and does *not* include
`PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST`. By `NVOC_EXPORTED_METHOD_DISABLED_BY_FLAG`
(`src/nvidia/inc/kernel/rmapi/control.h:159-162`) the CPU-RM `_IMPL` is therefore replaced by
`NULL` in the export table, and `grep -rn CtrlCmdPreempt src/ --include=*.c | grep -v generated`
returns **nothing** — the body does not exist in the open tree at all. ⇒ **On a GSP client the
entire implementation of `PREEMPT` lives behind the RPC, i.e. in the GSP.** *We are the GSP.*
The guest's own kernel does nothing but forward it to us and return whatever we say.

⇒ **There is no other party that could perform this control, and no NVIDIA code path that
compensates for our not performing it.** The `bWait`/`bManualTimeout`/`timeoutUs` semantics
(`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrla06c.h:179-213`, max 1 s) were ours to implement.
This is not an inert hint; it is a delegated verb we accepted and dropped.

The row's own authority text already flags the weakness: *"⚠ NATIVE GA106 ONLY (NV_OK @457).
⊘ The C is SILENT here … That is an ABSENCE OF EVIDENCE, not evidence of refusal"*
(`submit.rs:4521-4523`). The `InputOnlyControl` doc states the discipline this row breaches:
*"a row here is a claim that the ack is complete, not a parking space"* (`policy.rs:2113`).

**Severity scoping, measured rather than assumed.** On the only workload we have traced,
`0xa06c0105` is issued **once, at `cuCtxCreate`** (record 457 of 608 in `ctx_r1`), not as a
user-driven cancel — so today's `cup2`/`cup3` ladder is not exercising the lie. The lie is
latent, and it becomes live the moment any guest uses `PREEMPT` for what it is for.

⇒ **Recommendation (not applied — this is a read-only audit): `0xa06c0105` should leave
`INPUT_ONLY_CONTROLS`.** Either it forwards (there is a host TSG to forward to), or it refuses by
name. `SET_TIMESLICE` (`0xa06c0103`) is genuinely inert and may stay.

### 1.5 ★★★ What NVIDIA actually does — HAL-resolved for GA106 + GSP client

Both vendored trees were checked (`ogkm-580.159.04` = the pinned host driver;
`ogkm` = 610.43.02). **They differ in version but in none of the answers below** — the
load-bearing bodies and HAL tables are byte-identical or semantically identical; only
key-rotation plumbing was refactored. Citations are 580.159.04.

Two dispatch mechanisms decide what runs, and the second is a kind of "the `.c` is not the code"
this campaign had not yet named:

1. **nvoc chip/variant dispatch** (`src/nvidia/generated/g_*_nvoc.c`). GA106 sits in the mask
   `0xf1f0fc00` at `chipHal_HalVarIdx>>5 == 1` (`g_kernel_fifo_nvoc.c:919-948`).
2. ★ **RMCTRL flag-driven compile-out** — `src/nvidia/inc/kernel/rmapi/control.h:159-162`: if a
   control carries `ROUTE_TO_PHYSICAL` (0x40) without `PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST`, its
   CPU-RM `_IMPL` is replaced by **`NULL`** in the export table and the control is auto-forwarded
   to GSP. **The body exists in the tree and is never linked in.**

| verb | CPU-RM on GA106+GSP | binding evidence |
|---|---|---|
| `STOP_CHANNEL` | `_IMPL` **runs** (flags `0x10008`, no `ROUTE_TO_PHYSICAL`) — and it is *one RPC plus one notifier write*: `NV_RM_RPC_CONTROL` (`kernel_channel.c:1960`), then `kchannelNotifyRc_HAL` (`:1979`) → `krcErrorSetNotifier` → `krcErrorWriteNotifier_CPU` | `g_kernel_channel_nvoc.c:389-397`; `_HAL`s resolve `_IMPL`/`_CPU` unconditionally (`g_kernel_channel_nvoc.h:942`, `g_kernel_rc_nvoc.h:213`) |
| `RESET_CHANNEL` | `_IMPL` runs (flags `0x10008`); body's own comment: *"All real hardware management is done in the host. Do an RPC to the host"* (`kernel_channel.c:3058-3061`). ⊘ It does **not** clear the CPU-RM MMU-fault shadow — only `kchannelDestruct_IMPL:1261` does | `g_kernel_channel_nvoc.c:273` |
| `PREEMPT` | **compiled out to `NULL`** — pure RPC to GSP; no body in the open tree | `g_kernel_channel_group_api_nvoc.c:267-280`, flags `0x10248` |
| `GET_MMU_FAULT_INFO` | **compiled out to `NULL`** — GSP-only | `g_kernel_channel_nvoc.c:295-303`, flags `0x10048` |
| `GPFIFO_SCHEDULE` | `_IMPL` runs (flags `0x10008`) — and ⊘ **it does not branch on `bEnable` at all**; it asserts schedulability, sets the runlist-set flag, and RPCs (`kernel_channel.c:3105-3131`) | `g_kernel_channel_nvoc.c:~310-322` |
| `FIFO_DISABLE_CHANNELS` | `_IMPL` runs (flags `0x10108`); 100 % pass-through RPC on a GSP client, `NV_ERR_NOT_SUPPORTED` otherwise (`kernel_fifo_ctrl.c:725-750`) | `g_subdevice_nvoc.c:4921-4923` |

**Two consequences for us, both concrete:**

- ★ **Our `0x56` to `STOP_CHANNEL` suppresses the guest's own RC notifier write.**
  `kchannelCtrlCmdStopChannel_IMPL` returns at `kernel_channel.c:1961` on a non-`NV_OK` RPC and
  never reaches `kchannelNotifyRc_HAL` at `:1979`. So refusing this verb does not merely fail to
  stop a channel — it also removes the one notification the guest's *own* CPU-RM would have
  written for a stopped channel.
- ★★ **RC recovery: GSP writes the notifier; CPU-RM only sends events.** `_kgspRpcRCTriggered`
  (`kernel_gsp.c:547`) calls `krcErrorSetNotifier` **only when Confidential Compute is enabled**
  (`:661-671`); on a normal GA106 the comment at `kernel_rc_notification.c:352-358` says GSP
  already wrote it. `krcErrorSendEventNotificationsCtxDma_HAL` resolves unconditionally to
  **`_FWCLIENT`** (`g_kernel_rc_nvoc.h:280`), which writes **no** notifier memory — it only calls
  `notifyEvents`. ⇒ **In our architecture the RC notifier write is ours**, and w288's passthrough
  delegates it to the host RM. That is a legitimate substitution — **but only for channels that
  have a host twin**, which is why §2.1's `hObjectError = 0` sites and §1.2's orphaned
  `RC_TRIGGERED` matter.

⊘ **Also dead on GA106+GSP, checked so nobody re-derives it:** `kchannelFwdToInternalCtrl_HAL` →
`_56cd7a` (`return NV_OK`); `kfifoRecoverAllChannels_HAL` → `_92bfc3`
(`ASSERT(0); NOT_SUPPORTED`) for the PF; `krcErrorInvokeCallback_IMPL`'s only caller is
`vgpu_events.c:193`, so the whole RC client-action state machine is unreached bare-metal; and
`kern_gmmu_gv100.c`'s reset-channel block is dead **not** because of the chip HAL (GA106 →
`_GA100`, which delegates to the `_GV100` body) but because the block is inside
`if (IS_VIRTUAL_WITH_SRIOV(pGpu))` at `:2052`, after which `kgmmuServiceChannelMmuFault_HAL`
resolves to `_92bfc3`. ⇒ **CPU-RM's MMU-fault path terminates in `NV_ERR_NOT_SUPPORTED` and
performs no channel recovery.** ⚠ This *refines* the sibling lane's ruling (which cited
`:2127` as dead): the line is dead, but the reason is the SRIOV guard, not the chip binding.

### 1.6 What gVisor does, for comparison

`nvproxy` allows `RESET_CHANNEL` (0x906f0102, `version.go:338`), both `GPFIFO_SCHEDULE` forms
(`:366`, `:370`) and `PREEMPT` (`:369`) through as `rmControlSimple` — flat structs, straight
through, no reasoning about in-flight work anywhere (`grep -i 'danger|in-flight|quiesc'` over
`pkg/sentry/devices/nvproxy/*.go` returns nothing). `FIFO_DISABLE_CHANNELS` gets a dedicated
handler (`frontend.go:1036-1058`) whose only added logic is enforcing that
`pRunlistPreemptEvent` is NULL. ⊘ **`STOP_CHANNEL` (0xa06f0112) and `GET_MMU_FAULT_INFO`
(0x906f0106) are not present at all** and fall through to `NV_ERR_NOT_SUPPORTED`
(`frontend.go:804-814`) — i.e. gVisor has the same hole we do on `STOP_CHANNEL`.

⚠ It is **not** authority for us, and the difference is the whole point of the admitted/served
split: nvproxy is a passthrough sandbox with a real driver underneath, so "allow" means *the real
RM performs it*. Our ids are answered by an emulated GSP, so allowing an id means **we** must
perform it. Every `origin: Origin::Nvproxy` row in `crates/kayfabe-abi/src/capability.rs`
inherits that distinction. (Also: nvproxy's newest 580-branch ABIs are `580.126.09`/`580.126.20`,
so it would not attach to our pinned `580.159.04` as-is.)

---

## 2. Part B — error reporting

### 2.1 The w288 notifier passthrough — **BUILT + REACHABLE**

The chain from a guest action to the host RM ioctl:

guest `NV_CHANNEL_ALLOC_PARAMS` → `crates/kayfabe-rmrpc/src/lib.rs:1376`
(`decode_channel_error_notifier`) → `AllocFacts` (`crates/kayfabe-core/src/rmgraph.rs:417`) →
`ChannelFacts` (`crates/kayfabe-core/src/project.rs:1305`) → `Channel::error_notifier`
(`crates/kayfabe-core/src/gpu.rs:3161`) → `err_notifier_grant()`
(`crates/kayfabe-qemu-raw/src/shim.rs:2867`, 4 KiB `ReadWrite`, refusing `Unreachable`/misaligned
**by name**) → the two guest-reachable call sites, the doorbell MMIO trap
(`shim.rs:4899-4909`, inside `impl DoorbellPort for SharedDoorbell::ring`) and the engine-object
drain (`shim.rs:12321` → `device.rs:1058-1082`) → the birth arms
(`crates/kayfabe-isolate/src/lib.rs:2872`, `:2996`) → `describe_err_notifier` (`:3253`) →
`h_object_error` in `alloc_channel_in` (`crates/kayfabe-isolate-host/src/rm.rs:6081`) → the ioctl
at `rm.rs:1821`, status-checked at `:1824`.

This is the one error-reporting surface that reaches a real guest consumer today: `nvidia-uvm`
reads `error_notifier->status` at slot 0 and nothing else, and that word is now written by the
**host** RM directly into the guest's page.

**Is it on every channel-alloc path?** Of the five `alloc_channel(` call sites, three pass a
notifier (`isolate/lib.rs:2895`, `:3013`, `isolate-host/child.rs:621`); two are the diagnostic
`rmladder` binary. But at the **plan** seams — where this tree's measured systemic-omission
pattern lives — of four seams only two can ever set it:

- ✅ `plan_doorbell` (`crates/kayfabe-fwd/src/lib.rs:3384`), ✅ `plan_engine_object` (`:3980`)
- ❌ `handle_doorbell` (`:3703`) — hard-coded `None`, no production caller, structurally
  underivable there
- ❌ `exec_engine_object` (`:3936`) — hard-coded `None`, **and it has a live caller in the bare-`Gpu`
  reference model** (`crates/kayfabe-rmrpc/src/policy.rs:923`). The QEMU shim uses `SharedDevice`,
  so this is test-only today — but it is a **differential hole**: the reference model births host
  channels with `hObjectError = 0` while the shell births them over the guest's pages.
- ❌ `SharedDevice::forward_engine_object` (`crates/kayfabe-rt/src/device.rs:4145`) — takes the
  grant; its own doc records zero production callers.

**One true omission, guest-reachable:** `alloc_channel_for_isolate`
(`crates/kayfabe-isolate-host/src/rm.rs:5337`) → `alloc_channel_in(.., None)`, used by
`ce_channel` (`:6698`) — the isolate's own copy-engine channel, born on a guest CE doorbell,
always with `hObjectError = 0`. **Nothing can ever be told if that channel RCs.**

### 2.2 ★★★ The slot split is VIOLATED — we cause the host RM to write the guest's slot 1

ogkm's split, confirmed:
`ogkm: src/common/sdk/nvidia/inc/nvos.h:2854-2857` — `_TYPE_ERROR = 0`,
`_TYPE_WORK_SUBMIT_TOKEN = 1`, `_TYPE_KEY_ROTATION_STATUS = 2`, `__SIZE_1 = 3`.

Our side mirrors slot 0 only (`crates/kayfabe-abi/src/notifier.rs:90`,
`NOTIFICATION_TYPE_ERROR_INDEX = 0`); there is no constant, offset or code for slot 1 anywhere.

**We never write the page ourselves** — `describe_err_notifier` maps and describes, and the
isolate deliberately does no read-back and no write (`rm.rs:5395`: a guest-backed `OS_DESCRIPTOR`
cannot be CPU-mapped). `zero_notifier` (`rm.rs:5784`) zeroes a whole page but is called only from
`probe_guest_reachability`, i.e. only from the hand-run `rmladder` binary, and only on a
host-allocated notifier.

**But we make the host RM write it.** Verified first-hand end to end:

1. `crates/kayfabe-isolate-host/src/rm.rs:6081` — the channel is born with `h_object_error` =
   the guest's pages, `flags: 0` (`:6094`), so `notifyIndex[]` keeps its defaults.
2. `crates/kayfabe-isolate-host/src/rm.rs:6168-6172` — **immediately after the bind**, we issue
   `NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN` on that channel.
3. `ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:3348` — that control's last
   statement is `return kchannelNotifyWorkSubmitToken(pGpu, pKernelChannel, token);`,
   **unconditional on the host path** (`bIsVgpuRpcNeeded` is false, `bGenerateWorkSubmitToken` is
   true, for a bare-metal non-MODS host).
4. `kernel_channel.c:4085-4092` — `kchannelNotifyWorkSubmitToken_IMPL` takes
   `index = notifyIndex[_TYPE_WORK_SUBMIT_TOKEN]` (= 1) and calls
   `kchannelUpdateNotifierMem(pKernelChannel, index, token, 0, notifyStatus)`.
5. `kchannelUpdateWorkSubmitTokenNotifIndex_IMPL` confirms the memory it means:
   `hNotifier = pKernelChannel->hErrorContext` — **the same object we handed it**, i.e. the
   guest's pages. The size gate is `>= (index+1)*16 = 32` bytes; our grant is
   `ERROR_NOTIFIER_GRANT_BYTES = 0x1000` (`shim.rs:2843`), so it passes.

⇒ **The host RM writes 16 bytes at offset `0x10` of the guest's notifier page, carrying a
host-space work-submit token and `status = 0xFFFF`.** Slot 1 belongs to the guest's own CPU-RM in
our architecture (the guest is not `IS_VIRTUAL`; its `0xc36f0108` carries flags `0x10008`, is
answered locally, and writes its own slot 1). Ordering makes it worse rather than better: the host
channel is born **lazily at first doorbell**, i.e. after the guest already wrote its own value, so
we overwrite it.

⚠ **Stated as a verified mechanism, not as a measured guest consequence.** Which guest consumer
reads slot 1, and what it does with a host-space token, has **not** been measured — `nvidia-uvm`
reads slot 0. This needs a boot to settle and must not be reasoned about further. It is listed
high because the write is certain and it is into memory that is not ours.

**The other half of the slot-1 gap:** the *internal*-channel variant of `0xc36f0108`
(flags `0x10244`, `ROUTE_TO_PHYSICAL`) does RPC to us, and `0xc36f0108` is **not** in
`OBJECT_CONTROLS`. `encode_work_submit_token` (`crates/kayfabe-chips/src/ga10x.rs:525`) exists
with no production caller — **BUILT + ORPHANED** — so for internal channels we neither answer the
control nor fill slot 1; it falls to the ledger as `0x56`.

### 2.3 Tier-2 relay `NV906F_CTRL_CMD_GET_MMU_FAULT_INFO` — **BUILT + REACHABLE, and correctly on-demand**

This was the owner's explicit worry, and it checks out.

- **Production caller:** `policy.rs:2063-2065` dispatches `respond_get_mmu_fault_info`
  (`policy.rs:2818`) from `respond_control`, gated on `OBJECT_CONTROLS` membership
  (`policy.rs:1758`).
- **One-to-one and verbatim:** the guest's own params bytes are copied out
  (`policy.rs:2856`), handed to `relay_channel_control` (`:2858` → `device.rs:4514` →
  `route_control` → `RmBackend::control` → `rm.rs:2041`, the ioctl at `:2061`), and the host's
  answer comes back in place. `MmuFaultInfoParams::decode` at `:2882` runs **only** to build the
  log line, after the bytes are committed. `is_case2_control` is `false` for every cmd on GA10x
  (`crates/kayfabe-chips/src/ga10x.rs:280`), so it is never silently ACK-only on the production
  arch.
- **No eager read exists.** ★ *Known-positive for this zero:* the grep that would find one is
  `grep -rn 'get_mmu_fault_info\|NV906F_CTRL_CMD_GET_MMU_FAULT_INFO\|0x906f0106' crates/
  --include=*.rs`, and it **does** find a second issuer —
  `crates/kayfabe-isolate-host/src/rm.rs:7176`, `self.get_mmu_fault_info(c).ok()` inside
  `probe_guest_reachability` (`:6934`). So the search is proven live. I traced that caller
  myself: `probe_guest_reachability`'s only caller is
  `crates/kayfabe-isolate-host/src/bin/rmladder.rs:2389`, a hand-run diagnostic binary, on a
  channel it allocated itself, once, after its own wait expired. **No fault-time, boot-time,
  poll-loop, trace or production-test caller exists.**
- ★ **And serving it at all is architecturally required, not optional.** `GET_MMU_FAULT_INFO`'s
  export entry is `ogkm-580: g_kernel_channel_nvoc.c:295-303`, flags `0x10048` — **with
  `ROUTE_TO_PHYSICAL`** — so `kchannelCtrlCmdGetMmuFaultInfo_IMPL` is compiled out to `NULL` and
  there is no such body anywhere under `src/nvidia/src/`. The control is serviced entirely by the
  GSP, and we are the GSP. (Same structural position as `PREEMPT` in §1.4 — with the opposite
  outcome, which is what makes the pair worth reading together.)
- **Refusal is by name, never `0x56`:** `MMU_FAULT_INFO_BAD_PARAMS_STATUS = 0x1f`,
  `MMU_FAULT_INFO_REFUSED_STATUS = 0x40` (`submit.rs:1207`, `:1214`), both with an empty body.
  The source keeps them apart deliberately (*"you asked wrongly"* vs *"we could not"*).

### 2.4 ⊘ The status-layer trap — the wrapper is right; the seams above it flatten

Every NV RM ioctl in the workspace is in one file (`crates/kayfabe-isolate-host/src/rm.rs`, 12
`.ioctl(` sites); `NV_ESC_*` appears nowhere else executable. **Eleven of twelve check the
in-struct status** via `status_check` (`rm.rs:908`), which maps `0 → Ok` and preserves anything
else as `RmError::Other(status)`. The exception and the flattenings:

1. ⊘ **`rm.rs:2780` `read_version`** (`NV_ESC_CHECK_VERSION_STR`) — `ctl.ioctl(..).ok()?`. The
   `reply` word at +4 is never read; only the string is. **A driver that answered `reply != 0`
   while filling a plausible string passes `host_version_gate` (`:2769`) invisibly.** This is the
   one true unchecked-in-struct-status site, and it is exactly the measured class.
2. **`policy.rs:2874`** — `refuse(MMU_FAULT_INFO_REFUSED_STATUS)` collapses `ChannelGone`,
   `NoHostChannel`, `ClassifiedAckOnly` and `HostRefused(RmError::Other(<real host NV_STATUS>))`
   into a constant `0x40`. The variant is printed to QEMU stderr; **the guest is told a status
   the host never said.** Same shape at `device.rs:4575` and `policy.rs:903`.
3. **`policy.rs:2574`** — `rpc_result: NV_ERR_NOT_SUPPORTED` for *every* `BridgeRefusal` variant;
   the file's own comment at `:2549` flags it.
4. **`crates/kayfabe-isolate/src/lib.rs:3269`** — `let _ = rm.unmap_guest_ram(mapped)` on the
   notifier-descriptor failure path: if RM refuses the unmap, the guest's pages stay pinned and
   nothing counts it.

⚠ **Applying the trap to w288's own headline:** `alloc_gpfifo_channel` → `raw_alloc` **does**
`status_check`, so a host refusal of the notifier-bearing channel *is* counted — which is what
makes w288's `REFUSED=0` a real zero rather than an invisible one. But there is **no host-side
read-back of the descriptor by construction** (`rm.rs:5395`, `isolate/lib.rs:2905`). ⇒ the
strongest available claim is *"RM accepted the handle"*. **"RM will actually write those pages"
is not verified from our side by anything.**

### 2.5 The rest of the error-reporting surface

| surface | verdict | evidence |
|---|---|---|
| generic event delivery (`NV01_EVENT_OS_EVENT` → `POST_EVENT` → GSP stall vector) | **BUILT + REACHABLE** | `crates/kayfabe-device/src/plane.rs:3581` `deliver_os_events`, called at `:3526` at the end of every claimed GSP register write → `crates/kayfabe-gsp/src/boot.rs:1697` → `:1648` → `cpu_intr.latch(gsp_stall_vector)`. ⊘ The old C finding (`IrqRaise==1`, zero `IRQSCLR`) is **not** the Rust state |
| `EVENT_SET_NOTIFICATION` arming | **BUILT + REACHABLE, allow-listed to two indices** | `crates/kayfabe-abi/src/eventnotify.rs:331` `SILENT_NOTIFIERS` = `POWER_RESUME (194)` only; `:361` `DELIVERED_NOTIFIERS` = `FIFO_EVENT_MTHD (35)` only. Everything else `0x56` |
| RC error events / `NV2080_NOTIFIERS_RC_ERROR` subscription | **NOT BUILT** | the constant does not exist in the tree; an RC-error arming is refused `0x56` by the two-list gate — a named refusal, not a silent default |
| `RC_TRIGGERED` emission | **BUILT + ORPHANED** | `crates/kayfabe-rmrpc/src/fault.rs:165`, `:102`; the gatekeeper `crates/kayfabe-core/src/fault.rs:277` likewise. Tests only |
| our own `ErrorNotification` write into guest RAM | **BUILT + ORPHANED — by design** | `crates/kayfabe-abi/src/notifier.rs:97-140`; its only consumer is the orphaned `fault.rs`. Under w288 the host RM writes slot 0 through `hObjectError`, so this emitter is **superseded**, not merely unused |
| Xid printing / `NV_ERR_*` into guest dmesg | **NOT BUILT (indirect only)** | nothing formats an Xid. `ROBUST_CHANNEL_*` reaches the guest only as `info32` in slot 0, written by host RM, which the guest's own `nvidia.ko` turns into an Xid line. Kayfabe-detected conditions (`FwdFault`, `RmError`) go to QEMU stderr and nowhere the guest can see |
| UVM fatal-fault path | **BUILT + REACHABLE** | `uvm_channel_get_status` reads slot 0 and nothing else (`crates/kayfabe-abi/src/notifier.rs:53-57`, citing `uvm_channel.c:2058-2082`). This is what w288 bought and it is real |

---

## 3. Part C — leaks and lifetime

### 3.1 ★★★★★ THE REAP IS NEVER ARMED — the highest-value finding in this audit

The per-object teardown chain is **written, tested, and correct**:

guest client-root `FREE` → `RmEvent::Free` → `Spine::apply` → `plan_refresh` names the proc
`vanishing` (`crates/kayfabe-core/src/gpu.rs:3606`) → `Spine::vacate` (`:3645`) stages
`pending_release` via `stage_dropped_vases`/`stage_dropped_channels` (`:3229`, `:3281`) and
latches `CancelReason::ProcExit` (`Proc::vacate`, `:1690-1709`) → proc pushed to `self.retired`
(`:3910`) → **`Proc::drop` drains it** by issuing real `Release` verbs on the still-live isolate
(`:1818-1850`).

**The break is between "retired" and "dropped".** `Proc::drop` runs only when `Reclaimed` is
dropped, and `Reclaimed` comes only from `Spine::reap_retired` (`gpu.rs:4327`). Verified
first-hand:

- `reap_retired()` has exactly **one** non-test caller: `crates/kayfabe-rt/src/executor.rs:84`.
- `Executor` has **zero production call sites** — and the tree already knows: `shim.rs:3592-3596`
  cites `docs/design/completion_wait_architecture.md` §0.1 measuring `Executor::new` (with
  `Reactor::new`, `register_source`, `arm_counter`, …) at *"zero production call sites"*. The
  `Reactor` composition root that was later built (`shim.rs:3590`) was built for the **completion
  observer**, and it does not call the reap.
- Every other `reap_retired` caller is under `tests/`.

⇒ **Under QEMU, a dead guest process's proc is retired and never reaped.** Its `Proc` is never
dropped, so the staged per-object frees are never issued, and its `IsolateBox` is never dropped,
so the isolate child process is never killed. **Every host RM object it caused to exist — client,
device, subdevice, VAS, TSG, channels, USERD, ctx buffers, `OS_DESCRIPTOR` pins — stays live for
the lifetime of QEMU.**

This is `ARCHITECTURE.md:194-196`'s own stated limit arriving: *"the deferral has no bound, and
'reap-deferred' with nothing arming it is indistinguishable from 'reap-never'."*

**And it has a hard guest-reachable wall.** `MAX_RETIRED_PROCS = 1024` (`gpu.rs:59`) is enforced
at the only guest-reachable growth site (`gpu.rs:3465`): once 1024 procs are retired-unreaped,
deriving a **new** proc fails `GpuError::SpineCapacity`. The constant's own doc anticipates
exactly this: *"an adapter that never reaches one … would otherwise grow this list without
limit — each entry holding an isolate and a GPA arena."* ⇒ **a guest that runs and exits 1024
CUDA processes can no longer start a 1025th**, with 1024 live isolate processes and their whole
RM object trees standing behind it.

**Verdict: BUILT + ORPHANED.** Built, tested (`tests/tests/cross_proc_lifetime.rs:1519`,
`..._and_frees_its_objects_per_object`), unreachable from the VMM. **The fix is a composition-root
line, not a design.**

⊘ Stale doc found in passing: `ARCHITECTURE.md:197-201` still asserts §12.33's *"without issuing a
single `Free` verb"*. §12.35 closed that. The doc contradicts the code.

### 3.2 ★★★★ Guest-RAM pins, joins and exports are never released

**What a published row is** (~18 000 per boot at w292): `mmap(MAP_SHARED)` of the guest-RAM memfd
inside the isolate (`crates/kayfabe-isolate-host/src/guestram.rs:145-158`) →
`NV_ESC_RM_ALLOC_MEMORY` with `hClass = NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over that mapping
(`rm.rs:2396-2436`) → `NV_ESC_RM_MAP_MEMORY_DMA`, `DMA_OFFSET_FIXED_TRUE`, at the guest's own VA,
into **two** host VASes (`rm.rs:2137`, `map_dma_both` at `:4187-4213`).

**What each row pins:** pinned guest pages (`pin_user_pages`, held by the host `nvidia.ko` until
the descriptor is freed — stated at `crates/kayfabe-linux-raw/src/chardev_unsafe.rs:221-231`,
*"the pin is not undone by unmapping"*), one RM object handle, two GPU page-table mappings, and
one isolate-side `mmap`. **Yes: our action pins QEMU's guest RAM in host kernel memory.**

**The primitives all exist** — `unmap_guest_ram` (`rm.rs:5134`), `unmap_dma_both` (`:4219`),
`free` (`:4605`), `GuestRamPlane::release` (`guestram.rs:182`), `VerbPlan::Release`/`Orphans`.
What is missing is the wiring:

| release path | verdict |
|---|---|
| `stage_dropped_vases` → `pending_release` → `Orphans` (rows carrying `Binding::host`) | **BUILT + REACHABLE**, at VAS-death granularity — and gated on §3.1's reap |
| `kayfabe_fwd::unpublish_backing` (the per-range unpin) | **BUILT + ORPHANED** — verified: `crates/kayfabe-fwd/src/lib.rs:2936`; the only mention elsewhere in `crates/` is a doc comment (`gpu.rs:3219`); every caller is under `tests/` |
| `SharedDevice::drain_pending_releases` (the housekeeping sweep) | **BUILT + ORPHANED** — verified: `crates/kayfabe-rt/src/device.rs:1450`; callers only in `tests/tests/{t0_subset_free,l1_mean,retry_ledger}.rs` |
| guest-RAM pin release (`Vas::guest_ram_pins` → `unmap_guest_ram` + `free`) | **NOT BUILT** |
| `FbJoinTable::remove` / `SparseFb::remove_join` | **NOT BUILT** — exactly as `docs/design/operand_join_lifetime.md` §4 predicted; verified still accurate |
| `ChildExports.backings` / `ExportRegistry.adopted` release | **NOT BUILT** — `mint` (`export.rs:74`) and `adopt` (`:147`) have no inverse anywhere |

**The sharpest instance, verified first-hand.** `Vas::guest_ram_pins` is a
`BTreeMap<u64, GuestRamPin>` (`gpu.rs:233`) with `insert`/`get`/`range`/`len` and **no `remove`,
`retain`, `clear` or `drain` in the tree**. `stage_dropped_vases` walks `vas.table` and
`vas.blocks` only (`gpu.rs:3241`, `:3266`) — it never consults `guest_ram_pins`. `GuestRamPin` is
`Copy`, carrying a `HostHandle` and a `GuestRamMapped` (`gpu.rs:336-346`); dropping the `Vas`
drops the map and **loses the handles entirely**, so the objects become unnameable. The struct's
own doc says what that costs:

> *"`memory` is the RM object that **pins** the pages … `mapped` is the isolate's own window onto
> them and is undone by `munmap`. Releasing either alone leaves the other."* — `gpu.rs:331-334`

`GuestRamPin::mapped` has **one write and zero reads** in the whole tree (written
`crates/kayfabe-fwd/src/lib.rs:1900-1907`). Only pins that bound into the table exactly
(`bound_into_table`, `fwd/lib.rs:1938-1957`) get their descriptor freed via a row; **every
multi-row run pin — which the production caller routinely produces (`shim.rs:6605`, `:6656`,
`:9241`) — is never released at all**, and in *both* cases the isolate's `mmap` is never
`munmap`ed.

**Growth is unbounded per guest process.** None of `GuestRamPlane::live`, `Vas::guest_ram_pins`,
`FbJoinTable::joins`, `FbJoinTable::joined_objects` has a cap or a removal. The only numbers that
look like limits are **per-doorbell work budgets** (`VAS_PUBLISH_LEAF_BUDGET = 4096`,
`VAS_PINRATE_ROWS = 256`, `VAS_DRAIN_ROW_CAP = 65536`, wall budgets —
`shim.rs:14630-14709`), which bound how many rows one doorbell publishes, not how many
accumulate. Exhaustible, in order of tightness: **isolate VMA count** (one never-`munmap`ed
mapping per pin; `vm.max_map_count` defaults to 65530 and `VAS_DRAIN_ROW_CAP` is 65536, sitting
right on top of it), **pinned host RAM**, **RM object handles**, **host GPU page-table entries**.

**Guest-reachable amplifier for the export leak:** `join_fb_leaf` mints a backing at `rm.rs:5006`
**before** the RM chain runs; both failure arms (`:5069-5073` map failure, `:5078-5086`
`PlacementRefused`, RM status `0x51`) free the `OS_DESCRIPTOR` and unmap but **neither releases the
minted export token**. Guest-reachable at `shim.rs:10855` → `device.rs:3958` → `child.rs:442`.
⇒ a guest that repeatedly presents leaves at colliding VAs leaks one sealed memfd of `len` bytes
per attempt, unbounded.

★ *Known-positive for these zeros:* the same greps that found nothing for
`guest_ram_pins`/`FbJoinTable`/`ChildExports` **did** find `GuestRamPlane::release`
(`guestram.rs:182`) and its test `a_release_removes_the_mapping_and_a_dropped_plane_removes_them_all`
(`:247`), plus `unmap_guest_ram`, `unmap_dma_both`, `unpublish_backing` and
`NV_ESC_RM_UNMAP_MEMORY_DMA`. The searches are not blind.

### 3.3 ★★★★ The owner's question: in-flight work vs. cancellation — **there is no fence**

**Answer: yes, that path exists, and it is reachable.**

The only quiesce predicate in the tree is `Isolate::is_quiesced`
(`crates/kayfabe-isolate/src/lib.rs:3490`), which is `in_flight() == 0` — *no worker checked out*.
Its own doc is titled *"★ This is NOT 'the device is quiescent' — do not conflate them"*
(`:3450-3462`) and adds *"Not 'every host object has been reclaimed'"*. It gates
`reap_retired`, `Proc::drop` (`gpu.rs:1838`) and `checkout_with_pending_release`.

★ *Known-positive:* a real fence would look like `wait_for_idle` / `RM_IDLE_CHANNELS` /
`await_idle` / a semaphore wait on the teardown path. Grepping all crates returns only (i)
`CtxswPreemptionAsk::WaitForIdle`, an ABI *classifier* for a guest control we answer locally
(`submit.rs:2619`), and (ii) the string `"RM_IDLE_CHANNELS"` in an error-code name table in the
`rmladder` binary (`rmladder.rs:3404`). `HostRmBackend::await_semaphore` (`rm.rs:6567`) *is* a
real fence, used **only** for kayfabe's own CE copies (`rm.rs:6499`, `:6668`, `:7265`), never on
any teardown or free path.

**The reachable free-after-ring.** `commit_doorbell` (`crates/kayfabe-fwd/src/lib.rs:3547`) runs
**after** `ring_doorbell` has already stored the token. Its R5 re-validation can refuse —
`Stale::Proc` (`:3579`), `Stale::Route` (`:3584`), `Stale::Channel` (`:3597`), `Stale::Vas`
(`:3605`) — and the `orphans` closure at `:3561-3568` collects **the host channel that was just
rung**, plus its host VAS. `verb_op` then runs
`let _undisposed = kayfabe_fwd::dispose_on(&mut w, refusal.orphans);`
(`crates/kayfabe-rt/src/device.rs:1975`), which executes `VerbPlan::Release` = `NV01_FREE` on that
channel and VAS immediately. The guest receives `Err(refusal.fault)` (`device.rs:1981`) →
`DoorbellReport::Refused` ⇒ **the guest believes the submission was not served, and may reuse the
pushbuffer and semaphore pages.**

⚠ **Scoping this one honestly — and the HAL work settles half of it.** How bad it is depends on
RM's own free semantics. `kchannelDestruct_IMPL`
(`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:1133-1274`) was read line by line:
**there is no stop, no preempt, no idle-wait and no `kchannelNotifyRc` anywhere in it.** The
de-facto cancellation is a single synchronous `NV_RM_RPC_FREE` at `:1216`
(`rpcFree_v03_00` → `_issueRpcAndWait`, `rpc.c:3325-3349`), and its ordering *is* correct in the
one way CPU-RM controls: the GSP-side teardown completes **before** CPU-RM detaches the GR
context (`:1232`), frees the ctx buffers (`:1234`, `:1246`) and releases the chid (`:1260`).
⊘ The VAS is not sequenced here at all — `kchannelDestruct` never touches `hVASpace`/`pVAS`;
that is a resource-server refcount invariant, not something this function establishes.

⇒ **Whether the GSP's own channel-free handler fences the engine before replying is UNRESOLVED
and out of tree** — see §5.1. What *is* now established is that **nothing in CPU-RM fences**, so
there is no host-side code we could have relied on. If GSP fences, our free-after-ring is a loud,
contained fault; if it does not, it is a write into pages the guest has taken back.

★★ **And NVIDIA's own contracts say success does not mean quiescent**, which reframes this
finding from "we have a bug RM would not have" to "we inherited a hazard RM also documents":
`GPFIFO_SCHEDULE(bEnable=FALSE)` and `DISABLE_CHANNELS(bOnlyDisableScheduling=TRUE)` are
documented as *"no new work"*, explicitly not *"not running"*
(`ctrl2080fifo.h:300-339`); `STOP_CHANNEL(bImmediate=TRUE)` *"[does not] wait for idle"*
(`ctrla06fgpfifo.h:225-232`); `PREEMPT(bWait=FALSE)` returns before the preempt lands
(`ctrla06c.h:179-213`). The only two constructs in the whole set with drain semantics are
`STOP_CHANNEL(bImmediate=FALSE)` and `DISABLE_CHANNELS(bOnlyDisableScheduling=FALSE,
bRewindGpPut=TRUE)` — **and we serve neither** (§1.2). ⇒ the two verbs that carry the guarantee
are exactly the two we refuse.

**The default case is worse than the race.** §3.2's leaked `guest_ram_pins` and `GuestRamMapped`
mappings are **never** torn down before isolate death, so the host GPU keeps a live, RM-pinned
translation into those guest physical pages for the isolate's whole remaining life — long after
the guest freed them. That is *"keeps writing into pages already returned to the guest's free
pool"* arriving by construction rather than by a lost race. And with §3.1 unarmed, "the isolate's
whole remaining life" means "until QEMU exits".

**Nothing prevents guest page reuse.** `Shim::region_del` (`shim.rs:649-651`) forwards to
`QemuMachine::region_del` (`crates/kayfabe-vmm-qemu/src/lib.rs:1721-1741`), which touches only the
VMM's own view — no `unmap_guest_ram`, no `retire_proc`, no `unpublish_backing`. A balloon inflate
or a memory hot-unplug unjoins nothing. The guest's *own* page reuse is not observable at all; the
shim says so at `shim.rs:8874-8880` (UVM release / `va_space_destroy` *"are not observable events
here at all"*). ★ *Known-positive:* grepping `unmap_guest_ram|request_cancel|retire_proc|
unpublish_backing` across `crates/kayfabe-vmm*/` and `crates/kayfabe-qemu-raw/` returns only two
doc-comment mentions and zero call sites.

**Blast radius is bounded to the causing guest.** One sandbox per `(Proc, GpuId)`
(`crates/kayfabe-isolate/src/lib.rs:3750-3762`), its own RM client namespace, host VASes allocated
only through it; the guest-RAM memfd is granted per-isolate from one factory per VM
(`crates/kayfabe-isolate-host/src/isolate.rs:1187`); export tokens are isolate-scoped
(`:1200-1202`). An isolate can reach **any page of its own VM's RAM**, and **no other VM's**. The
leaked mappings therefore reach *any process inside the causing guest* that later gets those
pages — not the host, not a neighbour. Docs (`multi_tenant_isolation_assessment.md` §1,
`guest_blast_radius.md` §1) match the source.

### 3.4 fd lifetime — clean, and the one gap is not an fd gap

**No bare-`RawFd` ownership transfer exists.** Every host fd is `OwnedFd` or `std::fs::File`:
`/dev` and `/dev/nvidia*` (`crates/kayfabe-linux-raw/src/chardev_unsafe.rs:143`, `:196`, `:345`),
`memfd_create` (`host_fd_unsafe.rs:125`, `spawn_unsafe.rs:216`), `F_DUPFD_CLOEXEC`
(`host_fd_unsafe.rs:183`), pipes (`spawn_unsafe.rs:1025`), SCM_RIGHTS receives
(`scm_unsafe.rs:410`, adopted into `Vec<OwnedFd>` **before** the count bound, so an over-count
frame cannot strand kernel-delivered fds). `libc::close` appears **nowhere** in the tree, so there
is no unmatched- or double-close shape either. The only `into_raw_fd` is inside a `#[cfg(test)]`
assertion. ★ *Known-positive:* the grep
`libc::close|into_raw_fd|from_raw_fd|RawFd|dup(` does return the `spawn_unsafe.rs` /
`host_fd_unsafe.rs` adopters, so it would have surfaced a bare handoff.

**SCM_RIGHTS double-ownership: no leak.** `CrossedFd::adopt`
(`crates/kayfabe-isolate-host/src/fdcross.rs:94-108`) takes the `OwnedFd` **by value**, so a
kind-check refusal *closes* the descriptor instead of stranding it open; sends use `BorrowedFd`
(`scm_unsafe.rs:149`); every refusal arm in `call_for_backing`/`call_for_joined`
(`isolate.rs:433-465`, `:487-520`) leaves `fds` owned, including the "reply of the wrong shape"
arm that is the classic leak; `lend_to` (`fdcross.rs:138`) gates the borrow on target
`IsolateId`. The one deliberate double-ownership — the child keeps its `SharedRam` and sends a
`dup` (`export.rs:83-85`) — is correct as a design; the defect is §3.2's, that **neither end ever
releases**.

**The owner's design, split:**

- per-isolate fd tracking — **BUILT + REACHABLE** (`ChildExports`, `export.rs:48`, built at
  `child.rs:176`; `ExportRegistry`, `export.rs:120`, one per `HostIsolate`, `isolate.rs:880`).
- global per-client table addressing objects and closing on last reference — **NOT BUILT at the
  fd layer**; **BUILT + REACHABLE at the guest-object layer** (`RmGraph`, with real
  last-reference semantics — `free_subtree`, `rmgraph.rs:2008`, client-root free sweeping the
  namespace, `:2013-2018`). `RmGraph` tracks **guest** handles and never names a host handle;
  host handles live in `Proc::pending_release`/`Orphans` and the isolate's own `Objects`
  (`rm.rs:471-509`).

**The C-era orphan bug is closed the right way.** `HostHandle` carries its `IsolateId`
(`crates/kayfabe-isolate/src/lib.rs:140`) and `narrow()` refuses foreign ones, so the
guest-namespace/host-namespace confusion that produced *"two orphan generations in one namespace
freeing host memory RM says is live"* is **not expressible**. `HostRmBackend::free`
(`rm.rs:4605`) orders CE channel before its VAS, exec-VAS after the channel whose ring lives in
it, CPU mappings before RM objects, TSG outliving the channel, with `RingOwner::HandedIn` /
`UserdOwner::InRing` arms that deliberately free nothing; `free_one` (`:8521`) refuses a handle
this connection never minted. **The mirror failure — never freeing — is what §3.1 and §3.2
record.**

### 3.5 The isolate as a process

**Whole-client cascade — correct mechanism, same unarmed trigger.** `isolate.rs:1137-1148`:
*"dropping the child kills and reaps it, at which point the kernel frees the entire RM object tree
under this isolate's client."* The reaper is the isolate process dying, which closes its
`OwnedFd` on `/dev/nvidiactl` (`rm.rs:1371`) and makes RM cascade. **That fd is owned, so the
design is valid** — but the trigger is `SandboxChild::drop`
(`crates/kayfabe-linux-raw/src/spawn_unsafe.rs:980-1000`), reached only through
`HostIsolate` drop → `Proc` drop → `Reclaimed` drop → `reap_retired`. **Same orphaned edge as
§3.1.** ⇒ **BUILT + ORPHANED.**

**If the VMM dies:** the isolate is a direct child of QEMU, cloned into six namespaces including
`CLONE_NEWPID`. **There is no `PR_SET_PDEATHSIG` and no supervisor** — the only `prctl` in the
tree is `PR_SET_NO_NEW_PRIVS` (`spawn_unsafe.rs:584`). What saves it is EOF: QEMU's death closes
the parent ends of the worker socketpairs, `read_frame` returns `Ok(false)`, `worker_loop`
returns (`child.rs:383`), `serve` joins and returns (`child.rs:239-247`), the process exits, the
`nvidiactl` fd closes, RM cascades. **That works even for a `SIGKILL`ed QEMU.**
⊘ **The hole:** `serve` blocks on `t.join()` and a worker observes EOF only *between* verbs. A
worker parked inside a long or hung host RM ioctl never returns to `read_frame`, and the cancel
that would break it (`control_loop` → `interrupt_thread`, `child.rs:325-352`) is driven by a
datagram from the now-dead parent. ⇒ **an orphaned isolate holding a real GPU context, surviving
both the guest and the VMM, is reachable.** Narrow window, nothing structural closes it,
`PR_SET_PDEATHSIG` would close it in one line. **BUILT + REACHABLE via EOF; NOT BUILT for the
in-ioctl case.**

**If the isolate dies while the VMM lives:** noticed, named, no hang, **no double-allocate**.
`write_frame`/`read_frame_with_fds` failures become `RmError::Wedged` (`isolate.rs:410`, `:415`,
`:433`), the slot goes `Slot::Dead` (`:1091`), and `cancel_handle` (`:1069-1082`) is keyed on
*"a txn is outstanding"* rather than `Slot::Busy` so a requester parked in the dead verb is
released. Respawn is structurally forbidden by `Spine::refresh` step 0
(`gpu.rs:3670-3700`). **BUILT + REACHABLE.**

**The watchdog escape is honest about its residual.** `declare_wedged`
(`crates/kayfabe-rt/src/device.rs:1781-1809`) kills the slot, abandons the requester and condemns
the component as one act, and states the cost where the code is (`:1777-1780`): *"the D-state
host thread and its RM objects leak until the kernel finishes the ioctl — `SIGKILL` does not reap
a task in uninterruptible sleep."*

### 3.6 One early release, and its stated reason is wrong

`SparseFb::device_reset` does `self.joined.clear()` (`crates/kayfabe-device/src/fbwin.rs:1107-1119`),
justified in its own comment by *"the isolate's own half dies with the isolate."* **That premise
is false at this call site:** `Shim::reset()` (`shim.rs:12327-12329`) resets only `self.plane` —
it retires no `Proc`, drops no isolate, stages no `Orphans`. So after a guest-initiated device
reset the **device-side** `mmap` of every joined leaf is `munmap`ed while the **isolate-side**
`MappedRegion`, the `OS_DESCRIPTOR` and **both** GPU VA mappings remain live, and the host GPU can
still walk the previous device life's mappings. Not a use-after-free of host memory (the memfd
and the RM pin keep the pages alive) — **a leak plus a stale-mapping hazard, with an incorrect
lifetime argument attached.**

⊘ For balance, the *opposite* direction is properly guarded: a guest's own PTE clear cannot pull a
published row out from under the GPU — `apply_settlement` refuses it as
`PopulateRefusal::UnbindsPublished` (`crates/kayfabe-mmu/src/reach.rs:806-813`), and a re-point as
`RepointsPublished` (`crates/kayfabe-mmu/src/walker.rs:1061-1065`). The price of that guard is
that a published row is frozen for the life of the `Vas`, which is the stale-mapping cost
`operand_join_lifetime.md` §5 already names.

---

## 4. Ranked findings — by what a guest can actually cause

| # | finding | what a guest does to cause it | verdict | §|
|---|---|---|---|---|
| 1 | **The reap is never armed.** Every dead guest process leaks its entire host RM object tree plus a live isolate process, until QEMU exits; at 1024 retired procs no new process can be derived | run and exit CUDA processes — ordinary use | **BUILT + ORPHANED** | 3.1 |
| 2 | **Guest-RAM pins, joins and exports are never released.** Pinned host RAM, `OS_DESCRIPTOR`s, GPU PTEs and isolate VMAs grow monotonically per proc; the host GPU keeps live translations into guest pages the guest has freed | ordinary use; amplified deliberately by repeated colliding-VA leaf joins (unbounded memfd leak) | **NOT BUILT** (+ two orphaned release verbs) | 3.2 |
| 3 | **No GPU quiescence fence anywhere**, and a refused doorbell commit frees the host channel it just rang | race an R5 refusal; or simply free anything (the leaked-pin case is the default, not the race) | **NOT BUILT** (fence); **BUILT + REACHABLE** (free-after-ring) | 3.3 |
| 4 | **`NVA06C_CTRL_CMD_PREEMPT` is answered `NV_OK` and performs nothing**, on a TSG with a live scheduled host twin — and it is `ROUTE_TO_PHYSICAL`, so **we are the only party that could ever have performed it** | issue the control (libcuda issues it once per `cuCtxCreate` today, not as a cancel — so the lie is latent, not yet exercised) | **BUILT + REACHABLE**, and wrong | 1.4, 1.5 |
| 5 | **We cause the host RM to write the guest's notifier slot 1** with a host-space work-submit token | create a channel and ring it — ordinary use | **verified mechanism; guest consequence UNMEASURED** | 2.2 |
| 6 | **Every cancellation verb NVIDIA actually has is unserved**: `STOP_CHANNEL` (in all 25 boots' ledgers), `RESET_CHANNEL`, `DISABLE_CHANNELS`, `DISABLE_USERMODE_CHANNELS`, the whole RC family. ★ **The two that carry a drain guarantee — `STOP_CHANNEL(bImmediate=FALSE)` and `DISABLE_CHANNELS(bOnlyDisableScheduling=FALSE)` — are exactly the two we refuse**, and our `0x56` to `STOP_CHANNEL` additionally suppresses the guest CPU-RM's own RC notifier write | any UVM channel teardown; any MMU-fault recovery; any suspend | **NOT BUILT** (refused `0x56` by name) | 1.2, 1.5, 3.3 |
| 7 | **Descheduling never reaches the host.** `GPFIFO_SCHEDULE(false)` removes a set entry; the host TSG stays scheduled | issue the control | guest plane **BUILT + REACHABLE**; host plane **NOT BUILT** | 1.2 |
| 8 | **`RC_TRIGGERED` emission is orphaned** — we can never tell a guest its channel died except through the w288 slot-0 passthrough | cause a fault | **BUILT + ORPHANED** | 2.5 |
| 9 | **The isolate's own CE channel is born with `hObjectError = 0`** — nothing can ever be told if it RCs | any CE doorbell | **NOT BUILT** | 2.1 |
| 10 | **Status flattening**: the MMU-fault-info relay collapses every host outcome to `0x40`; `BridgeRefusal` collapses to `0x56` | ask for fault info after a fault | **BUILT + REACHABLE**, lossy | 2.4 |
| 11 | **`read_version` never reads its in-struct `reply`** — a host driver refusal passes the version gate invisibly | not guest-reachable (host-side) | **the measured `failed=0` class** | 2.4 |
| 12 | **`device_reset` releases the device half of every FB join** while the isolate half stays live, on a false lifetime premise | guest-initiated device reset | **BUILT + REACHABLE**, early release | 3.6 |
| 13 | **No `PR_SET_PDEATHSIG`** — an isolate parked in an ioctl when QEMU dies survives, holding a real GPU context | kill QEMU during a slow verb | **NOT BUILT** | 3.5 |
| 14 | **`exec_engine_object` hard-codes `hObjectError = None`** — the bare-`Gpu` reference model diverges from the shell | not guest-reachable today | **differential hole** | 2.1 |

**What is genuinely good, and should not be lost in the list above:** the default control arm is a
named refusal with a ledger, not a silent `NV_OK`; the fd layer has no raw-fd or SCM_RIGHTS leak
shape at all; `HostHandle`'s isolate stamping makes the C's orphan-generation bug inexpressible;
`HostRmBackend::free` gets RM's teardown ordering right and cites it; the MMU-fault-info relay is
strictly on-demand and verbatim, exactly as required; the cancellation seam
(`CancelReason`/`CancelSink`/txn staleness) is a careful, correct piece of work for the problem it
actually solves; and `Orphans`/`VerbFailure` are `#[must_use]` with messages that name the leak
they prevent.

**The distance between that and the findings is almost entirely composition-root wiring, not
design.** Findings 1, 2's release verbs, and 8 are all *"written, tested, and never called."*

---

## 5. What this audit could not settle, and must not be reasoned about further

1. **Does the GSP fence the engine on `NV01_FREE` of a channel?** §3.3's severity turns on it.
   ⊘ **Half-settled: CPU-RM definitively does not** (§3.3, `kchannelDestruct_IMPL` read in full).
   The other half is **out of tree, not merely unresolved** — the GSP-side handlers for
   `NV01_FREE` on a channel, `STOP_CHANNEL`, `RESET_CHANNEL`, `GPFIFO_SCHEDULE`,
   `FIFO_DISABLE_CHANNELS` and `PREEMPT` are all closed firmware. ⇒ **every quiescence-relevant
   behaviour lives in that unresolved set**, and no amount of ogkm reading will settle it. It
   needs a bench experiment or a decision to not depend on it.
2. **Which guest consumer reads notifier slot 1, and what a host-space token does to it** (§2.2).
   The write is certain; the consequence needs a boot.
3. **Does the host RM actually write the guest's pages through `hObjectError`?** We verify only
   that RM *accepted the handle* (§2.4). w288's `CUP2_RC` 124 → 1 is strong circumstantial
   evidence for slot 0; there is no read-back.
4. **What an interrupted `NV_ESC_RM_ALLOC` leaves behind** — still the open G4 question, named in
   `crates/kayfabe-isolate/src/lib.rs:2305-2308` as *"needs a bench experiment, must not be
   reasoned about"*. This audit does not change it.

## See also

- `docs/design/operand_join_lifetime.md` — the ogkm answer and the cleanup design for joins.
  Verified still accurate; §3.2 records the two leak classes that arrived after it (leg 8's
  ~18 000 rows and `GuestRamPin::mapped`).
- `docs/reference/mode2_bench_lifecycle.md` — the C's measured teardown behaviour, and why it is
  not an oracle here.
- `docs/design/gpfifo_schedule.md` — §2's P1/P2/P3 split; P3 (*"runlist ordering, timeslice,
  interleave, preemption … not modelled at all"*) is the design statement §1.4 argues has been
  outgrown for `PREEMPT`.
- `ARCHITECTURE.md:194-201` — states limit (a) that §3.1 measures arriving, and carries the stale
  §12.33 sentence noted there.

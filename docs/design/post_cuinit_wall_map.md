# The post-`cuInit` wall map — from the CE copy to a 2048² matmul

> ### STATUS — 2026-08-11 (w258 doc-hygiene sweep) / **LIVE — thesis unrefuted; ONE ROW (D2) is STALE**
>
> ★ **The doc's central claim stands** and was acted on across w220–w247: *the critical path to
> compute is not the next driver wall, it is the executor plane.* Keep planning from it.
>
> ⊘ **STALE, D2 only (`MC_SERVICE_INTERRUPTS`, `0x20801702`, §D2 below).** That row's *kayfabe*
> cell reads *"allowlisted (`crates/kayfabe-abi/src/capability.rs:745`), **unmodelled ⇒ refused by
> name**"*. **We now SERVE it**: `8574466` (2026-08-10) — *"§16.75 SERVE `0x20801702`
> MC_SERVICE_INTERRUPTS"* — adds `crates/kayfabe-rmrpc/src/policy.rs` (+128) and
> `tests/tests/mc_service_interrupts.rs` (+285).
> ⚠ **Only the kayfabe-status half of D2 moved.** The same commit *cites this doc approvingly* on
> D2's other half — the C's fix is still **DO-NOT-PORT**. Do not read "we serve it now" as
> "port the C's credit maze"; those are different claims and D2 makes both.

**Purpose.** Pre-solve, on paper, every wall between where kayfabe stands today and `cup8`, so the
bench never stalls on analysis. For each wall: the guest **function** (not the control id), every
demand it makes before its next hard exit, how the C served it, whether kayfabe has it, and the
sharpest question one boot could answer.

**This doc is not the first attempt.** `c_cuda_ladder.md` §2 is already an ordered wall list
(A: rbp-clobber, B: the bare 999, C: `MC_SERVICE_INTERRUPTS`, D: the GR keystone). It remains
correct as history. What this file adds: (1) a PORTABLE / PORTABLE-IF-REWRITTEN / ⊘ FORGERY
classification per C mechanism, (2) a does-kayfabe-have-it column against the tree, (3)
function-granular demand lists read from the **580** driver, and (4) six corrections, three of them
to its own brief and one to its own first draft (§0.5).

**Citation prefixes.** `C:` = `/workspace/nvidia-gpu-passthrough`, branch `consolidation`.
`ogkm-580:` = `C: research_clones/ogkm-580.159.04/` (`NVIDIA_VERSION = 580.159.04`, the bench driver).
`ogkm-610:` = `C: research_clones/ogkm/` (610.43.02). ⚠ **The two trees' line numbers differ by tens
of lines in every file touched here — never cross-cite.** `rs:` = this repo. `mem:` = the C-era agent
memory. Tags: `[src]` (read from code), `[measured]` (a run is named), `[inferred]` (reasoned,
nothing ran). Most of this file is `[src]`/`[inferred]`; that is the honest tag, not an apology.

---

## 0. What this REFUTES — including its own brief

### 0.1 ⊘ The wall kayfabe stands at CANNOT be forged — the driver byte-checks it

`[src]` `memmgrTestCeUtils` writes `0xAABBCCDD` to a vidmem memdesc, CE-copies it to a sysmem
memdesc, reads it back and asserts `sysmemData == vidmemData`
(`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/mem_mgr.c:466-470`). A completion signalled over data
that did not move fails the driver's own comparison. ⇒ **Forgery is not merely disallowed here, it
is not available.** The C's answer — really moving the bytes in software, then releasing the
semaphore the guest's own pushbuffer named — is honest, and kayfabe already implements that shape
(§3 B1).

### 0.2 ★★★ The C writes the completion BEFORE it rings the host doorbell, and never waits

`[src]` The C's per-channel doorbell loop is fixed and mechanical:

| step | site | what happens |
|---|---|---|
| 1 | `C: src/qemu/nvkvm_gpu_emul.c:4107` | `nvkvm_chan_execute()` — parse the methods, do the CPU copies, **and write every semaphore the stream names** (CE `:6496-6510`, host `SEM_EXECUTE` `:6519-6543`, **compute report `:6546-6572`**) |
| 2 | `:4116` | advance `fin_payload` by the GP entries consumed |
| 3 | `:4220` (and `:9159`) | `stl_le_p(usermode_qva + 0x90, token)` — **now** tell the host GPU to run the work |
| 4 | `:4296-4350` | write the channel-host `finishPayload` |
| 5 | `:4365` | `nvkvm_gsp_deliver_events()` → SWGEN0 → guest IRQ |

`[src]` And nothing in the live path polls a value the host GPU wrote; the only such poll is
`nvkvm_m2_channel_selftest` (`C: …:9450-9640`), which runs once from `realize` on a private channel
and never touches guest state.

⇒ `[inferred]` **`cup8`'s `bad=0 maxerr=0` is a real host-GR result read from behind a completion
that did not wait for it.** The arithmetic is un-forgeable and the host really ran it — that part of
the oracle stands. The *ordering* is a race the host wins because it finishes in microseconds. This
is the most likely shape of `#13`'s non-reproducibility (`mem: mode2_13_multiiter_idle_hang.md`:
1/3 one day, 9/9 the next). ⊘ **Do not port the ordering, and do not read a green `cup8` as evidence
about the completion plane** — it is evidence about the arithmetic only.

### 0.3 ★★★ The one forgery on the default stock-guest path cites a rule that does not exist

`[src]` `C: src/qemu/nvkvm_gpu_emul.c:4259` justifies the `finishPayload` write with *"per the
address-table rule «complete now if no real work»"*. That rule is **not in**
`C: docs/design/mode2_address_table.md`. Grepping both governing docs returns the C comment and
nothing else; the search is capable — `mode2_address_table.md` contains "complet" twice, so a hit was
possible. The phrase's actual home is `C: docs/design/mode2_2nd_context_hang.md:150`, where it is
**numbered option 2 of a two-option proposal**. And the document it was attributed to has a sibling
stating the contrary: *"A completion is a **real host-GPU write**, never a forged value"*
(`C: docs/design/mode2_forwarding_model.md:71`), with *"Forging a completion value the guest's
userspace observes"* in its anti-pattern list (`:151`).

⇒ A proposal became a rule by being cited, and the citation pointed at the document that forbids it.
⊘ **Never port a C mechanism on the strength of the rule its comment names — open the named document.**

### 0.4 ★★★ Serving `CE_GET_ALL_CAPS` would NOT move the CE-caps wall

`[src]` The function that walls UVM is `queryCopyEngines`
(`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:8449-8541`), and it never issues `0x20802a0a`.
Four call sites, three distinct ids, each under a hard exit:

| # | control line | cmd | note |
|---|---|---|---|
| 1 | `:8457` | `0x20800123` `GPU_GET_ENGINES` | **sizing pass**, `engineList = NULL`; exit `:8463 return status` |
| 2 | `:8473` | `0x20800123` again | **fill pass**, list from `portMemAllocNonPaged(4 * engineCount)` at `:8466`; exit `:8479 goto done` |
| 3 | `:8503` | `0x20802a01` `CE_GET_CAPS` | per copy engine; exit `:8509 goto done` |
| 4 | `:8521` | `0x20802a02` `CE_GET_CE_PCE_MASK` | per copy engine; exit `:8527 goto done` |

`ceCaps->supported = NV_TRUE` is set only after **both** 3 and 4 succeed (`:8534`).

⊘ **And the remembered shorthand is wrong in both numbers.** *"`queryCopyEngines` issues two controls
six lines apart"* — it is four call sites over three ids, and the spacings are **16** (1→2) and
**18** (3→4) lines. The two-under-two-`goto`s shape it describes matches the **loop body**
(3 then 4, per CE) and separately matches `engineAllocate`
(`ogkm-580: nv_gpu_ops.c:6210` alloc + `:6242` `GPFIFO_SCHEDULE`, both `goto cleanup_free_engine`,
19 lines apart).

`[src]` In this tree: `0x20800123` allowlisted (`rs: crates/kayfabe-abi/src/capability.rs:700`),
`0x20802a02` allowlisted (`:757`), and **`0x20802a01` absent from the allowlist entirely** ⇒ the
harder `ControlNotPermitted`. Meanwhile `0x20802a0a` (`:759`) and `0x20802a03` (`:758`) *are*
allowlisted and have encoders (`rs: crates/kayfabe-abi/src/cecaps.rs:426`, `:533`) whose only callers
live in `crates/kayfabe-device/tests/`.

⇒ **The obvious increment — wire the `CE_GET_ALL_CAPS` encoder into `answer()` — serves a control
this function never calls and moves the wall zero lines.** `read_the_caller_not_the_id`, caught
before a boot was spent.

### 0.5 ⊘⊘ THE GOLDEN CONTEXT IS A **BOOT** WALL, NOT A `cuCtxCreate` WALL — correcting this file's own first draft

`c_cuda_ladder.md` §2 places the GR keystone (Wall D) inside/just past `cuCtxCreate`, and this
document's first draft copied that placement. `[src]` Both are wrong for the 580 driver.

The golden-image channel is built from a **post-scheduling-enable callback fired during
`gpuStatePostLoad`**, i.e. during `modprobe`, before any CUDA process exists:

`kfifoStatePostLoad_GM107` (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:231`)
→ `kfifoTriggerPostSchedulingEnableCallback_IMPL` (`kernel_fifo.c:3076-3141`) → the handler list, in
registration order:

| # | handler | registered at |
|---|---|---|
| 1 | `memmgrPostSchedulingEnableHandler` → **`memmgrTestCeUtils`** | `mem_mgr.c:664` |
| 2 | `_kmigmgrHandlePostSchedulingEnableCallback` | `kernel_mig_manager.c:1207` |
| 3 | `_kgraphicsPostSchedulingEnableHandler` → **`kgraphicsCreateGoldenImageChannel`** | `kernel_graphics.c:373-377` |
| 4 | `kceRunFipsSelfTest` — skipped, gated on `bCcFipsSelfTestRequired`, cleared at `kernel_ce.c:462-465` when Confidential Compute is off | `kernel_ce.c:470` |

⇒ **The true order is B1 → B2 → B3 (golden context, incl. `GPU_PROMOTE_CTX`) → driver load
completes → `cuInit` → UVM's CE caps → UVM's channels.** Everything `c_cuda_ladder.md` calls Wall B,
C and D is either earlier than it says or later; the CE-caps wall in particular sits **after** the
golden context, not before it.

⊘ **Also correcting an inherited citation.** `kernel_graphics.c:478` *"Nothing to do for
non-GSPCLIENT"* is a **610** line number. In 580 the comment is at `:476` and `:478` is the
`return NV_OK`; the function is `_kgraphicsPostSchedulingEnableHandler` (580 `:464`, 610 `:467`).
⊘ And two names in circulation do not exist in **either** tree: `kchannelCreateRunlist` and
`kfifoChannelGroupConstruct`. The real TSG chain is `kchangrpapiConstruct_IMPL`
(`kernel_channel_group_api.c:49`) → `kchangrpInit_IMPL` (`kernel_channel_group.c:103`); runlist
assignment is `kfifoRunlistSetId_HAL` (`kernel_channel.c:891`, `:972`).

### 0.6 ⊘⊘ THE BRIEF'S PREMISE IS WRONG ABOUT WHAT BLOCKS THE LADDER — the headline

The brief asks for the next walls "in the order the guest will hit them", assuming the blocker is
always the next driver demand. `[src]` For every rung past the CE copy, it is not:

`rs: crates/kayfabe-qemu-raw/src/shim.rs:3694` `selected_isolate_plane()` returns
`IsolatePlane::Stillborn` when `KAYFABE_ISOLATES` is unset, and `host-isolates` is a **non-default**
feature (`rs: crates/kayfabe-qemu-raw/Cargo.toml:86-87`), so in every shipping build the entire
`kayfabe-isolate-host` tree — host `RM_ALLOC`, host channel, host CE, host doorbell — is **linked
out**. `shim.rs:2300-2323` says it plainly: the forwarding plane is *"`Stillborn` in every shipping
build"*, therefore `local_ce_is_the_only_executor` is `true` and **100 % of guest work runs on the
CPU executor** (`rs: crates/kayfabe-rt/src/ceutils.rs`).

A CPU executor can serve a CE memset and a CE copy. It **cannot** serve a GR matmul, and building
one that could is the one thing `C: docs/design/mode2_gr_forwarding.md:33` names FORBIDDEN
(*"a GR/compute METHOD emulator … Never build it"*).

⇒ **The critical path to compute is not the next driver wall. It is the executor plane.** Ordering
the queue by driver demand alone sends boots at questions the build cannot answer.

### 0.7 ★★★★ And flipping that switch REGRESSES the rung that currently works

`[measured 2026-08-08, boot pub1_3e43e9a, rev 3e43e9a]` (recorded at
`rs: crates/kayfabe-qemu-raw/src/shim.rs:2290-2323`) the CPU executor's admission test used to be
*"the core cannot address this channel"*. When `Vas::pdb` started resolving, the CeUtils scrubber's
channel became addressable, this executor **declined it**, the doorbell fell through to the
Stillborn plane, and the boot read `doorbells: 1 arrived, 0 served, 1 REFUSED
[FwdFault::IsolateRetired]` → `memmgrMemSet` `NV_ERR_TIMEOUT 0x65` at `mem_mgr.c:463` →
`ce_utils.c:349` → `RmInitAdapter failed! (0x25:0x65:1249)`.

The gate was rewritten to ask *"is there any other executor?"*, answered from the composition root
before any doorbell arrives. `[src]` So it is `true` **only while the plane is Stillborn**, and
`shim.rs:2320-2322` states the consequence: *"A build that selects a real isolate plane keeps the old
routing exactly — a channel the core can address goes to the core."*

⇒ **`KAYFABE_ISOLATES=real` does not add GR to a working CE path. It simultaneously takes the CE
scrubber away from the executor that serves it today** — and `RmInitAdapter` re-opens. Third
instance of `accuracy_is_fatal_when_a_fallback_was_keyed_on_ignorance`.

★ **The C solved exactly this, and its answer is a per-channel routing key, not a boolean.** `[src]`
The C routes by **client kind**: `nvkvm_m2_is_user_ce` (`C: …:2493`), `nvkvm_m2_is_gr_client`
(`:5115`), `nvkvm_m2_is_user_client` (`:5130`). The kernel-CeUtils scrubber stays on the software
path while GR and user-CE channels go to the host — exclusion at `C: …:4296-4298`, host-CE admission
at `C: …:6310-6311`. ⇒ Port the **three-way client-kind predicate**, not the boolean. §5 Q2.

### 0.8 ⊘ There is NO C oracle for CE-forward (task #2) — and the C ruled it out by measurement

`m2cexec` — the only flag under which the C's host CE executes a guest `LAUNCH_DMA`
(`C: …:6310-6311`) — defaults to `false` (`C: …:9932`) and appears in one script, the `m570` probe.
`[measured]` The green ladder ran `NVKVM_M2CEFWD=1` and nothing else
(`C: docs/BENCH_REBUILD_NOTES.md:486`), i.e. `m2cexec=off`. ⇒ **In the run that produced
`cup8 bad=0 maxerr=0`, every CE byte was moved by a QEMU `memcpy` (`C: …:6418`)** and no host copy
engine ran at all.

`[measured]` The C then probed it and ruled it out: *"inert — the bulk copies are **PHYSICAL-mode**
(`dst_phys=1 verdict=gpga`, guest-fb-phys meaningless to the host), so a verbatim pushbuffer-forward
can't translate them; and 73 % are <4K"*
(`C: docs/design/mode2_userbuf_vidmem_passthrough.md:546-550`).

⇒ **(a)** The completion-plane limit in `c_rust_trace_differential.md` extends here: agreement with
the C on CE-forward proves nothing, because the C has no CE-forward. **(b)** The structural half is a
*correctness* fact, not a perf one: a physical-mode CE copy has **no PTE**, so no host GMMU can reach
its destination. The C's correctness answer was to **back the physical destination range as a real
host vidmem object** (`nvkvm_m2_gpga_obj_ex`, `C: …:6260`), not to forward the copy. ⊘ Do not
re-derive the forward and re-discover it is untranslatable.

### 0.9 ★★★ TWO SILENT SKIPS — walls that will never appear as a refusal

Both are `[src]` and both are the `a_saturated_instrument_looks_exactly_like_absence` family: a
demand we fail to serve produces **`NV_OK` and a wrong number**, not a stop.

**(a) `0x20802a08` (CE fault-method-buffer size) cannot fail loudly.**
`gpuGetCeFaultMethodBufferSize_KERNEL` (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:6031-6043`) ends
in `return NV_OK` **unconditionally** — `:6039` only assigns `*size` when the inner call succeeded,
and `:6042` returns OK either way. Its consumer
`kchangrpAllocFaultMethodBuffers_GV100`
(`kernel_channel_group_gv100.c:36-150`) initialises `bufSizeInBytes = 0` at `:44`, calls it at `:77`
under `NV_ASSERT_OK_OR_RETURN` (which cannot fire), then checks `NV_ASSERT((bufSizeInBytes > 0))` at
`:78` — a **bare assert with no return**, a no-op in a release build. It proceeds to
`memdescCreate(…, bufSizeInBytes, …)` at `:109` and `memmgrMemSet` at `:138`. That base and size are
then handed to the far side verbatim: `kernel_channel_group_api.c:464-465` fills
`methodBufferMemdesc[].size` and `:491-496` sends
`NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS` (`0xa06c010a`); the same values are
duplicated into the channel-alloc RPC at `kernel_channel.c:2765-2783`.
⊘ **So an unserved `0x20802a08` yields a zero-length DMA target that the far side programs the CE
fault-method-buffer pointer at — silently.** It also under-sizes the **BAR2 aperture**
(`kfifoCalcTotalSizeOfFaultMethodBuffers_GV100`, `kernel_fifo_gv100.c:292-325`, consumed at
`kern_bus_gv100.c:454`) and the RM FB reservation (`mem_mgr_gm107.c:1925-1928`).
⊘ And `pGpu->ceFaultMethodBufferSize` is **never assigned anywhere in the tree** (read at
`gpu.c:6033`, declared at `generated/g_gpu_nvoc.h:1333`, no writer), so the control is re-issued on
every call rather than cached — it is a hot-path reply, not an init-time one.

**(b) The golden-context path skips itself on `gpcMask == 0`.**
`_kgraphicsPostSchedulingEnableHandler` (`kernel_graphics.c:464-515`) returns `NV_OK` at `:485` if
`pKernelGraphicsStaticInfo == NULL` and at `:486-487` if `gpcMask == 0`. ⇒ **If our static-info
answers zero GPCs, the entire golden-image channel — and with it `GPU_PROMOTE_CTX` and every GR
context buffer — is never built, and the boot looks clean.** GR compute then fails much later for a
reason that produced no signal at the time.

⇒ Both belong on the wall list as **things to assert positively**, never as refusals to wait for.

---

## 1. The anchor — where kayfabe stands, and what it implies

`[measured 2026-08-09, boot row1_44b7d69]` (`rs: docs/design/execution_plane_increments.md` §16.8)

```text
[FwdFault::RingBroughtNoEntry] { ring_va: GpuVa(0x121010000), index: 0, entries: 1024 }
  | c=0xc1d0000a vas=0xcaf00005 root=0x4000/ap1/sh47 … L0=0x4000 L1=0x5000 L2=0x6000 L3=0x7000
```

Eleven VA-space publications split into two families: working rows carry real GA106 framebuffer
physicals (`0x2efa9b000` and neighbours, descending); the refusing rows carry tiny ascending
consecutive 4 KiB values, contiguous across two VA spaces. Every other field is identical. ⊘ **The
next rung §16.8 specifies — dump our framebuffer at `0x4000`/`0x5000` and diff against `0x2efa9b000`
— is the bench agent's and is not re-litigated here.**

★ **What this map adds is the channel's identity.** `[src]` A UVM channel exists only after
`UVM_REGISTER_GPU`, which runs inside `cuInit`
(`uvm.c:1008` → `uvm_gpu.c:3801 uvm_api_register_gpu` → `uvm_gpu.c:2820 init_gpu` →
`uvm_gpu.c:1615 uvm_channel_manager_create`). ⇒ **`0xc1d0000a` is a `cuInit`-era channel, so this
boot got past the whole boot-path sequence B1–B4 in §3** — including `memmgrTestCeUtils` and the
golden-image channel. `[inferred]` That is either real progress or §0.9(b)'s silent skip, and
nothing in the current log separates them. **§5 Q0 is the question that does.**

`[src]` The current wall's exact driver home is `channel_init` (`uvm_channel.c:2518-2572`), whose
`:2563 uvm_push_end_and_wait` is **the first end-to-end push → doorbell → semaphore-writeback round
trip anywhere on the `cuInit` path**. Nothing past it happens until the device retires that push.

---

## 2. Two eras, and the switch between them

| era | executor | which walls it can answer | gate |
|---|---|---|---|
| **A — software CE** (today) | `rs: crates/kayfabe-rt/src/ceutils.rs:429` `run_submission` + `rs: crates/kayfabe-rt/src/cpu_ce.rs:315` | B1 … C4 — every wall whose work is a memset, a copy, or a control reply | `IsolatePlane::Stillborn` |
| **B — host forwarding** | `rs: crates/kayfabe-isolate-host/` (linked out today) | B3's GR half, D1 … D4 | `KAYFABE_ISOLATES=real` **and** `--features host-isolates` |

`[src]` The era-A completion writer is honest by construction: `run_submission` calls
`execute_ours_spans` and only then `write_resolved_completion`
(`rs: crates/kayfabe-rt/src/ceutils.rs:585`, then `:600+`); the payload write precedes the IRQ
(`cpu_ce.rs:320-331`); a submission that produced no range is refused by name rather than signalled
(`ceutils.rs:484-489`). ★ **That is the shape §0.2 says the C got wrong. Keep it.**

`[src]` `crates/kayfabe-completion` is a different thing sharing the name: `CompletionQueue`,
`FenceArms`, `DeliveryPlane`. It writes nothing to guest memory, and neither `signal_source` nor
`completion_poll` has a production caller — `rs: crates/kayfabe-rt/src/device.rs:2036-2041`
deliberately does not run the pump edge because the shell's deliverer is unbuilt. ⇒ When era B
arrives, **the honest wait belongs in `kayfabe-completion`, and it is currently reachable from
nowhere** (`a_declared_capability_reachable_from_nowhere`).

---

## 3. The wall list, in the driver's true order

### ─── BOOT PATH (inside `modprobe` / `RmInitAdapter`, before any CUDA process) ───

`[src]` The escalation from any of B1–B3 is identical and terminal:
`mem_mgr.c:4165` → `mem_mgr.c:526 NV_ASSERT_OK_OR_RETURN` → `mem_mgr.c:554` →
`kernel_fifo.c:3129 NV_ASSERT(0); break` → `kernel_fifo_gm107.c:233` →
`gpu.c:3440-3449 goto gpuStatePostLoad_exit` → `gpu.c:2613-2615` →
`osinit.c:1244-1251 RM_SET_ERROR(RM_INIT_GPU_LOAD_FAILED)`.

---

#### B1 — `memmgrTestCeUtils` — the driver's own byte-exact oracle

**1. Guest function.** `[src]` `memmgrPostSchedulingEnableHandler` (`ogkm-580: mem_mgr.c:547-555`)
→ `memmgrInitInternalChannels_IMPL` (`:480-529`) → `memmgrInitCeUtils_IMPL` (`:4106-4166`) →
`memmgrTestCeUtils` (`:407-478`).

**2. Demands**, in order, each under `NV_ASSERT_OK_OR_GOTO(failed)`:

| # | line | demand |
|---|---|---|
| 0 | `:487` | `memmgrScrubHandlePostSchedulingEnable_HAL` — the **scrubber** channel, one frame up, *before* CeUtils |
| 1 | `:4155` | `objCreate(CeUtils)` — a channel, its GPFIFO, pushbuffer and semaphore |
| 2 | `:439-446` | `memdescCreate` + alloc, 4 bytes, `ADDR_FBMEM` |
| 3 | `:450-458` | `memdescCreate` + alloc, 4 bytes, `ADDR_SYSMEM` on a GSP **client** |
| 4 | `:464` | `memmgrMemSet(vid, 0, 4, PREFER_CE)` — **CE submission #1 (a memset)** |
| 5 | `:466` | `memmgrMemWrite(vid, 0xAABBCCDD, 4, NONE)` — **not** CE; BAR2/PRAMIN |
| 6 | `:467` | `memmgrMemWrite(sys, 0x11223345, 4, NONE)` |
| 7 | `:468` | `memmgrMemCopy(sys ← vid, 4, PREFER_CE)` — **CE submission #2 (a copy)** |
| 8 | `:469` | `memmgrMemRead(sys, 4, NONE)` |
| 9 | `:470` | `NV_ASSERT_TRUE(sysmemData == vidmemData)` — **the oracle** |

★ **Two CE submissions of two different kinds.** A build serving the memset and stalling on the copy
looks identical from outside to one serving neither.
⊘ Three legitimate early-`NV_OK` escapes exist — `PDB_PROP_GPU_REUSE_INIT_CONTING_MEM` (`:423`),
`pLiteKernelChannel != NULL` (`:430`), and `hypervisorIsVgxHyper()` upstream at `:511`. **Do not take
the vGPU escape**; it reads as a one-line unblock, changes the whole vGPU posture
(`mem: mode2_vgpu_posture_decision.md` — default bare-metal) and moves the wall rather than removing
it. ⊘ And `memmgrInitInternalChannels_IMPL` **ends** at `:528` — there is no further demand in that
function after CeUtils.

**3. The C.** `[src]` `nvkvm_chan_execute` (`C: …:5791`) walks `[gp_get, GP_PUT)`, decodes the
NVB0B5/NVC7B5 method stream and *performs the op for real* — `memset` fill `C: …:6340-6364`,
`memcpy` `C: …:6418` — then releases the CE semaphore the stream itself named (`C: …:6496-6510`).
Its header says this is what *"makes the scrubber's CE self-verify (mem_mgr.c:469) see real data"*
(`C: …:5789`). ⇒ **PORTABLE.** The `MEMORY_SCRUB` branch (`C: …:6315-6319`) writes nothing because
the emulated FB reads sparse-zero — legitimate only while the fill constant is zero, and **not** a
licence to skip a `remap` fill with a non-zero `SET_REMAP_CONST_A`.

**4. kayfabe.** `[src]` **YES, production-reachable.** `rs: crates/kayfabe-qemu-raw/src/shim.rs:2385`
`try_ce_submission` → `rs: crates/kayfabe-rt/src/ceutils.rs:429` `run_submission` → `CeWork` decode
`rs: crates/kayfabe-chips/src/ga10x.rs:1345` → `rs: crates/kayfabe-rt/src/cpu_ce.rs:315`.

**5. Sharpest boot question.** ⇒ **"Are TWO CE submissions served on the CeUtils channel — one
`Fill`/`Scrub` and one `Copy` — and do the four bytes at the sysmem memdesc read `0xAABBCCDD`?"**
Print the `CeWork` variant per served submission and the destination bytes after each.

---

#### B2 — `_kmigmgrHandlePostSchedulingEnableCallback` — the handler nobody wrote down

**1. Guest function.** `[src]` `ogkm-580: src/nvidia/src/kernel/gpu/mig_mgr/kernel_mig_manager.c:942-1046`.
Runs **immediately after B1**, second in the handler list.

**2. Demands**, in order:

| line | demand | exit |
|---|---|---|
| `:975-977` | `NV_CHECK_OR_RETURN(… NV_WARN_MORE_PROCESSING_REQUIRED)` — a **retry** gate, not a failure |
| `:980-981` | `NV2080_CTRL_CMD_INTERNAL_MEMSYS_SET_PARTITIONABLE_MEM` = **`0x20800a51`** | `NV_CHECK_OK_OR_RETURN` |
| `:989-995` | `NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE` = **`0x20800a4c`** — gated `IS_GSP_CLIENT && !IS_VIRTUAL`, i.e. **exactly us** | `NV_CHECK_OK_OR_RETURN` |
| `:1003-1007` | early `return NV_OK` when MIG is unsupported — **the escape that makes the rest moot** |
| `:1018-1024` | `INTERNAL_MIGMGR_SET_PARTITIONING_MODE` | `NV_CHECK_OK_OR_RETURN` |
| `:1037`, `:1043` | `kmigmgrLoadStaticInfo_HAL`, `kmigmgrRestoreFromPersistence_HAL` | `NV_ASSERT_OK_OR_RETURN` |

★ `[inferred]` The `:1003` escape means only the **first two** controls are unavoidable on a
consumer GA106. `0x20800a51` and `0x20800a4c` are both `INTERNAL` (RM→GSP) controls, so they are
ours to answer as the fake GSP, not to forward.

**3. The C.** Not separately documented; served through the general GSP control path. **UNCLASSIFIED
— no C mechanism identified for these two ids.**

**4. kayfabe.** `[src]` Neither `0x20800a51` nor `0x20800a4c` appears in
`rs: crates/kayfabe-abi/src/capability.rs`; both take `ControlNotPermitted`. **UNBUILT.**

**5. Sharpest boot question.** ⇒ **"Does the guest issue `0x20800a51` and `0x20800a4c`, and does it
reach the `:1003` MIG-unsupported escape?"** If it reaches the escape, this handler costs exactly two
control replies and never needs the MIG machinery — a cheap, permanent scope reduction.

---

#### B3 — `kgraphicsCreateGoldenImageChannel` + `GPU_PROMOTE_CTX` — the GR keystone, at BOOT

**1. Guest function.** `[src]` `_kgraphicsPostSchedulingEnableHandler` (`kernel_graphics.c:464-515`)
→ `:508` `kgraphicsCreateGoldenImageChannel_IMPL` (`kernel_graphics.c:2135-2541`).
⊘ Gates first: `IS_GSP_CLIENT` at `:476-478`; MIG at `:481-482`; `pKernelGraphicsStaticInfo == NULL`
at `:485`; **`gpcMask == 0` at `:486-487`** (§0.9(b)); `pmaQueryConfigs` at `:494`; scrubber-not-ready
→ `NV_WARN_MORE_PROCESSING_REQUIRED` at `:501-505`.

**2. Demands**, in order, each `goto cleanup` unless noted:

| line | demand |
|---|---|
| `:2180-2181` | `rmapiutilAllocClientAndDeviceHandles` — client + `NV01_DEVICE_0` + `NV20_SUBDEVICE_0` |
| `:2292-2293` | **`FERMI_VASPACE_A`** alloc, `hVASpace = 0xbaba0042` |
| `:2314-2315` | **`NV01_MEMORY_SYSTEM`** — pushbuffer, 32 GPFIFO entries |
| `:2327-2328` | **`NV50_MEMORY_VIRTUAL`** — pushbuffer virtual |
| `:2337-2363` | `gpuIsClassSupported(…_CHANNEL_GPFIFO_A/B)` chain; none ⇒ `NV_ERR_NOT_SUPPORTED` |
| `:2404-2406` | **`NV01_MEMORY_LOCAL_USER`** — USERD |
| `:2454-2456` | **`AMPERE_CHANNEL_GPFIFO_A`** alloc — the whole channel chain below runs here |
| `:2489-2493` | `vaspaceReserveMempool(Σ pContextBuffersInfo->engine[i].size)` — soft |
| `:2513-2520` | **`AllocWithHandle(hObj3D, GR_OBJECT_TYPE_3D or _COMPUTE class)`** — triggers `kgrobjConstruct` |
| `:2532-2533` | `Free(hClientId)` — tears the whole thing down again |

`[src]` Inside that last alloc, `_kgrAlloc` (`kernel_graphics_object.c:172-228`) demands, each
`NV_CHECK_OK_OR_RETURN` / `NV_ASSERT_OK_OR_RETURN`:
`:191` `numGpcs > 0` · `:196` `kgrobjSetComputeMmio_HAL` · `:202` `kgraphicsInitializeDeferredStaticData`
(which issues **`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO` = `0x20800a32`** at
`kernel_graphics.c:743-752`) · `:209` `kgraphicsAllocKgraphicsBuffers_HAL` · `:215`
`kgrctxAllocCtxBuffers` · `:218` `kgrctxMapCtxBuffers` · **`:224` `kgrobjPromoteContext`**.

★ **`kgrobjPromoteContext_IMPL` (`kernel_graphics_object.c:50-163`) is the issuer of
`GPU_PROMOTE_CTX`,** at `:135-140`, and **its failure is FATAL** — it aborts the class alloc at
`kernel_graphics.c:2519`, which kills the handler at `:508`, which kills the boot. Export record
`generated/g_subdevice_nvoc.c:310-324` gives `flags = 0x10244` =
`PRIVILEGED | ROUTE_TO_PHYSICAL | ROUTE_TO_VGPU_HOST | GSP_PLUGIN_FOR_VGPU_GSP` ⇒ **the guest CPU-RM
never handles it locally; it always goes out over RPC.** On `NV_OK` it marks each `bInitialize` entry
initialised (`:142-160`); on failure nothing is marked and `:162` returns the status.
⊘ `kgrctxPreparePromoteCtxBuffer:1884-1885` returns early with **no VA promote when the VAS is
externally owned (UVM)** — which is exactly why the C recorded *"every context buffer NONMAPPED with
`va=0`"* for the crashing client `0xc1d00003`
(`[measured]` `mem: mode2_promote_ctx_and_uvm_wall.md`, 2026-06-05, GA106 + 580.159.04).

★ Also hard: `kgraphicsAllocGrGlobalCtxBuffers_TU102:147-148` asserts
`pKernelGraphicsStaticInfo->pContextBuffersInfo != NULL` and reads `engine[…].size/.alignment` from
it ⇒ **`0x20800a32` must return real sizes**, and a zero-filled reply produces zero-sized context
buffers rather than a refusal.

**3. The C.** `[src]` The golden completion is signalled without running anything —
`C: src/qemu/nvkvm_gpu_emul.c:6047-6054`, in its own words:

> *"The golden-image / watchdog / scrubber channels append a `SEM_EXECUTE` RELEASE after their engine
> work to signal completion; `channelWaitForFinishPayload` polls that semaphore. **We honor the
> EXPLICIT release here (translate the SEM addr, write the payload) WITHOUT running the GR/compute
> methods** — per the Phase-B design we never emulate GR, we only signal completion."*

⇒ **⊘ FORGERY on its face**, with a real argument behind it: the guest's golden buffer content is
read only at context-**restore**, and since all real GR runs on the host — whose own golden context
is valid — the guest's copy is never observed (`C: docs/design/mode2_gr_forwarding.md:73-81`).
`[measured]` The host self-maps its own GR context buffers at the guest VAs on the `0xc7c0` forward:
every `back_and_map[ctx0..5]` returns `st=0x51 ALREADY-HOST-MAPPED`, while GPFIFO — which the host
does not auto-map — places at `st=0x0` (`mem: mode2_grctx_privilege_wall.md:13-27`, 2026-06-05, one
`m2exec` run on GA106 + 580).

⇒ **PORTABLE-ONLY-IF-REWRITTEN, with a proof obligation that must be discharged, not assumed.**
Signal only where the work-product is *shown* unobserved; on the negative branch, **refuse by name**.
⊘ Do not port an unconditional `SEM_EXECUTE` honour.

**4. kayfabe.** ★ `GPU_PROMOTE_CTX` is **the one rung on this ladder that is fully joined**: `[src]`
permit `rs: crates/kayfabe-abi/src/capability.rs:701` → shape
`rs: crates/kayfabe-abi/src/versions.rs:1130` → `translate_promote_ctx`
(`rs: crates/kayfabe-rmrpc/src/lib.rs:1598`) → `route_promote_ctx`
(`rs: crates/kayfabe-core/src/promote.rs:277`) → `apply_promote_ctx` (`:340`) → `Gpu::promote_ctx`
(`rs: crates/kayfabe-core/src/gpu.rs:4014`) → **production entry**
`rs: crates/kayfabe-qemu-raw/src/shim.rs:2208`. ⊘ `gpu_promote_ctx.md`'s "two blockers" framing is
stale; the same file's §9 records them resolved.
`[src]` `0x20800a32` is **absent** from `capability.rs` ⇒ `ControlNotPermitted`. The class allocs
(`FERMI_VASPACE_A`, `NV01_MEMORY_SYSTEM`, `NV50_MEMORY_VIRTUAL`, `NV01_MEMORY_LOCAL_USER`,
`AMPERE_CHANNEL_GPFIFO_A`) go through `translate_alloc` (`rs: crates/kayfabe-rmrpc/src/lib.rs:1133`);
`0xc56f` is decoded as `AllocParams::Channel` (`rs: crates/kayfabe-abi/src/versions.rs:1015`).

**5. Sharpest boot question.** ⇒ **"Did this boot reach `kgraphicsCreateGoldenImageChannel` at all,
or return `NV_OK` early — and if it ran, did `GPU_PROMOTE_CTX` return `NV_OK`?"** This is §5 Q0 and
it is the highest-value single question in this document, because §0.9(b) means a *skipped* golden
context and a *successful* one are indistinguishable today, and the difference decides whether GR
compute is reachable at all.

---

#### B4 — the rest of driver load (low risk, listed for completeness)

`[src]` After the fifo callbacks, the remaining `gpuStatePostLoad` demands on a GA102-family part are
thin: `NV2080_CTRL_CMD_GPU_GET_OEM_BOARD_INFO` (`gpu.c:3498-3503`, **failure non-fatal**),
`gpuFabricProbeStart` (`:3512`, skipped under `hypervisorIsVgxHyper`), `gpuIsSystemRebootRequired_HAL`
(`:3516`). Then `gpuInitVmmuInfo` (`gpu.c:2604`, hard) and `memmgrCheckZeroPmaUsage` (`:2681`), then
`RmInitAdapter`'s tail: `krcWatchdogInit_HAL` (`osinit.c:2161`), `kfifoGetUserdBar1MapInfo_HAL`
(`:2213`, hard bail `:2223`), `RmInitPowerManagement` (`:2232`).
⊘ `[src]` None of `OBJUVM`, `KernelHwpm`, `OBJSWENG` — the engines ordered after `KernelFifo` on
GA102 — has a `StatePostLoad` body, so nothing else fires from that loop.

---

### ─── `cuInit` PATH (`UVM_REGISTER_GPU`) ───

`[src]` `uvm.c:1008` → `uvm_gpu.c:3801 uvm_api_register_gpu` → `uvm_gpu.c:2820 init_gpu` (`:1528`).
`init_gpu`'s demands before the channel manager, each `return status`: `get_gpu_caps` (`:1566`),
`alloc_and_init_address_space` (`:1575`), `get_gpu_fb_info` (`:1581`), `get_gpu_ecc_info` (`:1587`),
`get_gpu_nvlink_info` (`:1593`), `uvm_pmm_gpu_init` (`:1599`), `init_semaphore_pools` (`:1607`),
then **`uvm_channel_manager_create` (`:1615`)**.

---

#### C1 — `channel_manager_pick_ces` → `ces_validate` — where zeros are fatal

**1. Guest function.** `[src]` `uvm_channel_manager_create` (`uvm_channel.c:3873-3915`) →
`channel_manager_create_pools` (`:3833-3871`) → `channel_manager_pick_ces` (`:3159-3189`).
⊘ **Correction to a live campaign citation:** `uvm_gpu.c:489` also calls
`nvUvmInterfaceQueryCopyEnginesCaps`, but its enclosing function is `gpu_info_print_ce_caps`
(`uvm_gpu.c:476`) — a `seq_file` debug dump whose failure prints *"unavailable (query failed)"* and
falls through. **That call site is not a wall.** `uvm_channel.c:3172` is.

**2. Demands**, each `goto out`: `:3172` `queryCopyEngines` (§0.4's four call sites) · `:3176`
`ces_validate` · `:3182` `pick_ces`. Then `:3853` `channel_manager_create_ce_pools` (`:3422-3452`)
adds one pool **per set bit in `ce_mask`** (`:3437`, hard `return` at `:3439`).

`[src]` `ces_validate` (`uvm_channel.c:2927-2955`) is the contract, and it is small enough to state
exactly. With `ce_is_usable(cap) == cap->supported && !cap->grce` (`:2913-2923`; plus `secure` only
when Confidential Compute is enabled):

- at least one CE must be usable, else `NV_ERR_NOT_SUPPORTED`;
- **every** usable CE must have `sysmem` set, else `NV_ERR_NOT_SUPPORTED`;
- **every** usable CE must have `p2p` set, else `NV_ERR_NOT_SUPPORTED`.

⇒ `{0,0,0,0}` fails on *"no usable CE"*; a partial fill fails on `sysmem` or `p2p`. This is the
mechanical form of the standing directive *"⊘ refusals by name, never zeros — `{0,0}` caps claims a
CE that can do nothing"*, and it is the driver's own check, the oracle we prefer
(`mem: isolate_the_drivers_own_checks.md`).

★ **Two traps in `queryCopyEngines` itself.** `[src]` (i) The first `0x20800123` passes
`engineList == NULL` and only `engineCount` is read; `portMemAllocNonPaged(4 * engineCount)` at
`:8466` sizes off it, so `engineCount = 0` risks `NV_ERR_NO_MEMORY` at `:8470` and writing a list on
the first call writes through a NULL `NvP64`. The two calls are **not** interchangeable. (ii) Entries
failing `NV2080_ENGINE_TYPE_IS_COPY` or with `COPY_IDX >= NV2080_ENGINE_TYPE_COPY_SIZE` are silently
`continue`d at `:8492-8497` ⇒ **a wrong engine list yields zero usable CEs with `status == NV_OK`**,
and the failure surfaces one function later in `ces_validate` as *"no usable CE"*.

**3. The C.** `[measured]` The Wall-B fix populated `ceCaps[]` and the CE-present bits in the
replayed `GspStaticConfigInfo` (GSP RPC `fn=65`) with host values, and separately dropped a spurious
`CE4` from the device-info table (commit `fe49ffc`)
(`mem: mode2_cuctxcreate_999_diagnosis.md:26-42,51-86`, 2026-06-04). ⇒
**PORTABLE-ONLY-IF-REWRITTEN**: right values, wrong route — a replayed static-info blob is exactly
the shape the empty-row defect lives in (`rs: docs/design/c_rust_trace_differential.md`).

**4. kayfabe.** `[src]` `0x20800123` allowlisted (`rs: crates/kayfabe-abi/src/capability.rs:700`);
`0x20802a02` allowlisted (`:757`); **`0x20802a01` absent** ⇒ `ControlNotPermitted`. No arm for any of
them in `DriverAbiTable::control_params` (`rs: crates/kayfabe-abi/src/versions.rs:1128-1137` models
four shapes only: `SET_PAGE_DIRECTORY`, `GPU_PROMOTE_CTX`, `VASPACE_COPY_SERVER_RESERVED_PDES`,
`UNSET_PAGE_DIRECTORY`). ⇒ all three refused today.

★ **The values need not be derived.** `[measured 2026-08-01, real GA106 GPU-d0913685, driver
580.159.04]` `rs: traces/real_ga106/rmladder_r18_cecaps_real_ga106.txt` records `0x20802a0a`
answering `NV_OK` with 136 bytes beginning `e3 03 e3 03 e2 03 e2 03` — `{0x3e3, 0x3e3, 0x3e2, 0x3e2}`,
byte-identical to the C's Wall-B values from an independent measurement. The same file records the
privilege split that decides method: `0x20802a0b` carries neither privilege bit ⇒ `KERNEL_PRIVILEGED`
⇒ **derive**; `0x20802a0a` carries `NON_PRIVILEGED (0x8)` ⇒ **measure**. ⊘ `0x20802a01`'s own
privilege bit and `capsTbl` bytes are **not** in that capture and must not be assumed from
`0x20802a0a`'s.

**5. Sharpest boot question.** ⇒ **"Which of `queryCopyEngines`'s four call sites does the guest reach
before it stops, and what `engineCount` does the second `0x20800123` carry?"** Print the refusal per
id in issue order. This separates *"never past the count probe"* from *"answered engines, not
`0x20802a01`"* — two different commits — without assuming which id is "the" wall.

---

#### C2 — TSG allocation and the fault-method buffers — a SILENT wall

**1. Guest function.** `[src]` `channel_pool_add` (`uvm_channel.c:2842-2911`) → `tsg_create`
(`:2675`) → `nvUvmInterfaceTsgAllocate` → `nvGpuOpsTsgAllocate` (`nv_gpu_ops.c:6354-6421`) →
`pRmApi->Alloc(KEPLER_CHANNEL_GROUP_A)` (`:6405`) → `kchangrpapiConstruct_IMPL`
(`kernel_channel_group_api.c:49`) → `kchangrpInit_IMPL` (`kernel_channel_group.c:103-360`).

**2. Demands**, in order:

| line | demand | exit |
|---|---|---|
| `kernel_channel_group_api.c:225` | `vaspaceGetByHandleOrDeviceDefault` | `NV_ASSERT_OK_OR_GOTO` |
| `kernel_channel_group.c:246` | **`kchangrpAllocFaultMethodBuffers_HAL`** → `0x20802a08` | `goto failed` **but see §0.9(a) — it cannot fail** |
| `kernel_channel_group.c:264-275` | `kchangrpMapFaultMethodBuffers_HAL` per runqueue | `goto failed` |
| `kernel_channel_group_api.c:408-419` | `AllocWithSecInfo(KERNEL_GRAPHICS_CONTEXT, SKIP_RPC)` — where the GR context object is born |  |
| `kernel_channel_group_api.c:428-435` | `NV_RM_RPC_ALLOC_OBJECT` for the TSG | `:447 goto failed` |
| `kernel_channel_group_api.c:491-496` | **`NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS` = `0xa06c010a`** | `:502 goto failed` — **hard** |

**3. The C.** ⊘ **The oracle is POSITIVELY WRONG here.** `C: src/qemu/mode2_initctrl_ga106.h` carries
`0x20802a08` with `dlen = 0`, which decodes as size 0. ⇒ **⊘ FORGERY-DO-NOT-PORT**, in the specific
sense that decoding an empty capture to zeros is the forgery.

**4. kayfabe.** `[src]` `0x20802a08` **absent** from `rs: crates/kayfabe-abi/src/capability.rs` ⇒
`ControlNotPermitted`. An encoder exists at `rs: crates/kayfabe-abi/src/fmbsize.rs:161` and its only
callers are in `crates/kayfabe-device/tests/ce_fault_method_buffer_size.rs` — declared, reachable
from nowhere. `0xa06c010a` is likewise absent.

**5. Sharpest boot question.** The **size** needs no boot: `[measured 2026-08-01, real GA106, driver
580.159.04]` `rs: traces/real_ga106/fmb_real_ga106.txt` records `params.size = 20480 (0x5000)` and
`kchangrpAllocFaultMethodBuffers bufSizeInBytes=20480 runQueues=2`. ⇒ The boot-worthy question is
**"is `0x20802a08` issued once, or once per channel group?"** — the same trace shows repeats, and per
§0.9(a) `pGpu->ceFaultMethodBufferSize` has no writer anywhere in the tree, so a `ControlNotPermitted`
here is a **hot-path** silent zero, not an init-time one. ★ Wire `fmbsize.rs` to `answer()` **before**
any era-B boot; a 0-length CE fault buffer is a DMA target with a hardware writer.

---

#### C3 — channel allocation, work-submit token, `GPFIFO_SCHEDULE`

**1. Guest function.** `[src]` `channel_create` (`uvm_channel.c:2423-2507`) → `internal_channel_create`
(`:2346-2391`) → `nvGpuOpsChannelAllocate` = `channelAllocate` (`nv_gpu_ops.c:5730-6167`) +
`engineAllocate` (`:6170-6258`).

**2. Demands**, in order: `:5849` GPFIFO phys alloc · `:5864` CPU map · `:5889` error-notifier alloc ·
`:5976` USERD alloc (Volta+) · `:5996` USERD CPU map · `:6029-6035` channel alloc · `:6069`
`kfifoEngineInfoXlate_HAL(RUNLIST)` · `:6086-6095` `MapToCpu` of the control page ·
then in `engineAllocate`: `:6210-6215` `pRmApi->Alloc(device->ceClass)` with
`NVB0B5_ALLOCATION_PARAMETERS` (`goto cleanup_free_engine`) · `:6223` `nvGpuOpsGetWorkSubmissionInfo`
→ **`NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN` = `0xc36f0108`** (`nv_gpu_ops.c:5616-5621`) ·
`:6242-6247` **`NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` = `0xa06f0103`** with `bEnable = NV_TRUE`
(`goto cleanup_free_engine`).

★ `[src]` The token is computed in software from the runlist id and chid
(`kfifoGenerateWorkSubmitTokenHal_GA100`, `kernel_fifo_ga100.c:177-236`, `:226-227`), and returns
`NV_ERR_INVALID_STATE` at `:215-221` if `!kchannelIsRunlistSet` ⇒ **the runlist must be set before
the token is asked for**, which is an ordering constraint on us, not a value we invent.

**3. The C.** `[src]` `nvkvm_m2_doorbell_setup` (`C: …:7966-8053`) allocates `AMPERE_USERMODE_A`
(`0xc561`), maps it, fetches `0xc36f0108` and issues `GPFIFO_SCHEDULE` (`0xa06c0101`) on the host.
⇒ **PORTABLE.**

**4. kayfabe.** `[src]` `0xc36f0108` and `0xa06f0103` are **not** modelled in
`versions.rs:1128-1137`. Doorbell-token encoding is designed
(`rs: docs/design/doorbell_token_encoding.md`, `mode2_doorbell_mapping.md`). Host-side alloc/schedule
is era B and unreachable (§0.6).

**5. Sharpest boot question.** ⇒ **"Does the guest ask for a work-submit token, and does the token it
receives round-trip to the same `(runlist, chid)` on the next doorbell?"** A token mismatch and a
missing ring are indistinguishable at the doorbell.

---

#### C4 — `channel_init` — the FIRST push → doorbell → semaphore wait, and where kayfabe stands

**1. Guest function.** `[src]` `channel_init` (`uvm_channel.c:2518-2572`).

**2. Demands**, in order: `:2528` `uvm_channel_reserve` (hard `return`) · `:2542-2543`
`set_gpfifo_pushbuffer_segment_base` + `write_ctrl_gpfifo` (a **control** GPFIFO entry) · `:2546`
`uvm_push_begin_on_reserved_channel("Init channel")` (on failure `:2551` releases and returns) ·
`:2557` `ce_hal->init(&push)` · `:2561` `host_hal->init(&push)` · **`:2563`
`uvm_push_end_and_wait(&push)`** — rings the doorbell at `channel->workSubmissionOffset` with
`channel->workSubmissionToken` and **spins on the tracking semaphore until the GPU completes**.

★ **This is the wall in §1**, and its shape decides everything downstream: it is a *wait on a
semaphore that something else must write*. ⊘ Note `write_ctrl_gpfifo` at `:2543` — a **control**
GPFIFO entry carries no pushbuffer pointer, so a ring walker that requires one will read it as
malformed rather than as a legitimate entry kind.

**3. The C.** Served by the software path: `chan_execute` parses and executes, then the parsed
`SEM_RELEASE` writes the value (`C: …:6519-6543`). ⇒ **PORTABLE for the data, and see §0.2 for the
ordering — do not port that.**

**4. kayfabe.** `[src]` Production-reachable and currently refusing by name:
`rs: crates/kayfabe-rt/src/ceutils.rs:484-489` `RingBroughtNoEntry`;
`rs: crates/kayfabe-qemu-raw/src/shim.rs:2519` for `CeResolve::NoPublication`;
`rs: crates/kayfabe-rt/src/ceutils.rs:597` refuses a `HostCe` span rather than skipping it.

**5. Sharpest boot question.** Owned by the bench agent (§16.8's next rung). ⊘ This document proposes
nothing about it. ★ The one thing it adds: **check whether entry 0 is a `write_ctrl_gpfifo` control
entry** (`uvm_channel.c:2543`) before concluding the ring is unwritten — a control entry and an
unwritten slot are two different zeros.

---

#### C5 — `nvGpuOpsBindChannelResources` — the second `GPU_PROMOTE_CTX`

`[src]` `nv_gpu_ops.c:10823-10909`: skipped entirely when `resourceCount == 0` (**which is the case
for CE channels**, `:10855`); otherwise `:10874-10879` `kfifoEngineInfoXlate_HAL(RUNLIST→RM_ENGINE_TYPE)`,
`:10883-10889` fills `promoteEntry[].bufferId/.gpuVirtAddr`, `:10891-10896` issues **`0x2080012b`**,
and on `NV_OK` sets `bIsContextBound = NV_TRUE` at `:10901-10904`. Failure is fatal for GPU VA-space
registration. **kayfabe: joined** (see B3). `[inferred]` Because UVM's channels here are CE channels,
this is likely a no-op on the `cup2` path and becomes live only once a GR channel is registered.

---

### ─── `cuCtxCreate` AND COMPUTE (era B only) ───

#### D1 — the `0xc7c0` alloc reply (`c_cuda_ladder.md` Wall A)

`[src]` The guest RM copies `class_size` bytes from the reply payload into the caller's params
buffer, taking the size from `rmapiGetClassAllocParamSize` and **never reading
`rpc_params->paramsSize`** (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11177`, `:11237-11241`).
⇒ the invariant is simply *supply `class_size` valid bytes at the params offset*.
**C:** `nvkvm_m2_shadow_fwd` (`C: …:6749`) issues a real `NV_ESC_RM_ALLOC` through the isolate
(`C: …:7090-7091`) and captures the host's real params (`C: …:7103-7113`) ⇒ **PORTABLE**. ★ Open
rider: echoing is a **restore, not a fill** — but see `mem: echo_is_correct_not_a_stopgap.md` for the
recorded counter-case where echoing is *correct* (nothing writes `0xc7c0`'s `caps`; the hardware
returns `pAllocParms+0x58`, libcuda's own stack pointer, so "filling" it would corrupt it). The
distinction is per-field, not per-policy.
**kayfabe:** `[src]` **UNBUILT as a reachable path.** `0xc7c0` is admitted as `NoDeclaredFacts` and
recorded only (permit `rs: crates/kayfabe-abi/src/capability.rs:1120`; shape
`rs: crates/kayfabe-abi/src/versions.rs:1022`). `plan_engine_object` / `commit_engine_object` /
`forward_engine_object` exist (`rs: crates/kayfabe-fwd/src/lib.rs:2114`, `:2181`, `:2287`; wrapper
`rs: crates/kayfabe-rt/src/device.rs:1778`) with **no production caller** — every call site is in
`tests/`. **Question:** *"Does the host's `0xc7c0` `RM_ALLOC` return `NV_OK`, and do the 16 reply
bytes differ from the request bytes?"* — the second half is answerable by no committed trace, because
`mem: echo_is_correct_not_a_stopgap.md` records the C's shim dumps post-call only.

#### D2 — `MC_SERVICE_INTERRUPTS` (`0x20801702`) — the C says DO NOT PORT its fix

`[measured]` 28× `0x20801702` interleaved with 18× `0x2080012b` during a `cuCtxCreate` hang
(`C: docs/design/mode2_execfwd_keystone_plan.md:283-290`, 2026-06-10). The C **rejected its own fix**
(M8.108's service-zero credits) as a green poll with no work and chose route B, real completion
(`C: docs/design/mode2_cuctxcreate_resume.md:283-321`). `[src]` And its later record is blunt: *"the
current branch's `0x20801702` default reply (echo mask, status=0, no forward) is already the oracle's
clean guest-only behavior; the oracle's M8.108 env-knob maze … is **SLOP — do NOT port it**"*
(`C: docs/design/mode2_execfwd_keystone_plan.md:295-299`). ⇒ **⊘ DO-NOT-PORT** M8.108; **PORTABLE**
for the echo-mask shape, with the completion coming from D3.
**kayfabe:** allowlisted (`rs: crates/kayfabe-abi/src/capability.rs:745`), unmodelled ⇒ refused by
name. IRQ machinery exists and is production-reachable — `Vmm::raise_irq`
(`rs: crates/kayfabe-vmm/src/lib.rs:789`) from `rs: crates/kayfabe-rt/src/cpu_ce.rs:330`, backed by
`rs: crates/kayfabe-qemu-raw/src/shim_unsafe.rs:681`.

#### D3 — the compute completion — ⊘ THE FORGERY, and the trap the brief warned about

`[src]` A compute launch appends `SET_REPORT_SEMAPHORE` (`0x1b00` upper, `0x1b04` lower, `0x1b08`
payload, `0x1b0c` trigger); `cuStreamSynchronize` spins on it. The C writes it at
`C: src/qemu/nvkvm_gpu_emul.c:6546-6572` on `OPERATION == RELEASE`, zeroing the 12-byte timestamp.
⊘ **Note the scope difference from the `finishPayload` write:** `C: …:4296-4298` excludes GR and
user-CE clients; **this parser has no such exclusion**, and per §0.2 it runs at step 1, before the
host doorbell is rung at step 3. ⇒ **⊘ FORGERY-DO-NOT-PORT.**
★ **This is precisely the trap the brief names.** The obvious era-B increment — *"the ring works, now
add the completion tail"* — reproduces `:6546` exactly. `wire the ring FIRST, complete SECOND` means,
concretely: **the completion value must be read back from a location the host GPU wrote**, never
computed from the methods we parsed.
**kayfabe:** `[src]` **UNBUILT, and the right home is empty** — the only writer into guest memory is
`rs: crates/kayfabe-rt/src/cpu_ce.rs:315`, correctly scoped to work the CPU executor performed;
`crates/kayfabe-completion` has no production caller (§2).

#### D4 — bulk HtoD for `cup8` (task #2)

`[measured]` The C moved these bytes with a QEMU `memcpy` (`C: …:6418`), because `m2cexec` was off in
every green run (§0.8). Its correctness mechanism was M5.60: back a `dst_phys=1` range **directly**
as a real host vidmem object (`C: …:6250-6266`), because a physical CE copy has no PTE and no
page-table walk can ever discover it. ⇒ **PORTABLE-ONLY-IF-REWRITTEN** (back-the-destination); **no
oracle at all** for forwarding the copy.
**kayfabe:** `[src]` `CeWork` production-reachable (`rs: crates/kayfabe-chips/src/ga10x.rs:1345`);
`CeExecutor::{HostCe, Ours}` declared (`rs: crates/kayfabe-isolate/src/lib.rs:1244`); host execution
at `rs: crates/kayfabe-isolate-host/src/rm.rs:2930` — **unreachable** (§0.6), and a `HostCe` span is
*hard-refused* at `rs: crates/kayfabe-rt/src/ceutils.rs:597`. ⇒ correct refusal, unbuilt capability.
**Question:** *"For `cup8`'s HtoD, is `dst_phys` set?"* One printed bit decides whether task #2 is a
forwarding problem or a destination-backing problem.

---

## 4. Classification index — every C mechanism named here

| C mechanism | site | class |
|---|---|---|
| software CE method execution (`memcpy`/`memset`) | `C: …:5791`, `:6340-6364`, `:6418` | **PORTABLE** — at B1 it is *required*, because the driver byte-checks |
| `back_and_map` / `back_and_map_sys` / `enum_gr_sysmem` | `C: …:7894`, `:8214`, `:8815` | **PORTABLE** |
| `doorbell_setup` (USERMODE alloc, token, TSG schedule) | `C: …:7966-8053` | **PORTABLE** |
| the host doorbell ring | `C: …:9159`, `:4220` | **PORTABLE** |
| `shadow_fwd` RM alloc replay to the isolate | `C: …:7090-7091` | **PORTABLE** |
| host-CE `LAUNCH_DMA` forward (`m2cexec`) | `C: …:6310-6315` | **PORTABLE** — default-OFF and **never green**; no oracle (§0.8) |
| three-way client-kind predicate | `C: …:2493`, `:5115`, `:5130` | **PORTABLE — and the answer to §0.7** |
| back a `dst_phys` range as a host vidmem object | `C: …:6250-6266` | **PORTABLE-ONLY-IF-REWRITTEN** |
| CE-caps / engine-list values | `mem: mode2_cuctxcreate_999_diagnosis.md` | **PORTABLE-ONLY-IF-REWRITTEN** — right values, wrong route |
| golden-channel `SEM_EXECUTE` honoured without running GR | `C: …:6047-6054`, `:6519-6543` | **PORTABLE-ONLY-IF-REWRITTEN** — needs the unobserved-artifact proof, with a named refusal on the negative branch |
| `finishPayload` write at `gpfifo_va + 0x8004` | `C: …:4296-4350` | **PORTABLE-ONLY-IF-REWRITTEN** — see the note below |
| `chan_sem_wr32` (the central software semaphore writer) | `C: …:5546-5769` | **PORTABLE-ONLY-IF-REWRITTEN** — keep the VA resolution, delete the unconditional write |
| **compute report semaphore `0x1b0c`** | `C: …:6546-6572` | ⊘ **FORGERY-DO-NOT-PORT** |
| `0xFFF500`/`0xFFF504`/`0xFFF508` backdoor | `C: …:3783-3829` | ⊘ **FORGERY-DO-NOT-PORT** — and moot: needs `docs/kernel_patches/mode2_uvm_complete_proof.patch`, which `[measured]` the 2026-07-29 stock-guest ladder did **not** use (`C: docs/BENCH_REBUILD_NOTES.md:336-348`) |
| `m2semval` read-side semaphore injection | `C: …:1336-1344` | ⊘ **FORGERY-DO-NOT-PORT** (default-off) |
| M8.108 `MC_SERVICE_INTERRUPTS` credit maze | M8.108 | ⊘ **DO-NOT-PORT — the C says so itself** (D2) |
| event delivery triggered by `any_completed` | `C: …:4363-4367` | ⊘ **FORGERY-DO-NOT-PORT** *as a completion signal* — causally downstream of our own write, not the host's |
| decoding a `dlen = 0` capture row to zeros | `C: src/qemu/mode2_initctrl_ga106.h` | ⊘ **FORGERY-DO-NOT-PORT** (C2) |

★ **On the `finishPayload` row, because the brief calls it a fabrication and the code is more
interesting than that.** `[src]` `fin_payload` advances only when `nvkvm_chan_execute` actually
consumed GP entries — `c->gp_get == before` takes a `continue` at `C: …:4111`, and the advance at
`:4116` is by the number consumed. The `finishPayload` is the channel-**host** retirement counter,
released per GP entry by the front end and never by a CE method, so advancing it for entries we
really executed is *emulating the channel host*, a device role we legitimately occupy. ⚠ **The hazard
is real but it is elsewhere**: `chan_execute` is documented *"Bounded + fault-safe (bail on any
miss)"* (`C: …:5790`), so a walk that bails mid-pushbuffer while `gp_get` has already advanced
retires an entry whose methods did not all run. ⇒ Port the *counter*, gate it on **completed**
execution rather than **consumed** index, and refuse by name on a partial walk. ⊘ Do not port the
justification at `:4259`; it is the citation refuted in §0.3.

---

## 5. The ordered queue — six questions, in the order they should be spent

**Q0 (era A, and the highest-value question in this file) — did the golden context RUN?**
*"Did this boot reach `kgraphicsCreateGoldenImageChannel`, or take one of
`_kgraphicsPostSchedulingEnableHandler`'s early `NV_OK` returns — and if it ran, did `0x2080012b`
return `NV_OK`?"* Assert positively on `gpcMask != 0` and `pContextBuffersInfo != NULL`
(`kernel_graphics.c:485-487`, `kgraphics_tu102.c:147`). ⇒ §0.9(b) means a skipped golden context and
a successful one are indistinguishable today, and the answer decides whether GR compute is reachable
at all. `[src]` §1 shows this boot got past B1–B4, so **one of the two happened and we do not know
which.**

**Q1 (era A) — B1's two submissions.** *"Are TWO CE submissions served on the CeUtils channel — one
`Fill`/`Scrub` and one `Copy` — and do the four bytes at the sysmem memdesc read `0xAABBCCDD`?"**
⇒ Separates "served the memset, stalled on the copy" from "served neither"; identical today.

**Q2 (design, no boot) — the executor routing key.** Replace `local_ce_is_the_only_executor`
(`rs: crates/kayfabe-qemu-raw/src/shim.rs:2323`) with a per-channel client-kind key ported from
`C: …:2493 / :5115 / :5130`. ⇒ **Prerequisite for every era-B boot**, because §0.7 shows the plane
switch otherwise regresses B1. ⊘ Do not install a real isolate plane before this lands.

**Q3 (design, no boot) — close C2's silent zero.** Wire `rs: crates/kayfabe-abi/src/fmbsize.rs:161`
to `answer()` with the measured `20480`, and allowlist `0x20802a08`. ⇒ §0.9(a): today an unserved
`0x20802a08` is not a refusal, it is a zero-length DMA target and an under-sized BAR2 aperture. The
value is already measured; this costs no hardware.

**Q4 (era A) — C1's issue order.** *"Which of `0x20800123` (×2), `0x20802a01`, `0x20802a02` does the
guest reach before it stops, and what `engineCount` does the second `0x20800123` carry?"*
⇒ Answers §0.4 empirically and shows whether the count-then-fill trap has bitten.

**Q5 (era B, the real one) — D3's negative control.** *"With our own completion writers disabled,
does any byte of the semaphore page change after the ring?"* ⇒ The C ran this control and it came
back **negative**: `[measured]` under `m2hostsem=on` (host as the sole writer) the CE scrubber wait
timed out — `NV_ERR_TIMEOUT memmgrMemSet PREFER_CE @ mem_mgr.c:463` → `RmInitAdapter failed` →
`cuInit 999` (`C: docs/design/mode2_execfwd_keystone_plan.md:246-252`, 2026-06-10). ⊘ **So the C's own
negative control says the host GPU was never writing that semaphore.** A positive result in Rust
would be the first evidence in this campaign that the host writes anything back. ⊘ Until it is
positive, **do not build a completion tail** — that is the trap in §0.2 and D3.

---

## 6. What this document cannot tell you

- ⊘ **It proposes nothing about the current wall** (§1 / §16.8's page-directory address families).
  That rung is the bench agent's and stands unaltered.
- ⊘ **`0x20802a01`'s privilege bit and reply bytes are unmeasured.** `rmladder_r18` covers
  `0x20802a0a` and `0x20802a0b` only; nothing licenses carrying either one's shape across.
- ⊘ **`0x20800a51`, `0x20800a4c` and `0x20800a32` have no measured reply anywhere** — not in the C's
  captured table, not in `traces/real_ga106/`. B2 and B3 name them; neither names a value.
- ⊘ **Whether `0x20800170` (`GET_ENGINES_V2`) is also on the path is unresolved.** `queryCopyEngines`
  uses `0x20800123`; the C's Wall B named `0x20800170`. Both may be true of different callers, and no
  run here separates them.
- ⊘ **Whether kayfabe's current boot RAN or SKIPPED the golden context is unknown** (Q0). Everything
  in D1–D4 is conditional on the answer.
- ⊘ **The ~10–20 ms completion latency the C profiled**
  (`C: docs/design/mode2_userbuf_vidmem_passthrough.md`, m573) is not reconciled with §0.2's
  immediate write. Two readings survive — the latency is the QEMU copy itself, or something does
  serialize — and nothing here decides between them.
- The classification column is a **judgement about portability**, not an experiment. Every ⊘ is
  argued from a cited line; every PORTABLE is argued from a cited line plus the C's own green run.
  Neither is a run on this tree.

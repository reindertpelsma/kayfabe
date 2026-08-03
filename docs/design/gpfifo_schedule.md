# `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` (`0xa06f0103`) — what we perform, and what we do not

Repo task **#177**. Status: **SERVED**, with one half performed here, one half deferred to
the first doorbell, and one half refused by name.

> ⊘ This document exists because the control cannot be judged by its reply. Its params are
> three `NvBool`s, **all `[IN]`** — there is no output field to get right. Answering `NV_OK`
> to an action we did not perform is the failure mode this project already has a vocabulary
> for (`0x20800a6c`, `crates/kayfabe-abi/src/l2evict.rs`), so the argument has to be written
> down before the code, and it has to be falsifiable.

---

## 1. Which control, established from the driver's own source

⚠ Two commands share the name and they are **not** interchangeable. The one on this path was
determined by reading, not by assuming:

| id | class | object | on this path? |
|---|---|---|---|
| `0xa06f0103` `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` | `KEPLER_CHANNEL_GPFIFO_A` | a **single channel** | ★ **yes** |
| `0xa06c0101` `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` | `KEPLER_CHANNEL_GROUP_A` | a **TSG** | no |

`_memmgrMemUtilsScrubInitScheduleChannel` issues `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` on
`pChannel->channelId` (`ogkm-580.159.04:
src/nvidia/src/kernel/gpu/mem_mgr/mem_utils.c:1973-1989`). The TSG form appears nowhere on
it. The scrubber channel has no TSG: `channelSetupIDs` allocates no group and no VASpace for
a physical CE channel.

The `a06c` form *is* what this port sends the **host** — the isolate's own channel does live
in a group (`kayfabe-isolate-host/src/rm.rs`, `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` on
`parts.tsg`). ⇒ **guest asks `a06f` on a channel; host is told `a06c` on a group.** Same
requirement, different objects, and conflating them is why this row's first draft said this
id was "not what this port uses".

### What the guest's kernel half does with it — nothing

```c
NV_ASSERT_OR_RETURN(kchannelIsSchedulable_HAL(pGpu, pKernelChannel), NV_ERR_INVALID_STATE);
SLI_LOOP_START(...) kchannelSetRunlistSet(pGpu, pKernelChannel, NV_TRUE); SLI_LOOP_END
//
// All real hardware management is done in the host.
// Do an RPC to the host to do the hardware update and return.
//
if (IS_VIRTUAL(pGpu) || IS_GSP_CLIENT(pGpu)) {
    NV_RM_RPC_CONTROL(pGpu, ..., NVA06F_CTRL_CMD_GPFIFO_SCHEDULE, ...);
    return rmStatus;
}
```

(`ogkm-580.159.04: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:3085-3131`.) Two local
bookkeeping steps, then it is **ours**. So the entire operation is on our side of the line —
which is exactly what the sweep row said, and exactly why it concluded (wrongly) that
`NV_OK` could not be honest.

### The status, and why `0x56` was never an answer

★★ The documented return set is `NV_OK`, `NV_ERR_INVALID_OBJECT_HANDLE`,
`NV_ERR_INVALID_STATE`, `NV_ERR_INVALID_OPERATION` (`ogkm-580: ctrla06fgpfifo.h:59-64`).
**`NV_ERR_NOT_SUPPORTED` (`0x56`) is not in it.** The bench guest printed

```
NVRM: _memmgrMemUtilsScrubInitScheduleChannel: Unable to schedule channel, status: 56
```

for six weeks, and that `56` is hex — `0x56` — i.e. `NV_ERR_NOT_SUPPORTED`
(`ogkm-580: nvstatuscodes.h:115`), which is what `kayfabe_gsp::GspFsm::answer` posts when
**nobody claimed the command**.

★★★ **The base is not a reading — it is the format specifier, and it is the one line this
whole reinterpretation rests on.** `ogkm-580:
src/nvidia/src/kernel/gpu/mem_mgr/mem_utils.c:1985`:

```c
NV_PRINTF(LEVEL_ERROR, "Unable to schedule channel, status: %x\n", rmStatus);
```

⊘ Cite it, because the alternative reading is not merely wrong, it is *plausible and
different*: decimal `56` is `0x38` = `NV_ERR_INVALID_OPERATION`, which **is** one of the four
documented returns above, and would have meant RM examined the request and rejected it —
the opposite conclusion, reached from the same six characters. A paragraph that warns two
lines below about writing status constants from memory should not itself assert a base
without its source. *(Citation added by the verifier, not the author.)* ⇒ `0x56` on this control is a *signature of absence*, not a
decision. A refusal this port actually decides must therefore not reuse it, or the only
place a human ever reads this failure — the guest's own dmesg — cannot tell "I examined your
channel and declined" from "no code path exists". #177 answers
`NV_ERR_INVALID_STATE` (`0x40`) instead.

⚠ Three status numbers were written from memory in the first draft of this rung and **two
were wrong** (`INVALID_STATE` is `0x40`, not `0x39`; `INVALID_OBJECT_HANDLE` is `0x33`, not
`0x1e`). They were caught by opening `nvstatuscodes.h`. A plausible-looking constant is not a
sourced one.

---

## 2. What "performing it" means here — the argument, before the code

RM is buying one postcondition: *work submitted to this channel executes.* Decompose it:

| | claim | ours? |
|---|---|---|
| **P1** eligibility | a submission on this channel is **accepted** rather than dropped | ★ yes, and performable |
| **P2** execution | the work is actually run by an engine | deferred — see §3 |
| **P3** scheduling semantics | runlist ordering, timeslice, interleave, preemption, and the *enabled-vs-scheduled* split | ⊘ not modelled at all |

### P1 — performed, and made falsifiable

`kayfabe_core::gpu::ExecPlane` gains **`requested`**, written only by
`Gpu::schedule_channel`, i.e. only by this control. `kayfabe_fwd::plan_doorbell` now refuses
a doorbell on a channel that is not in it:

```rust
FwdFault::NotScheduled { chan, vchid }
```

★★★ **This is the whole rung.** Before it, `plan_doorbell` read
`!proc.exec.scheduled.contains(&cid)` as a *memo* and scheduled an unscheduled channel on the
fly — so serving `0xa06f0103` would have had **nothing to perform**, and its `NV_OK` would
have been unfalsifiable by construction. With the gate the control has an observable: the
same doorbell is refused before it and planned after it.

⊘ The two sets are deliberately **not** one bit:

| set | written by | means |
|---|---|---|
| `requested` | the guest's control | "the guest declared this channel runnable" |
| `scheduled` | `commit_doorbell` | "a host runlist submit actually happened" |

Collapsing them would make "we recorded an intent" and "a host GPU accepted it"
indistinguishable, which is `refusal_invisible_in_the_ledger`'s failure with the labels
swapped.

### P2 — deferred to the first doorbell, and the deferral is argued, not assumed

★ **Argument 1 (observational).** Between this control returning and the first doorbell on
the channel, **no work can execute on it**: a GPFIFO channel runs only what `GP_PUT`
advertises, and only once the host is told. So *"on the runlist now"* and *"on the runlist by
the time the first submission is seen"* are indistinguishable to the guest. RM's own next two
steps — `_memmgrMemUtilsScrubInitRegisterCallback`, then
`kfifoRmctrlGetWorkSubmitToken_HAL` (`ogkm-580: mem_utils.c:2022-2027`) — probe neither.

★ **Argument 2 (the oracle).** ⚠ Split at the epistemic level actually held, because the two
halves are not the same kind of thing: a **reading** of `C: src/qemu/mode2_initctrl_ga106.h`
and `C: src/qemu/nvkvm_gpu_emul.c`, and a **run** somebody else made on a rebuilt bench on
2026-07-29 (`docs/BENCH_REBUILD_NOTES.md`).

*Read* (a citation into the artifact, no run behind it): the C answers this exact
id `NV_OK` from its captured table (`C: src/qemu/mode2_initctrl_ga106.h:6234` — row
`{0xa06f0103u, 0x0u, 3u, 0u, ctl_a06f0103}`), and it performs the host-side schedule **at the
first doorbell** rather than at the control (`C: src/qemu/nvkvm_gpu_emul.c:8038-8048` M5.8,
`:4176-4194` M5.25).

*Measured*, by the C campaign and not by this rung: that architecture carried a **stock,
unpatched** NVIDIA driver to `cuCtxCreate → 2048² matmul` at `bad=0 maxerr=0` on a rebuilt
bench, 2026-07-29 (`nvidia-gpu-passthrough/CLAUDE.md`, the `cup2 → cupctx2_min → cup8 →
cup8_iter` ladder; `docs/BENCH_REBUILD_NOTES.md`).

⇒ What the two together support is narrow and worth stating narrowly: **an implementation
that deferred this control's host-side act to the doorbell was accepted end to end by a real
driver.** They do not show that deferring is the only correct choice, and they say nothing
about this port's own code.

★ **Argument 3 (hardware, `[measured]`).** A real GA106's own GSP answers this command
`gspst=0x0` with the three params bytes echoed —
`traces/real_ga106/rpc_transcript_real_ga106.txt:59`, `cmd=0xa06f0103 psize=3 gspst=0x0
head=01 00 00`. ⊘ The reply body is taken from **there** and not from the C's row, which is
`dlen = 0` — one of the eleven empty rows the FIFTH LIMIT contradicts. The C's *status* is
corroborated by hardware; its *body* is unmeasured, and an empty capture is evidence of
nothing.

### P3 — refused by name

`bSkipSubmit` / `bSkipEnable` separate "in the runlist" from "will actually be run"
(`ogkm-580: ctrla06fgpfifo.h:44-55`). This port's state is a single membership with no third
value. Either flag set is
`kayfabe_abi::submit::GpfifoScheduleError::UnmodelledSkip` → `NV_ERR_INVALID_STATE`.

⊘ Serving them by ignoring them would be the silent-`NV_OK` failure with extra steps: the
guest would have asked for a channel that is scheduled and *not* submitted, and been given
one that is both. Runlist **ordering** (timeslice, interleave, preemption) is not modelled
either, and no control for it is served.

---

## 3. ⊘ What is still false, named rather than buried

**The first channel this control is ever asked about cannot presently be executed on, and we
know that at the moment we answer `NV_OK`.**

RM allocates the global CeUtils scrubber with `hVASpace = NV01_NULL_OBJECT`, deliberately:

```c
// For physical CE channels, we will use RM internal VAS to map channel buffers
NV_ASSERT(pChannel->hVASpaceId == NV01_NULL_OBJECT);
if (bUseVasForCeCopy || (IS_GSP_CLIENT(pGpu) && bMIGInUse)) { ...allocate one... }
```

(`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:86-93`.) On this boot
`bUseVasForCeCopy` is **false** — confirmed two ways: the guest never prints *"Unable to bind
Channel"*, and `0xa06f0104` never reaches the unserviced ledger (control boot `ctl177a`,
17 distinct unserviced ids, `0xa06f0103` present and `0xa06f0104` absent). So
`project::resolve_channel_vas` finds no VASpace, `ChannelFacts::vas_pdb` is `None`, and the
first doorbell on this channel refuses `FwdFault::NoVas`.

⇒ The honest statement of what `NV_OK` promises here is: **"you are eligible; the runlist
submit happens at your first doorbell; and if it cannot, you get a named refusal, not a
hang."** For this particular channel it currently cannot. That is **unbuilt** (E5 is partial
— the address table is populated only to a root page), not impossible: RM's internal VAS is
built by RPCs we can observe.

★ Three named futures that would make §2's licence false:

1. **A host-side schedule that can fail after we said `NV_OK`.** It must surface as a named
   doorbell refusal, never a hang. It does today.
2. **A consumer that probes runlist membership between the control and the first doorbell.**
   Argument 1 dies immediately. Nothing on RM's `RmInitAdapter` path does; a future one
   might.
3. **A channel that is rung by something other than a guest doorbell** — the gate is on
   `plan_doorbell`, and a second submission path would bypass it.

---

## 4. The boot — `[measured]` 2026-08-03 on `vb` (vast 46494693, RTX 3060 GA106, host
   580.159.04 open)

Two boots, one fresh QEMU each, same box, same guest image, taken an hour apart. The
binaries' embedded `kayfabe-rev` was read out of the hypervisor itself (`strings
qemu-build/qemu-system-x86_64 | grep kayfabe-rev`), never from `BUILD_REV.txt`.

| | **control** `ctl177a` | **treatment** `trt177a` |
|---|---|---|
| archive rev *in the binary* | `9dcd5caa5543…` | `1ed29650962c…` |
| guest's own wall | `mem_utils.c:2006` — *"Unable to schedule channel, status: 56"* | ★ `mem_utils.c:2022` — *"event notification control failed"* |
| commands decoded | 92 | **95** |
| unserviced | 20, **17 distinct** | 19, **16 distinct** |
| `0xa06f0103` in the ledger | ★ **present** | ★ **absent** |
| verdict line | `RmInitAdapter failed! (0x25:0xffff:1249)` | `RmInitAdapter failed! (0x25:0xffff:1249)` |

★★★ **The boot moved.** The scheduling failure is gone from the guest's dmesg, the guest
issued **three more commands** than it could before, and the unserviced set lost exactly one
**member** — `0xa06f0103` — with every other id unchanged. (Membership, never cardinality:
`unserviced.rs`'s own rule.)

⊘ **It did not move far, and the distance was predicted.** The sweep row already recorded
`[measured]` boot `schedprobe1` (`0bf7eb7` + a throwaway serve arm, never landed): a bare
`NV_OK` moves the wall *exactly one step*, `:2006` → `:2022`. This rung lands on the same
step. ⇒ **the boot's motion cannot distinguish a performed schedule from a fabricated one**,
which is why the falsifiable claim in §2 is the doorbell gate and its mutation (`M1`,
`scripts/bite_gpfifo_schedule.py`) rather than this table.

### ★★ The new wall is INVISIBLE IN THE LEDGER, and that is the finding worth carrying

`_memmgrMemUtilsScrubInitRegisterCallback` issues `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION`
for `NV2080_NOTIFIERS_FIFO_EVENT_MTHD` — index **35**, action `REPEAT` (`ogkm-580:
mem_utils.c:1918-1930`; and a real GA106 sends exactly that, `cmd=0x20800301 psize=20
head=23 00 00 00 02 00 00 00`, `traces/real_ga106/rpc_transcript_real_ga106.txt:60`).

`0x20800301` is **served** — it is in `WantedTable::ALL` — and `InitTablePolicy` **refuses**
it, because 35 is not in `eventnotify::SILENT_NOTIFIERS` (which carries only
`POWER_RESUME`). `refuse()` returns `Some(Reply)`, so the chain terminates and the
`UnservicedLedger` never sees it:

> the treatment boot reports `bridge refusals: 0` and 16 distinct unserviced ids, and
> `0x20800301` is in **neither** list. The only place this wall exists is the guest's own
> dmesg.

⇒ This is `refusal_invisible_in_the_ledger` reproduced live, on the very next rung. Anyone
picking the next control by diffing unserviced ledgers will not find it.

★ It is also the **next member of the set this port already named**: `sweep.rs`'s `0xa06f0104`
row calls `0xa06f0103` (schedule), `0xa06f0104` (bind), `0xc36f0108` (token) and the index-35
arming *"ONE requirement asked four times"*. One is now spent. ⚠ And the set is smaller than
it looked on **this** boot: `0xa06f0104` is never issued at all (`bUseVasForCeCopy` is false —
§3), so three remain, not four.

## 5. Where the code is

| what | file |
|---|---|
| decode / encode / refusal vocabulary | `crates/kayfabe-abi/src/submit.rs` (`decode_gpfifo_schedule`, `GpfifoScheduleError`, `encode_gpfifo_schedule`, `GPFIFO_SCHEDULE_REFUSED_STATUS`) |
| the state | `crates/kayfabe-core/src/gpu.rs` (`ExecPlane::requested`, `Spine::by_chan`) |
| route / act | `crates/kayfabe-core/src/gpu.rs` (`route_schedule_channel`, `apply_schedule_channel`, `Gpu::schedule_channel`) |
| ★ the gate | `crates/kayfabe-fwd/src/lib.rs` (`plan_doorbell`, `FwdFault::NotScheduled`) |
| the policy | `crates/kayfabe-rmrpc/src/policy.rs` (`OBJECT_CONTROLS`, `ObjectPolicy::respond_control`) |
| the shell's two ranks | `crates/kayfabe-rt/src/device.rs` (`SharedDevice::schedule_channel`) |
| the corrected triage row | `crates/kayfabe-device/src/sweep.rs` (`cmd: 0xa06f_0103`) |
| tests | `tests/tests/gpfifo_schedule.rs` |
| mutation harness | `scripts/bite_gpfifo_schedule.py` |

⚠ `ObjectPolicy` claims `RmControl` **by command id** (`OBJECT_CONTROLS`), never by function.
Claiming the function would make it answer every control in the port, and because
`PolicyChain::respond` is a `find_map`, the `UnservicedLedger` at the end of the chain would
go permanently silent — the port's primary instrument for *"what has the guest asked for that
we do not answer"*.

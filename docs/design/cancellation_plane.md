# The cancellation plane — what NVIDIA's cancel verbs promise, and what we may honestly answer

> **STATUS: LIVE — 2026-08-14 (w306).** Design. ⊘ **No bench** — both are held by sibling lanes, so
> **nothing here was booted.** Every claim is read off `ogkm-580.159.04`, kayfabe source at master
> `6d501933`, or **committed traces**, and each says which. Where a claim needs a boot it says so and
> is **not** asserted.
>
> ★ One thing was **built** (§7): the plane's discipline as a checked table. **No verb's answer
> changed.** §8 says why the one verb-level increment that would be genuinely honest is
> *deliberately* not built here.
>
> Supersede in place. Parents — `docs/audits/w301_cancellation_error_leaks.md` (the census this
> starts from; §0.4 and §1 correct it), `docs/design/blocking_and_completion_model.md` (the
> `INLINE-SAFE` predicate), `docs/design/road_to_v1_after_cup2.md` §0 (the completion rule).

---

## 0. ★★★ Five things predicted going in that the measurement refutes

Stated first, because four of them change what gets built.

### 0.1 ⊘ "The `bImmediate=TRUE` forms are pure de-scheduling, so tier 2 and cheap"

**False for `STOP_CHANNEL`, which is the verb this plane turns on.** The header's own words
(`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrla06f/ctrla06fgpfifo.h:216-243`):

> *"Stopping the channel here means **disabling and unbinding the channel and removing it from
> runlist**. … `bImmediate` — If `NV_FALSE`, we will **wait for default RM timeout for channel to
> idle**. If `NV_TRUE`, we **don't wait** for channel to idle. **If channel is not idle, we
> forcefully preempt it off the runlist. If the preempt times out, we will RC the channel.**"*

⇒ `bImmediate=TRUE` is not "no hardware postcondition". It is "do not wait politely first" — it
still preempts, still ends with the channel off the runlist, still RCs on failure. **Both forms name
host hardware state we cannot produce.** The `TRUE`/`FALSE` axis is therefore *not* the axis that
decides tier. §2 gives the axis that does.

### 0.2 ⊘ …and `bImmediate` is not a variable in production at all

`[read, ogkm-580]` UVM's only call site passes `va_space->user_channel_stops_are_immediate`
(`kernel-open/nvidia-uvm/uvm_user_channel.c:788`). That field is written in **exactly one place in
the tree** — `kernel-open/nvidia-uvm/uvm_test.c:89`, a test ioctl — and `uvm_va_space_t` is
`uvm_kvmalloc_zero`'d (`uvm_va_space.c:181`).
⇒ **Production always sends the DRAINING form.** The cheap form is test-only.
★ *Known-positive for that zero:* the same grep returns the declaration (`uvm_va_space.h:382`) and
the read site, so it is not a blind search.

### 0.3 ★★★★★ …and the drain verbs do **not** need tier 3 — the completion has no consumer

The brief's central prediction — *"the ones that [drain] are tier 3 and need the deferred-completion
path"* — is **refuted twice over**, and this is the finding the whole design turns on (§3).

1. **The guest throws the status away.** `nvGpuOpsStopChannel` is `void`, wraps the control in
   `NV_ASSERT_OK(...)` — which `src/nvidia/inc/libraries/utils/nvassert.h:467-473` expands with
   *"no other action"* — and then sets `pKernelChannel->bIsContextBound = NV_FALSE`
   **unconditionally** (`nv_gpu_ops.c:10956-10964`). `uvm_user_channel_stop` is `void` and sets
   `atomic_set(&user_channel->is_bound, 0)` regardless (`uvm_user_channel.c:787-791`).
2. **There is no transport to defer into.** `CommandPolicy::respond` returns `Option<Reply>`
   (`crates/kayfabe-gsp/src/boot.rs:349-356`, `Reply` at `:336-343`) — **answer now** or **decline**,
   no third state — and the alternative was considered and rejected *in source*
   (`boot.rs:1296-1301`): *"needs a queue of deferred replies, and a deferred reply is state that
   can be lost, reordered or double-sent."*

⇒ **Tier 3 is neither available nor needed here.** The real blocker is elsewhere — §2.4.

### 0.4 ⊘ The w301 census is incomplete, and the missing verb arrives exactly as often as `STOP_CHANNEL`

`NV2080_CTRL_CMD_GPU_EVICT_CTX` (`0x2080012c`) is **not in w301 §1.2's table** and belongs there.
`[measured 2026-08-14, w306]` over the **195 committed boot logs that print an unserviced ledger**
(`traces/{boots,guest_boots,w294_cudalimit,w297_cup3,w298_ablation,w299_multiproc}`, 202 qemu logs of
which 195 reach a ledger):

| id | ledgers containing it |
|---|---|
| `0xa06f0112` `STOP_CHANNEL` | **115 / 195** |
| `0x2080012c` `GPU_EVICT_CTX` | **115 / 195** |
| **both / only-STOP / only-EVICT** | **115 / 0 / 0** |

★★ The perfect co-occurrence is not luck — it identifies the caller exactly. `nvGpuOpsStopChannel`
issues those two controls back to back (`ogkm-580: nv_gpu_ops.c:10956-10962` then `:10966-10983`),
the second gated on `channelEngineType == UVM_GPU_CHANNEL_ENGINE_TYPE_GR`. **Zero singletons across
115 boots means every one of those boots stopped a GR channel through that one function.**

And `GPU_EVICT_CTX` is the same structural class as `PREEMPT`: flags **`0x1c240`**
(`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:326-332`) carry `ROUTE_TO_PHYSICAL` (`0x40`)
without `PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST` (`0x40000`), so by
`NVOC_EXPORTED_METHOD_DISABLED_BY_FLAG` (`control.h:159-162`) `subdeviceCtrlCmdGpuEvictCtx_IMPL` is
`NULL` in the export table and **no body exists under `src/nvidia/src/`**. It is delegated wholly to
the GSP, and **we are the GSP**.
★ *Known-positive:* the identical search over its mirror `subdeviceCtrlCmdGpuPromoteCtx_IMPL`
(`0x2080012b`, flags `0x10244`) also finds no body — **and that one we do serve**, as
`ObjectPolicy::respond_promote_ctx`. ⇒ *the exact inverse of a verb we already implement is a verb we
refuse.* ⊘ It is not even unknown to us: `0x2080_012c` is used as a **negative fixture**
(*"not in the table at all"*) at `crates/kayfabe-abi/tests/mean_wire.rs:1882`.

### 0.5 ⊘ Two smaller census corrections

- **`RESET_CHANNEL` (`0x906f0102`) HAS reached us.** w301 §1.2 says *"never observed reaching us."*
  `[measured, w306]` it is in **1 / 195** ledgers — `traces/w299_multiproc/run_w299concurrent_qemu.log.gz`,
  the **multi-process concurrent** boot, listed adjacent to `0xa06f0112` and `0x2080012c`. ⇒ the
  cancellation surface **widens as the ladder gets further**, which is the ranking fact.
- **`NV2080_CTRL_CMD_FIFO_CHANNEL_PREEMPTIVE_REMOVAL` (`0x2080110a`)** is documented
  (`ctrl2080fifo.h:281-292`: *"removes the specified channel from the associated GPU's runlist and
  then initiates RC recovery"*) but has **no export entry at all** in 580.159.04. ★ *Known-positive:*
  the identical grep for its neighbour `0x2080110b` hits `g_subdevice_nvoc.c:4923`. ⇒ nobody can
  issue it; it is not a verb we need. Recorded so it is not re-derived.

---

## 1. The verb table — contract, HAL binding, what we do, what honest looks like

⚠ **HAL resolution is part of every row.** In ogkm the `.c` you read is often not the code that
runs: a control carrying `ROUTE_TO_PHYSICAL` (`0x40`) without `PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST`
(`0x40000`) has its CPU-RM `_IMPL` replaced by **`NULL`** in the export table
(`ogkm-580: src/nvidia/inc/kernel/rmapi/control.h:159-162`) and is auto-forwarded to GSP. **We are
the GSP**, so "routed to GSP" means *we* must perform it.

| verb (id) | contract — what it PROMISES | CPU-RM `_IMPL` on GA106+GSP? | arrives? `[w306, 195 ledgers]` | kayfabe today | honest implementation |
|---|---|---|---|---|---|
| **`STOP_CHANNEL`** `0xa06f0112` | **NOT RUNNING.** *"disabling and unbinding … removing it from runlist"*; `bImmediate=FALSE` *"wait for default RM timeout for channel to idle"* (`ctrla06fgpfifo.h:216-243`). Also contractually *"set an error notifier to notify user space that channel is stopped"* | **YES**, flags `0x10008` (`g_kernel_channel_nvoc.c:391`). Body = one `NV_RM_RPC_CONTROL` (`kernel_channel.c:1958-1968`) then, **only on `NV_OK`**, `kchannelNotifyRc_HAL` (`:1979`). **The drain is 100 % behind the RPC, i.e. ours** | **115/195** | **NOT BUILT** — zero id hits in `crates/`; blanket `0x56` + ledger (`crates/kayfabe-device/src/lib.rs:1251` → `unserviced.rs:309`) | forward to the host twin (`relay_channel_control`, §2.3) — **but see §2.4: it does not fit inline** |
| **`GPU_EVICT_CTX`** `0x2080012c` | **NOT RUNNING** — *"evict a Virtual Context"* (`ctrl2080gpu.h:1003-1022`); evicting a GR context requires it be switched out | **NO** — flags `0x1c240` ⇒ compiled out. Pure GSP verb | **115/195**, always paired with the row above | **NOT BUILT** (`0x56`); present only as a negative fixture | needs a GR-context eviction we have no verb for. **Refuse by name** |
| **`RESET_CHANNEL`** `0x906f0102` | **NOT RUNNING** — *"resets the channel corresponding to specified engine and also resets the specified engine"* (`ctrl906f.h:105-148`). ⊘ No idle/preempt language: it is the **post-RC recovery** verb, not a graceful drain | **YES**, flags `0x10008` (`g_kernel_channel_nvoc.c:271`). Writes `bIsRcPending` **OUT** before the RPC (`kernel_channel.c:3056`), then verbatim RPC. Body's own comment: *"All real hardware management is done in the host"* | **1/195** (w299 multiproc) | **NOT BUILT** — allowlist row only (`crates/kayfabe-abi/src/capability.rs:859`), and the capability gate is **not on this path** (w301 §1.1) ⇒ still `0x56` | channel-scoped ⇒ **forwardable today** via `relay_channel_control` (§2.3). Bounded? unmeasured |
| **`FIFO_DISABLE_CHANNELS`** `0x2080110b` | **BOTH, caller's choice** — verbatim (`ctrl2080fifo.h:301-337`): `bOnlyDisableScheduling=FALSE` ⇒ *"ensure **none of the listed channels are running in hardware**"*; `=TRUE` ⇒ *"none … **can be scheduled** … but will not remove any … if they are currently running."* `bRewindGpPut` ⇒ RAMFC `GP_PUT := GP_GET`. Default is preempt-off | **YES**, flags `0x10108` (`g_subdevice_nvoc.c:4921`) — but the body is a priv check on `pRunlistPreemptEvent` then a verbatim RPC, and the **bare-metal branch is `NV_ERR_NOT_SUPPORTED`** (`kernel_fifo_ctrl.c:714-750`) ⇒ *this control only exists via GSP* | **0/195** | **NOT BUILT** — allowlist row only (`capability.rs:784`) ⇒ `0x56` | ⚠ **the one caller that CONSUMES the status** (§3.3). Needs a group/VAS-scoped host verb. Refuse by name |
| **`DISABLE_USERMODE_CHANNELS`** `0x20801117` | **NO NEW WORK** — *"disable or enable scheduling of all usermode channels"* (`ctrl2080fifo.h:831-849`), one `NvBool`, no channel list | **NO** — flags `0x40` (`g_subdevice_nvoc.c:5056`), no body anywhere. Pure GSP verb. Callers = `rm_stop_user_channels` / `rm_restart_user_channels` (`osapi.c:2671-2727`), the suspend/resume + GPU-reset gate | **0/195** | **NOT BUILT** — not even allowlisted | a global doorbell gate. Cheap in principle; unreached, so **do not build on spec** |
| **`PREEMPT`** `0xa06c0105` (TSG) | **NOT RUNNING when `bWait=TRUE`** — *"waits till the preempt completes"*; `bWait=FALSE` returns before it lands, and *"calling … multiple times with `bWait = NV_FALSE` … can lead to undefined results"* (`ctrla06c.h:177-213`). `MAX_MANUAL_TIMEOUT_US = 1 000 000` | **NO** — flags `0x10248` ⇒ compiled out | **0/195**; `[w303]` also absent from both crossing boots | ★ **BUILT + REACHABLE and it DECIDES** (w303): `respond_preempt`, `crates/kayfabe-rmrpc/src/policy.rs:2252-2345`. `NV_OK` iff `GroupHostTwins.with_host_twin == 0`; else/unroutable `PREEMPT_UNPERFORMED_STATUS` = `NV_ERR_INVALID_STATE` | forward on the host TSG. ⊘ **Not forwardable by today's relay** — it is group-scoped (§2.3) |
| **`GPFIFO_SCHEDULE(bEnable=FALSE)`** `0xa06f0103` / `0xa06c0101` | **NO NEW WORK** — *"the channel will be **disabled and removed from runlist**"* (`ctrla06fgpfifo.h:35-65`); no idle, preempt or wait language. ⊘ RM's own `DISABLE_CHANNELS` text is explicit that removal-from-runlist ≠ not-running | **YES**, flags `0x10008`; ⊘ **the body does not branch on `bEnable` at all** (`kernel_channel.c:3105-3131`) | present in both crossing boots | guest plane **BUILT + REACHABLE** (`apply_schedule_channel`, `crates/kayfabe-core/src/gpu.rs:671`; group form `:936`; enforced at `crates/kayfabe-fwd/src/lib.rs:3432-3436` `FwdFault::NotScheduled`). **Host plane NOT BUILT** — `RmBackend::schedule` has **no enable flag** (`crates/kayfabe-isolate/src/lib.rs:887`) and `b_enable: 1` is hardcoded (`crates/kayfabe-isolate-host/src/rm.rs:4595`) | ★ **the cheapest real increment on the whole plane** — §2.5 |
| **`RC_*` family** `0x208022xx` | not cancellation. `…05/06/13` read the error list; `…07` wipes history; `…0a/0b/0c/10` are watchdog **policy** votes | mixed; `0x20802207` is `0x44` ⇒ GSP-only | 0/195 | 2 allowlist rows only | **nothing here stops an engine** — recorded so it is not re-derived |
| **`RC_AND_PERMANENTLY_DISABLE_CHANNELS`** `0x00802008` | client-scoped kill switch — *"RC and disable channels permanently for the given clients"* (`ctrl0080internal.h:91-115`) | **no export entry**; physical/GSP-only internal, issued by the IMEX/fabric path (`compute/imex_session_api.c:52-71`) | 0/195 | NOT BUILT | not reachable from our guests. Out of scope |
| ★ **`FIFO_IDLE_CHANNELS`** `0x00801714` | ★★ **the only verb whose text says "waits for pending work to complete"** — *"**idles (deschedules and waits for pending work to complete)** channels belonging to a particular device"*, with a **caller-supplied `timeout`** and `NV_ERR_TIMEOUT` in its status set (`ctrl0080fifo.h:386-421`) | **YES**, flags `0x109`; CPU-RM takes the GPU lock **itself** rather than using `ROUTE_TO_PHYSICAL`, then RPCs the physical RMAPI (`kernel_fifo_ctrl.c:102-158`). Second entry point `NV_ESC_RM_IDLE_CHANNELS = 0x41` → RPC **fn 22** | **0/195** — and **0** in every native libcuda trace (§1.1) | NOT BUILT | ★ *if* a guest ever asks, this is the one verb that tells us **how long it is willing to wait** — the only negotiated deadline in the whole set |

### 1.1 ⊘ A measurement I got wrong first, and the correction is the useful half

I first reported `NV_ESC_RM_IDLE_CHANNELS` as *"issued exactly once per run by real libcuda"* from
`traces/host_reference_ga106/*.jsonl.zst`, matching `"nr":65`. **That is wrong.** Those records carry
`"dev":"nvidia-uvm"` and `"uvm":"MAP_DYNAMIC_PARALLELISM_REGION"` — **UVM ioctl 0x41, a different
namespace that happens to share the number** with `NV_ESC_RM_IDLE_CHANNELS`.

Filtering on `dev ∈ {nvidiactl, nvidia0}`: **`nr=65` appears ZERO times** in `init_r1`, `dev_r1`,
`ctx_r1`, `alloc_r1`, `launch_r1`, `ce_r1`.
★ *Known-positive for that zero, and it is unusually strong:* the RM-escape histogram of the same
file is rich and non-empty (`nr` 42 ×197, 41 ×98, 43 ×92, 78 ×25, 79 ×23, 214 ×2, 200 ×2, 94 ×1), and
**the value 65 does occur in the file — on the other device.** So the filter is live, and the thing
that would have produced a false positive is present and was excluded by name.

⇒ This is `an_ioctl_number_is_not_a_length_because_its_bits_parse_as_one` in its other form: **a
number is not an identity until the namespace is named.**

---

## 2. ★★★ The split — and it is not drain-vs-no-drain

The brief asked for the verbs to be split by *"whether they need a drain"*. That axis does not
survive §0.1: **both** `STOP_CHANNEL` forms, `RESET_CHANNEL`, `PREEMPT(bWait)` and
`DISABLE_CHANNELS(FALSE)` all promise host hardware state, and none of them is cheap for us. The axis
that decides is:

> **(A)** what the verb lets the caller *conclude* — `NoNewWork` or `NotRunning`; and
> **(B)** how we could *discharge* that conclusion — **vacuously**, **by forwarding**, or **not at
> all**; and
> **(C)** whether the discharge fits `INLINE-SAFE`.

### 2.1 Tier V — VACUOUS. `NV_OK` is a true statement. **Cheap and already built once.**

Work reaches the host GPU through exactly one door: a **host channel** born for a guest channel and
rung through `RmBackend::ring_doorbell`. A channel or group with **no host twin** has never submitted
a byte to hardware, so *every* verb in the table is vacuously complete for it — `NotRunning` included.

**BUILT + REACHABLE for `PREEMPT` only**: `respond_preempt` (`policy.rs:2294-2307`) via
`census_group_host_twins` (`crates/kayfabe-core/src/gpu.rs:901`), seated in production at
`crates/kayfabe-qemu-raw/src/shim.rs:3067-3073`. ⊘ The asymmetry is deliberate and stated at
`gpu.rs:868-880`: `with_host_twin > 0` means *"we cannot prove it is idle"*, **never** *"work is
executing"*.

★ This tier extends to `STOP_CHANNEL`, `RESET_CHANNEL` and `GPU_EVICT_CTX` for **one added routing
helper** (a channel-scoped `census_channel_host_twin` over `route_schedule_channel`,
`gpu.rs:646-662`). **It is not built here** — §8 says why.

### 2.2 Tier D — DE-SCHEDULE, forwarded. `NoNewWork` is a promise about the runlist, and it is cheap.

`GPFIFO_SCHEDULE(bEnable=FALSE)` and `DISABLE_USERMODE_CHANNELS` promise only that nothing *new* runs.
A host `GPFIFO_SCHEDULE(bEnable=0)` on the host TSG is an ordinary short control — no idle wait, no
preempt, no timeout in its contract. ⇒ **it is the one member of this plane that plausibly satisfies
`INLINE-SAFE(b)` today.**

### 2.3 Tier F — FORWARD. ★★ Cheaper than expected: the generic relay already exists and is reachable.

`SharedDevice::relay_channel_control` (`crates/kayfabe-rt/src/device.rs:4560-4566`) takes
**`cmd: ControlCmd` as a free parameter** and lands on `HostRmBackend::control`
(`crates/kayfabe-isolate-host/src/rm.rs:4690-4698`) → `raw_control` → the ioctl, with **no allowlist
at that layer**. Its only production caller today is `GET_MMU_FAULT_INFO`
(`policy.rs:3024` → `:3064`), and `is_case2_control` is `false` for every cmd on GA10x
(`crates/kayfabe-chips/src/ga10x.rs:280-282`), so nothing is silently downgraded.

| verb | forwardable by today's relay? | why |
|---|---|---|
| `STOP_CHANNEL`, `RESET_CHANNEL` | ✅ **shape-wise yes** — a `policy.rs` arm and an `OBJECT_CONTROLS` row, nothing else | they name a **channel**, which is what `route_bind_channel` resolves, and the twin is `Channel::host_channel` |
| `PREEMPT`, `GPFIFO_SCHEDULE(disable)` | ❌ | they name a **TSG**. There is no group-scoped relay; the host TSG is reachable only from inside `HostRmBackend::schedule` as `channel_parts(raw).tsg` (`rm.rs:4591`) |
| `DISABLE_CHANNELS` | ❌ | a **list** of `(hClient, hChannel)` across a VAS; no fan-out relay exists |
| `GPU_EVICT_CTX` | ❌ | names a subdevice + engine + channel/group; no host GR-context verb exists |

★ **Forwarding is the architecturally right shape**, and §5 is why: it makes the closed-firmware
question moot rather than answered.

### 2.4 ★★★★★ Tier X — and the real blocker is not the transport. **There is nowhere to run a long host verb.**

Every host RM verb kayfabe issues goes through `IsolateSlot::call`
(`crates/kayfabe-isolate-host/src/isolate.rs:360-370`): `write_frame` then `read_frame`,
**synchronously**. There is no submit-without-wait anywhere on the trait
(`RmBackend`, `crates/kayfabe-isolate/src/lib.rs:764-1219` — `alloc*`, `schedule`, `free`, `control`,
`map/unmap_gpu_va`, `ring_doorbell`, `ce_copy`, `fb_*`, `*_guest_ram`; **no stop, preempt, idle,
disable, reset, halt or quiesce**).

And every production caller of that path is **on the BQL**: the RPC decode-and-reply chain runs
`nvkvm.c:488` → `kayfabe_shim_regs_write` (`shim_unsafe.rs:1301`) → `Regs::write`
(`shim.rs:12289`) → … → `policy.respond` (`boot.rs:1521`) → `post` (`:1534`), holding the rank-0
`RankedMutex<PlaneState>` (`plane.rs:3347`) throughout. The shim states the consequence itself at
`shim.rs:12303-12308`: *"The vCPU is halted inside its own MMIO trap … replies written into guest RAM
above are not yet readable by the guest, because the guest is not running."*

★ *Known-positive for "nowhere off-BQL":* exactly **one** thread is spawned by the shim —
`kayfabe-completion-observer` (`shim.rs:11745-11750`) — and it is handed a **read-only closure by
type** (`shim.rs:3651-3656`: *"cannot write, cannot raise, cannot resolve — those capabilities are
not in the type it is given"*). So the search finds a real off-vCPU thread, and that thread
structurally cannot issue a verb.

**Now the budget.** `INLINE-SAFE(b)` needs the shortest guest-side timeout covering the operation.
Two of them, and they disagree by an order of magnitude:

| bound | value | source |
|---|---|---|
| the guest's **GSP RPC deadline** | `defaultus + defaultus/2` = **6 s** in `NV_GPU_MODE_GRAPHICS_MODE`, **45 s** in `COMPUTE_MODE` | `_kgspRpcRecvPoll`, `kernel_gsp.c:2372-2378`; `defaultus` = 4 s / 30 s from `osGetTimeoutParams`, `arch/nvalloc/unix/src/os.c:1961-2003` |
| the guest kernel's **soft-lockup / RCU-stall detectors** | far below that, and they see the **whole VM frozen** | `blocking_and_completion_model.md` §1 |

⊘ **The RPC deadline is not the binding constraint — the BQL is.** A forwarded
`STOP_CHANNEL(bImmediate=FALSE)` can legitimately take the *host's* default RM timeout (same 4 s/30 s
class) to return, and for that whole time **every vCPU and QEMU's main loop are stopped**. The guest
then blames itself, which is the exact failure `blocking_and_completion_model.md` §3 predicts as
*silent*.

⚠ And a second hazard, worth naming because it is invisible: **`gpuGetMode` is dynamic.** It flips to
`COMPUTE_MODE` when a `GR_OBJECT_TYPE_COMPUTE` object is allocated (`kernel_graphics_context.c:3183-3186`,
re-running `timeoutInitializeGpuDefault`). Guest and host need not agree, so *"the host's halt timeout
is smaller than the guest's RPC deadline"* can be true at one instant and false at the next.

⇒ **Tier X = a forward that is correct but does not fit inline.** `STOP_CHANNEL(FALSE)`,
`DISABLE_CHANNELS(FALSE)`, `PREEMPT(bWait=TRUE)`, `IDLE_CHANNELS`. **Blocked on an off-BQL execution
site for host verbs — not on a completion transport, and not on the reactor.** That is a materially
more precise ask than *"wire the reactor"*, and it is the one thing this plane is waiting on.

### 2.5 The split, as the shippable answer

| tier | verbs | ship this week? |
|---|---|---|
| **V — vacuous** | any verb, when the named channel/group has no host twin | ★ yes in principle (one routing helper); ⊘ **not here** — §8 |
| **D — deschedule, forwarded** | host `GPFIFO_SCHEDULE(bEnable=0)`; `DISABLE_USERMODE_CHANNELS` | ★ **the cheapest honest increment on the plane.** Needs `RmBackend::schedule` to grow an enable flag (5 impls) — and it closes w301 finding #7 |
| **F — forward, channel-scoped** | `RESET_CHANNEL`; `STOP_CHANNEL(bImmediate=TRUE)` | shape-ready via `relay_channel_control`; **bound unmeasured** ⇒ needs a boot before it may be inline |
| **X — forward, unbounded** | `STOP_CHANNEL(FALSE)`, `DISABLE_CHANNELS(FALSE)`, `PREEMPT(bWait)`, `IDLE_CHANNELS`, `GPU_EVICT_CTX` | ⊘ **no.** Refuse by name until there is an off-BQL site |

---

## 3. How does the guest learn? — **the RPC reply is the sole transport, and nothing reads it**

### 3.1 There is exactly one transport, on both sides

- **NVIDIA's side.** `pRunlistPreemptEvent` — the one documented async escape hatch
  (`ctrl2080fifo.h:330`: *"KEVENT handle for Async HW runlist preemption … When NULL, will revert to
  synchronous preemption with spinloop"*) — is **dead everywhere**. It appears in three non-generated
  places: the header, the priv check (`kernel_fifo_ctrl.c:725-730`), and two vGPU marshallers that
  **hard-null it** (`inc/kernel/vgpu/rm_plugin_shared_code.h:350`, `:4047` — *"vGPU do not support
  guest kernel handles"*). UVM passes `{0}` (`nv_gpu_ops.c:934`, `:992`). ★ *Known-positive:* the
  same grep returns all three live sites, so it is not a broken search.
  The only other GSP→CPU async channels are `_kgspRpcRCTriggered` and `_kgspRpcOsErrorLog` — the
  **Xid/RC** plane, which is GSP *deciding* a channel faulted, not an ack for a control the guest
  issued.
- **Our side.** `CommandPolicy::respond -> Option<Reply>` (`boot.rs:349-356`) is synchronous **by
  type**, written in the same frame as the decode (`boot.rs:1521` → `:1534` → `:1614`). No pending
  table, no cookie map: `GspFsm`'s 15 fields (`boot.rs:574-681`) contain no in-flight set, and there
  are **zero** `HashMap`/`BTreeMap`/`VecDeque` in production `kayfabe-gsp`.
  ★ *Known-positive:* `grep -rn 'Deferred' crates/` returns 14 live hits — all
  `CoreEventKind::{DeferredReap, CompletionRedeliver}` — so the search finds deferral machinery where
  it exists, and finds none for replies.

### 3.2 ★★★ And the guest does not need to learn, because it does not look

`STOP_CHANNEL`'s completion signal is the RPC status, and **both consuming layers discard it**
(§0.3). The consequence is uncomfortable and must be said plainly:

> ⚠ **A kayfabe that lies about the drain and a kayfabe that reports failure produce the SAME guest
> behaviour.** Only the actual engine state differs. The guest sets `bIsContextBound = NV_FALSE` and
> `is_bound = 0` either way and proceeds to unmap.

⇒ **This is why §0 of `road_to_v1_after_cup2.md` cannot be satisfied by anything the guest observes
here.** The rule — *a completion is sent only if the observed state after it is intended and safe in
the guest* — has to be enforced on **our** side or not at all, because the guest has no observation
to make.

### 3.3 ⚠ The one caller that DOES consume the status — pre-registered

`nvGpuOpsDisableVaSpaceChannels` / `nvGpuOpsEnableVaSpaceChannels`
(`ogkm-580: nv_gpu_ops.c:926-982`, `:984-1040`) issue **`DISABLE_CHANNELS` with
`bOnlyDisableScheduling` left zero** — i.e. the full *"none of the listed channels are running in
hardware"* form — and **propagate the status to their caller**.
`[measured, w306]` `0x2080110b` is in **0 / 195** ledgers. ⇒ today §3.2 holds universally; **the day
`DISABLE_CHANNELS` first appears in a ledger, §3.2 stops being true and this design needs re-reading.**
That is the falsifier for the whole "no completion needed" argument, and it is cheap to watch: it is
one id in the unserviced ledger every boot already prints.

### 3.4 Could a deferred reply be built, if it were ever needed?

Mechanically yes, and it is worth writing down so nobody re-derives it: the guest **polls guest RAM**
(`_kgspRpcRecvPoll`'s `for(;;)`, cited in-tree at `crates/kayfabe-rmrpc/src/lib.rs:75-83`), the
status-queue IRQ is deliberately **not delivered** (`qemu/hw/misc/nvkvm/nvkvm.c:592`, `:614-621`),
`RpcCommand` is `Clone` with `reply()` carrying `sequence` forward (`crates/kayfabe-gsp/src/rpc.rs:359`,
`:472-483`), and `QemuVmm::gpa_write` takes **no BQL** (`crates/kayfabe-vmm-qemu/src/lib.rs:2126`).
⊘ **The cost is not the RAM write.** It is (i) `GspFsm::post` needs `&mut GspFsm` inside the rank-0
`PlaneState` mutex, so a second thread posting a reply is `INLINE-SAFE` clause **(c)** by
construction; (ii) it discards the `answer-then-commit` invariant the FSM is built on
(`boot.rs:1296-1301`); and (iii) **`NV_ASSERT_OR_RETURN(!pKernelGsp->bPollingForRpcResponse, …)`**
(`kernel_gsp.c:2345`) forbids recursive polling, so we must not emit an event that provokes a nested
synchronous RPC while a reply is outstanding.
⇒ **Do not build it.** It buys a completion nobody reads (§3.2), at the price of the one invariant
the GSP FSM has.

---

## 4. ★★★★★ The safety half — YES, and the worst instance is ordinary use, not a race

**The question:** *is there a path where the guest believes work is cancelled (or a channel freed) and
the host GPU is still writing into pages the guest has reused?*

**Answer: yes, three of them.** Ranked by what a guest can actually cause, not by apparent severity.

### 4.1 ★★★★★ #1 — the UVM stop→unmap sequence. Every teardown. **115 / 195 boots.**

UVM's own comment states the safety property it is buying, `uvm_user_channel.c:877-878`:

> *"Tell RM to kill the channel **before we start unmapping its allocations**. This is to **prevent
> spurious MMU faults during teardown**."*

The sequence, all measured or read:

1. `uvm_user_channel_stop` → `nvGpuOpsStopChannel` → `STOP_CHANNEL(bImmediate=FALSE)` — the draining
   form (§0.2).
2. We answer **`0x56`**. `kchannelCtrlCmdStopChannel_IMPL` returns at `kernel_channel.c:1966`; the
   channel is not stopped and `kchannelNotifyRc_HAL` never runs.
3. `NV_ASSERT_OK` discards the status (`nvassert.h:467-473`); `bIsContextBound = NV_FALSE` and
   `is_bound = 0` are set anyway.
4. `uvm_user_channel_detach` unmaps the channel's allocations.

⇒ **The guest executes a sequence whose entire purpose is "no engine is touching these pages when I
unmap them", and the step that establishes the precondition is refused.** Whether an engine *is* still
running is unknown to us; what is certain is that the guest's belief is unfounded **by construction**,
on the ordinary teardown path, in 115 of 195 committed boots.

⊘ **This is not the audit's free-after-ring race.** It needs no race, no concurrency and no second
vCPU. **BUILT + REACHABLE** as a defect: reached from any CUDA process exiting.

### 4.2 ★★★★ #2 — the leaked pins. By construction, never released.

`Vas::guest_ram_pins` (`crates/kayfabe-core/src/gpu.rs:233`) — **verified again at `6d501933`** —
has `insert`, `get`, `range`, `len` and **no `remove`, `retain`, `clear` or `drain` anywhere in the
tree**. ★ *Known-positive:* the same file contains 22 `.remove(`/`.retain(` hits, e.g.
`proc.exec.requested.remove(&route.chan)` at `gpu.rs:671`. The search is live; the absence is real.

⇒ w301 §3.3's sentence stands unchanged: the host GPU keeps a live, RM-pinned translation into guest
physical pages for the isolate's remaining life, long after the guest freed them. ★ w303 armed the
reap (`shim.rs:12390`), which now issues the *staged* frees — but staging walks `vas.table` and
`vas.blocks` only and **never consults `guest_ram_pins`**, so arming the reap did not touch this.

⊘ **New, same class, found this rung:** `CompletionQueue::pending` is appended to on every observed
completion (`crates/kayfabe-fwd/src/lib.rs:6830`, `:6897`, `:6904`) and **`CompletionQueue::ack` has
zero callers anywhere, including tests**. It is bounded at `MAX_OUTSTANDING_COMPLETIONS = 1<<18` and
monotonic below that — a second accumulator a cancellation plane must drain.

### 4.3 ★★★ #3 — free-after-ring. Real, and **narrower** than w301 states.

`RmBackend::ring_doorbell` executes at `crates/kayfabe-isolate/src/lib.rs:2961`; only afterwards does
`commit_doorbell` (`crates/kayfabe-fwd/src/lib.rs:3547`) re-validate and possibly refuse, whereupon
`dispose_on` (`crates/kayfabe-rt/src/device.rs:1975`) issues `NV01_FREE` immediately and the guest is
told `DoorbellReport::Refused`.

⊘ **Precision w301 blurs:** the `orphans` closure (`fwd/lib.rs:3561-3568`) frees `fresh_chan` /
`fresh_vas` — **the host objects this very verb just materialized**. Because materialization and the
ring happen on the same verb, that *is* the channel just rung, but it is never a pre-existing one.
⇒ the exposure is scoped to **first-doorbell** channels.

| `Stale` variant | what a guest does to cause it | retried? |
|---|---|---|
| `Stale::Rebound` (`fwd/lib.rs:1011`) | rings the same channel from **two vCPUs** | ★ yes, bounded by `MAX_COMMIT_RETRIES = 8` |
| `Stale::Proc` / `Route` / `Channel` / `Vas` | a `FREE` of the proc / channel / VAS lands between plan and commit | **no** ⇒ the free-after-ring fires |

### 4.4 The ruling

**The safety half outranks the feature, and the ordering is forced.** #1 and #2 are not fixed by
serving any cancellation verb: #1 is fixed by *performing* a stop (tier F/X, §2.4), #2 by a release
path that has nothing to do with the guest's verbs. ⇒ **A cancellation plane that served
`STOP_CHANNEL` honestly and left #2 in place would have moved the smaller number.**

⊘ **Blast radius is unchanged and is the one good news:** one sandbox per `(Proc, GpuId)`, its own RM
client namespace, guest-RAM memfd granted per-isolate. The leaked mappings reach *any process inside
the causing guest*, and **no other VM's** (w301 §3.3, re-checked against
`multi_tenant_isolation_assessment.md` §1).

---

## 5. What is genuinely blocked on closed firmware — **and why the design does not depend on it**

The open question w301 left: **does the GSP fence the engine on `NV01_FREE` of a channel?**
CPU-RM definitively does not — `kchannelDestruct_IMPL` (`kernel_channel.c:1132-1270`) was re-read in
full for this document: **no stop, no preempt, no idle-wait, no `kfifoStartChannelHalt`, no timeout,
no polling loop**, just a synchronous `NV_RM_RPC_FREE` at `:1214`.
★ *Known-positive:* greps for each of those names return live hits elsewhere (`nv_gpu_ops.c:10961`,
`kernel_gsp.c:724`, `kernel_idle_channels.c:44`, `kernel_fifo_ctrl.c:120`), so the search works and
the function genuinely contains none.

### 5.1 ★★★ The ruling: **the design does not depend on the answer, and it is shaped that way on purpose**

- **Where we FORWARD a guest verb** (tiers D, F, X), the host's real GSP performs it and our answer is
  the host's answer. Whatever GSP does is **by definition** what a native process gets. ⇒ the question
  is *moot*, not answered.
- **Where we free a host channel on our own initiative** — `dispose_on` after an R5 refusal (§4.3), the
  reap's staged `Release`, `SparseFb::device_reset`'s `joined.clear()` — there is no guest verb to
  forward and the answer *would* matter.
  ⇒ **but the fix there is an invariant, not a fact:** *never free a host channel we have not first
  stopped through the host RM.* Adopting it makes the firmware answer irrelevant in that arm too.

⇒ **The experiment below is severity-informative, not design-blocking.** It tells us how urgent §4.3
is; it does not gate any decision here. Recorded because the brief asked, and because a sibling lane
can run it cheaply.

### 5.2 The one experiment that settles it — native, no guest, no QEMU

**Question.** Does the host GSP stop the engine before `NV01_FREE` of a channel returns?

**Method.** On the bench host, natively (no VM): allocate a channel; submit work that repeatedly
writes a monotonically increasing value into a host-visible page for ≥ 2 s (a semaphore-release loop
or a chain of large CE copies); confirm the engine is busy by sampling; then issue `NV01_FREE` on the
channel with **no** `STOP_CHANNEL`, `PREEMPT` or `IDLE_CHANNELS` first. Record the value at the
instant `NV01_FREE` returns (t0), then at t0+50 ms, t0+500 ms and t0+2 s.

**Three outcomes, pre-registered** (two values would not distinguish the interesting case):

| observation | reading |
|---|---|
| frozen at the t0 value | **GSP fences synchronously on free.** §4.3 is a contained defect |
| still advancing at t0+500 ms | **GSP does not fence.** §4.3 is a write into pages the guest may have taken back — and §5.1's invariant becomes mandatory, not merely tidy |
| advances briefly, stops before t0+500 ms | **GSP fences asynchronously.** The question becomes the *bound*, and the run must be repeated with the sampling tightened |

**Two controls, both required.**
1. **Positive control** — the identical sequence with an explicit `STOP_CHANNEL(bImmediate=FALSE)`
   before the free. If the counter still advances after the free returns in *that* arm, the instrument
   is measuring something other than engine writes and the run must be **discarded, not read**.
2. **Authorship control** — prove the counter is written by the GPU and not by the CPU, by the method
   `docs/reference/native_dataplane_cup2_ga106.md` establishes (a report timestamp tracking the GPU's
   own clock), **not** by a watchpoint: *a DMA write is invisible to x86 debug registers*, so a
   watchpoint is a negative control only.

---

## 6. Ranked findings — by what a guest can actually cause

| # | finding | what a guest does | verdict | § |
|---|---|---|---|---|
| 1 | **UVM's stop-before-unmap sequence is defeated on every teardown.** The guest performs a sequence whose stated purpose is preventing MMU faults while unmapping, and we refuse the step that establishes the precondition. `115 / 195` boots | exit a CUDA process — ordinary use | **NOT BUILT** (refused `0x56` by name) | 4.1 |
| 2 | **Guest-RAM pins are never released**, and the reap being armed did not change it: staging never consults `guest_ram_pins`, which still has no removal | ordinary use | **NOT BUILT** | 4.2 |
| 3 | **There is nowhere to run a long host verb off the BQL.** `IsolateSlot::call` is write-then-read; every production caller is inside the vCPU MMIO trap; the one off-vCPU thread is read-only *by type* | n/a — structural | **NOT BUILT**, and it is the plane's real blocker | 2.4 |
| 4 | **Free-after-ring**, scoped to first-doorbell channels; four of five `Stale` variants are non-retryable | free a channel/VAS/proc while a doorbell is in flight; or ring from two vCPUs | **BUILT + REACHABLE** | 4.3 |
| 5 | **`GPU_EVICT_CTX` (`0x2080012c`) is a delegated GSP verb we refuse**, arriving in exactly the same `115 / 195` boots as `STOP_CHANNEL` and absent from w301's census. It is the inverse of `GPU_PROMOTE_CTX`, which we serve | exit a CUDA process with a GR channel | **NOT BUILT** | 0.4 |
| 6 | **Descheduling still never reaches the host.** `RmBackend::schedule` has no enable flag and hardcodes `b_enable: 1` | issue `GPFIFO_SCHEDULE(bEnable=0)` | guest plane **BUILT + REACHABLE**; host plane **NOT BUILT** | 1, 2.5 |
| 7 | **`CompletionQueue::pending` is appended to and never acked** — `ack` has zero callers anywhere | any doorbell that observes a completion | **BUILT + ORPHANED** consumer | 4.2 |
| 8 | **`RESET_CHANNEL` has reached us once**, in the multi-process boot — w301 says never | run concurrent CUDA processes | **NOT BUILT** | 0.5 |
| 9 | **A forwarded drain would be honest and would still be wrong**, because it blocks the BQL for the host's RM timeout | issue `STOP_CHANNEL` — every teardown | design constraint | 2.4 |

**What is good and should not be lost:** the default control arm is a **named refusal with a ledger**,
not a silent `NV_OK`; `respond_preempt` is a correct, tested instance of the vacuous-completion
construction and is the template the rest of the plane should follow; `relay_channel_control` is
generic in `cmd` and already reachable, so tier F costs a policy arm rather than an architecture; and
`Reply` being `Option<Reply>` with no third state is what makes *"we forged a deferred completion"*
unrepresentable today.

---

## 7. What was built here, and why it was safe

**`CANCELLATION_VERBS` + the gate — the plane's discipline, as a checked table. No verb's answer
changed.**

- `crates/kayfabe-abi/src/submit.rs` — `CancellationVerb`, `CancelPromise`, `CANCELLATION_VERBS`,
  `cancellation_verb()`. Each row carries the **promise** (`NoNewWork` / `NotRunning`) with the
  verbatim `ogkm-580` sentence that states it, the HAL binding, the measured arrival census, and the
  status this port must answer when it cannot perform the verb.
- `tests/tests/cancellation_plane_is_honest.rs` — the gate.

**Why a table and not a verb.** w301's finding was not that a row was *wrong*; it was that
`INPUT_ONLY_CONTROLS` membership carried **no reason**, so a row that had stopped being true looked
exactly like one that never was — and it survived review for two days. w303 fixed the one instance.
This fixes the **class**: a cancellation verb cannot be added to the echo-ack path without the gate
going red, and cannot be added to the table without writing its promise down.

★ **The known-positive, watched failing — twice, and the second one is the argument.**
Re-inserting `0xa06c0105` into `INPUT_ONLY_CONTROLS` (its state at `91f8b34b`) makes
`no_cancellation_verb_is_answered_by_an_input_only_echo` fail by name. ⊘ But
`preempt_is_decided.rs` already catches *that id*, so it proves the gate fires and not that it is
worth having. The injection that does is a **different** verb — `0xa06f0112`:

```text
=== w303's gate (id-specific — GREEN, i.e. BLIND to this):
test result: ok. 6 passed; 0 failed
=== w306's gate (RED):
★★★★★ FORGED COMPLETION. 0xa06f0112 NVA06F_CTRL_CMD_STOP_CHANNEL is a cancellation verb
promising `NotRunning`, and it is in INPUT_ONLY_CONTROLS — …
test result: FAILED. 4 passed; 1 failed
```

⇒ **the existing suite is green while the new forgery is present.** w303 fixed a row; this
quantifies over the class, including verbs nobody has written an arm for yet.
⊘ *A gate nobody has seen fail is not a gate.*

⚠ **And one operational lesson, paid for in this rung.** The first injection was reverted with
`git checkout <file>` while the *table itself* was still uncommitted — **which deleted it**. It was
recoverable only because the edit text was still in the session. ⇒ **commit before injecting a
known-positive**, always: the revert step of a watched-failing experiment is
`git checkout`, and that verb cannot distinguish the injection from the work.

**Why it was safe to build with no bench:** it adds data and a test. No control's answer, status,
body or claimed-set membership changes, so no boot can observe it.

---

## 8. What must NOT be built without a boot — pre-registered, with falsifiers

### 8.1 ⊘ `STOP_CHANNEL`'s vacuous arm. Specified here; **deliberately not built.**

The tier-V construction (§2.1) applies to `0xa06f0112` verbatim: no host twin ⇒ nothing ever reached
hardware ⇒ the channel is stopped by construction ⇒ `NV_OK` is **true**. It is one routing helper away.

**Why it does not ship in this lane, and the reasoning is the point.** An `NV_OK` here is not inert:
`kchannelCtrlCmdStopChannel_IMPL` proceeds to `kchannelNotifyRc_HAL` (`kernel_channel.c:1979`) →
`krcErrorSetNotifier(..., ROBUST_CHANNEL_PREEMPTIVE_REMOVAL, ...)` → `krcErrorWriteNotifier_CPU`
(HAL resolves unconditionally, `g_kernel_rc_nvoc.h:213`), which writes **slot 0** of the channel's
`hErrorContext` with `info32 = 45`, `status = 0xffff` (`kernel_rc_notification.c:59`, `:71`).

⇒ **That is the exact word w288 bought.** UVM polls notifier slot 0 as its only error exit; moving
`CUP2_RC` 124 → 1 turned on that path. Making the guest write *"this channel was preemptively
removed"* into it is a **guest-observable behaviour change on a path that runs in 115 of 195 boots,
including both crossing boots**, and this lane has **no bench**. w303 could ship the identical
construction for `PREEMPT` risk-free only because `0xa06c0105` is measured absent from both crossing
boots; `0xa06f0112` is measured **present**.

**Pre-registered falsifier for the rung that does ship it** (three values, not two):

| observation after the change | reading |
|---|---|
| `CUP2_RC=0` **and** `CUP3_VAL=43`, `0xa06f0112` leaves the unserviced ledger | the arm is inert on the ladder and honest — keep |
| either metric regresses | the notifier write is consumed somewhere we did not model — **revert**, and the boot has told us something new about slot 0 |
| ladder unchanged **and** `0xa06f0112` still in the ledger | the arm never fired: every stopped channel had a host twin. ⇒ the vacuous arm is the wrong half and tier F is the only path |

⊘ The third row is the one a two-valued test would miss, and it is the likely one.

### 8.2 ⊘ Do not forward a drain inline

§2.4. A forwarded `STOP_CHANNEL(bImmediate=FALSE)` is *honest* and still wrong: it blocks the BQL for
the host's RM timeout and freezes the VM. **Refusing by name is the correct answer until there is an
off-BQL site for host verbs.**

### 8.3 ⊘ Do not downgrade `bImmediate=FALSE` to `TRUE` on the wire

A tempting bound: forward the `TRUE` form, which does not wait for idle. ⊘ It is a different verb —
`FALSE` lets in-flight work **complete**, `TRUE` **kills** it and may RC the channel
(`ctrla06fgpfifo.h:224-231`). The caller cannot tell (§3.2), which makes it *undetectable*, which
makes it worse rather than better: it is `PREEMPT`'s forgery with an extra step.

### 8.4 ⊘ Do not build the deferred-reply queue

§3.4. It buys a completion nobody reads, and costs the FSM's `answer-then-commit` invariant plus a
rank-0 lock taken from a second thread.

### 8.5 ★ What *should* be next, in order

1. **The host-plane deschedule** (tier D) — `RmBackend::schedule` grows an enable flag (5 impls) and
   `GPFIFO_SCHEDULE(bEnable=0)` reaches the host TSG. Closes w301 #7, is a short control, and is the
   only member of this plane that plausibly satisfies `INLINE-SAFE(b)` today.
2. **§8.1's vacuous arm**, on a bench, with the three-valued falsifier above.
3. **An off-BQL execution site for host verbs** (§2.4) — the prerequisite for everything in tier X,
   and the thing to state as the ask rather than *"wire the reactor"*.
4. **The release path for `guest_ram_pins`** — larger than this plane and ranked above it (§4.4).

---

## 9. What this document could not settle

1. **Whether the GSP fences on channel free.** Out of tree. §5 says the design does not depend on it
   and gives the one experiment; a sibling lane can run it.
2. **Whether a forwarded `STOP_CHANNEL` or `RESET_CHANNEL` returns fast enough to be inline.** The
   *contract* bound is the host's default RM timeout (4 s / 30 s class); the *typical* bound is
   unmeasured. One boot with the relay wired and timed would settle it, and until then tier F stays
   unshipped.
3. **Whether any guest consumer reads the RC notifier slot 0 written on a successful `STOP_CHANNEL`**
   in a way that matters (§8.1). Needs the boot.
4. **How many channels reaching `STOP_CHANNEL` have a host twin.** Decides whether tier V is most of
   the plane or none of it, and is exactly §8.1's third falsifier row. One counter in a boot log
   answers it.

## See also

- `docs/audits/w301_cancellation_error_leaks.md` — the census; §0.4, §0.5 and §4.3 correct it.
- `docs/design/blocking_and_completion_model.md` — `INLINE-SAFE`; §2.4 is its first real test and
  reports that clause **(b)** fails for the whole drain set.
- `docs/design/road_to_v1_after_cup2.md` §0 — the completion rule; §3.2 records that on this plane the
  guest makes no observation, so the rule must be enforced on our side or not at all.
- `docs/design/gpfifo_schedule.md` — the P1/P2/P3 split; tier D is P3 arriving for the disable form.

# The GSP-plane status census — every non-OK we emit below the ioctl boundary

Task: `gsp-plane-census`. Companion to `c_rust_trace_differential.md` (the C oracle) and to the
**userspace** ioctl differential recorded in the C artifact at
`/workspace/nvidia-gpu-passthrough/traces/guest_mode2_vh2/MANIFEST.txt`.

The userspace differential censused the **ioctl boundary** and found three `0x56`s, one of them
hardware's own. It said so, and named its own scope limit: *"the same guest's kernel emits
NVRM lines this instrument cannot see … the RM↔GSP RPC plane, which this instrument cannot see
at all."* This document is that plane.

---

## ★★★★★ 0. LEAD WITH THE REFUTATIONS — three, and the first one inverts the brief

### R1. "Real hardware returns non-OK essentially never" is TRUE of the ioctl boundary and FALSE of the GSP plane — by a factor of 70

The prior this census was commissioned under was transferred from the userspace differential:
real hardware answered non-OK **once in 613 ioctls** (0.16 %), so a `0x56` we emit is *prima
facie* a divergence.

⊘ **Measured, and it does not transfer.** `traces/rpctrace_ga106_boot1.bin` — real GA106,
real NVIDIA GSP firmware, driver 580.159.04, the same `RmInitAdapter` workload:

| boundary | non-OK replies | rate |
|---|---|---|
| userspace ioctl (host reference, 613 records) | 1 | **0.16 %** |
| **GSP RPC control plane** (310 control replies) | **35** | **11.3 %** |

**12 distinct control commands** are answered `NV_ERR_NOT_SUPPORTED` (`0x56`) by NVIDIA's own
firmware, and one more is answered `0x57`:

| cmd | status | n | cmd | status | n |
|---|---|---:|---|---|---:|
| `0x2080012f` | `0x56` | 1 | `0x20801357` | `0x56` | 2 |
| `0x2080014b` | `0x57` | 8 | `0x20808546` | `0x56` | 12 |
| `0x20800157` | `0x56` | 2 | `0x20809038` | `0x56` | 1 |
| `0x20800a87` | `0x56` | 2 | `0x2080a0f2` | `0x56` | 1 |
| `0x20800b05` | `0x56` | 2 | `0x2080a63c` | `0x56` | 1 |
| `0x20801322` | `0x56` | 1 | `0x90e70113` | `0x56` | 1 |
| `0x20801344` | `0x56` | 1 | — | — | |

⇒ On this plane `0x56` is **not an anomaly, it is normal firmware behaviour**. A census that
ranks by "we emitted a `0x56`" ranks nothing here. Had this document been written to the brief's
prior it would have opened a worklist of ~41 items, five of which are *already hardware-exact*.

★ **And the id is not even a function that determines the answer.** Two commands are answered
**both OK and non-OK by the same firmware in the same capture** — `0x2080014b` (`0x0` and
`0x57`) and `0x20808546` (`0x0` and `0x56`, 12 of 18). Whatever decides those answers is state,
not identity. ⇒ A per-id answering table cannot be a *specification* of this plane, only an
approximation of it, and the two mixed rows are where that shows.

### R2. The brief's own worked example of the two-boundary trap names the wrong function

The brief warns — correctly, and it is the most valuable warning in it — that one event wears
two names on the two sides of the RPC boundary, and offers as the example:

> `0x2080200a PERF_BOOST` appears zero times in our boot logs; `kperfBoostSet_IMPL` repackages
> it as `0x20800a9a INTERNAL_PERF_BOOST_SET_2X` … ⇒ the guest's
> `kperfGpuBoostSyncStateInit_IMPL … status=0x56` is what you are looking for.

**The repackaging is confirmed** (`ogkm-580.159.04:.../perf/kern_perf_boost.c:82-92` builds
`NV2080_CTRL_INTERNAL_PERF_BOOST_SET_PARAMS_2X` and issues `0x20800a9a`; `:94 return status;`).

⊘ **But `kperfGpuBoostSyncStateInit_IMPL` is a different function issuing a different control.**
`.../perf/kern_perf_gpuboostsync.c:53-58` issues
`NV2080_CTRL_CMD_INTERNAL_PERF_GPU_BOOST_SYNC_GET_INFO` = **`0x20800a80`**
(`ctrl2080internal.h:1779`). `0x20800a9a` has **no** dmesg line at all — `kperfBoostSet` prints
nothing on failure. Two perf functions, two ids, and the loud one is not the one the brief was
pointing at.

★ The trap is real and it *caught the brief itself*: chasing a symbolic name across the boundary
is not the same as chasing the id, and the near-identical prefixes (`0x20800a80` /
`0x20800a9a`) are exactly the shape that survives review.

### R3. The loudest single line in the census cannot stop anything, by construction

`kperfGpuBoostSyncStateInit_IMPL: Failed to read Sync Gpu Boost init state, status=0x56` — 10
occurrences, the joint-most-frequent named control failure. Its function ends:

```c
kperfGpuBoostSyncStateInit_IMPL_exit:
    return NV_OK;                    /* kern_perf_gpuboostsync.c:78-79 */
```

It **unconditionally returns `NV_OK`** on every path. No amount of it can be a wall. ⇒ Frequency
is not evidence on this plane; this is the "`NV_ERR_NOT_SUPPORTED` is the FORGIVEN status"
finding, now with a call site that forgives it in the strongest possible way — by discarding it.

---

## 1. Provenance — what was measured, where, and at which revision

| | |
|---|---|
| guest kernel evidence | `/workspace/nvidia-gpu-passthrough/traces/guest_mode2_vh2/dev_dmesg.log.gz` (**347 NVRM lines**, the superset — it runs to t=980 s where `ctx_dmesg.log.gz` stops at t=617 s and is a prefix of it) |
| capture | vast.ai `vh2` (47373001), 2026-08-10 14:21–14:35 UTC, boot tag `nvd1` |
| ★ source revision | **`954f926`** (per that capture's MANIFEST). ⊘ Not `master`. |
| guest driver | NVIDIA **Open** Kernel Module 580.159.04, stock, unpatched |
| oracle | `traces/rpctrace_ga106_boot1.bin` — real GA106, real firmware, 1076 records, 2 sessions, `wrapped=false`, `dropped=0`, decoded by `scripts/rpctrace/decode_rpctrace.py` (which refuses an incomplete trace rather than warning about it) |
| ogkm | ★ `research_clones/ogkm-580.159.04/`. ⊘ The sibling `research_clones/ogkm/` is **610.43.02** — a different tree whose line numbers do not correspond to this guest's dmesg. |

### ⊘ Four instrument gaps, stated before any result

**(a) The device-side control census does not exist for the `nvd1` boot.** The emulator prints
`commands: N decoded, M UNSERVICED …, K distinct` **at shutdown**, and the `nvd1` QEMU
still had the hung workload attached when its log was taken — `run_nvd1_qemu.log.gz` is 56 lines
and carries no census. The refusal **set** in §3 is therefore taken from
`traces/guest_boots/run_w22{1,2}_*_qemu.log` (revisions `49dc3ec` / `346921b`), which are
**different boots at different revisions**. ⇒ Set membership in §3 is a property of those
revisions; only the guest-side counts in §2 belong to `954f926`. ★ This is the trap CLAUDE.md
records as costing weeks — a claim inherits the revision it was *measured* at, not the one it
is *about*.

**(b) The census prints ids, never counts.** Each `unserviced fn 76 cmd 0x…` line appears once
per distinct id. Nothing in §3 is a frequency.

**(c) Every refusal on this plane looks the same on the wire.** `BridgeRefusal::rpc_result` is
`NV_ERR_NOT_SUPPORTED` for **every** variant, deliberately
(`crates/kayfabe-rmrpc/src/policy.rs:2204-2212`): *"That makes a refused promote-ctx
wire-indistinguishable from an unserviced one."* ⇒ From the guest's dmesg alone one **cannot**
tell "no arm claimed this" from "an arm decided to refuse".

★★★★ **(d) And the device-side census UNDERCOUNTS our own refusals — measured.** The
`unserviced fn 76 cmd …` ledger is fed by the *fall-through* at the end of the responder chain,
so a control that has an arm which **decides** to refuse never reaches it. `0x2080012b`
`GPU_PROMOTE_CTX` is exactly that: `grep -c 'unserviced fn 76 cmd 0x2080012b'` over both boot
logs returns **0**, while the guest logs its `0x56` five times and `policy.rs:2214-2231`
documents choosing that status on purpose.

⇒ **The 41-id set in §3 is a LOWER BOUND on what we refuse, not the refusal set.** The two
boundaries are not two views of one population: the guest's dmesg sees refusals the ledger
cannot, and the ledger sees ids the guest never logs. This census only found `0x2080012b`
because the *guest* named it — an id-worklist built from the device ledger alone would have
been silently short, with no counter anywhere reading zero to say so.

---

## 2. The census — clustered into bursts, because the clusters are the finding

★★★★ The 347 NVRM lines are **not one population**. Clustering at a 20 s gap
(`GAP=20.0`) splits them into six bursts, and the split is load-bearing: the `ctx` workload
hangs and is killed **180 s later**, so its *teardown* noise is separated from its *init* noise
by a ~175 s hole. Counted together, the single largest block in the census is a **consequence of
the wall** presenting as a candidate for it.

| burst | t (s) | lines | what it is |
|---|---|---|---|
| 0 | 11 | 1 | module load |
| 1 | 159–163 | 30 | a cuInit-bearing attempt — init only |
| 2 | 254–258 | 34 | a cuInit-bearing attempt, **+16 × `UVM_CHANNEL_RETAINER`** |
| 3 | 434–440 | 131 | **teardown after a 180 s kill**, overlapping the next init (**+16 × `UVM_CHANNEL_RETAINER`**) |
| 4 | 615–617 | 97 | **teardown after a 180 s kill** |
| 5 | 971–980 | 54 | ★ two complete cycles, **no teardown burst, no kill gap** — the `dev` stage, `prog rc=0` |

⚠ **The burst↔run mapping is NOT 1:1, and the census does not pretend it is.** The
`kgrobjPromoteContext` marker — one per `cuInit` — fires **5** times, while the MANIFEST
records **4** captured program runs (`dev_r{1,2}`, `ctx_r{1,2}`). One cuInit-bearing process in
this dmesg is **unattributed**. ⊘ Nothing below rests on the mapping: the ranking rests only on
burst 5 being the `dev` stage, which three independent signals agree on — it is the last thing
in a log captured at t≈980 s, it contains **no** 180 s kill gap and **no** `STOP_CHANNEL`
teardown, and its two cycles complete in 9 s.

### ★★★★★ Burst 5 is the discriminator, and it is empirical

Burst 5 is the `dev` stage: `cuInit` + every device query, **exit code 0**, the run the
workload's own stdout calls `DONE`. Every kind below occurs in it, most exactly twice — once
per run (`GspRmFree` 6× and `vaListDestroy` 8×, being per-object rather than per-run):

`0x20800afe` · `0x20800aff` · `0x20800a80` · `0x20802a0f` · `kceGetPceConfigForLceType()` ·
`GspRmAlloc 0x0070` · `GspRmAlloc 0xc36f` · `GspRmAlloc 0x402c` · `kgrobjPromoteContext()` ·
`kernel_rc_watchdog.c:1198` · `mem.c:180` · `rs_client.c:844` · `rs_server.c:259` ·
`rs_server.c:1375` · `fecs_event_list.c:1623` · `GspRmFree` · the three `…proceeding...` lines

⇒ **Each of these is present, at full multiplicity, in a run that SUCCEEDS.** That is a
`PROCEEDS` verdict established by measurement, not by reading a control-flow graph — and it is
the only kind of verdict that survives being wrong about the source.

⊘ **What is absent from burst 5:** `GspRmAlloc 0xc574`, `GspRmAlloc 0x208f`,
`NVA06F_CTRL_CMD_STOP_CHANNEL`, `NV2080_CTRL_CMD_GPU_EVICT_CTX`, `nv_gpu_ops.c:10328`.

### 2.1 Full census, with the source verdict beside the measured one

Counts are over all four program runs in `dev_dmesg.log.gz`. `ogkm-580.159.04` paths are
relative to `src/nvidia/src/`.

| what we refuse | id / class | n | in the rc=0 run? | source verdict | deciding line |
|---|---|---:|---|---|---|
| `INTERNAL_INIT_USER_SHARED_DATA` | `0x20800afe` | 10 | yes | **PROCEEDS** | caller discards it: `gpu/gpu_user_shared_data.c:310` bare call, `:315 return NV_OK;` |
| `INTERNAL_USER_SHARED_DATA_SET_DATA_POLL` | `0x20800aff` | 5 | yes | **PROCEEDS** (init path) / **STOPS** (RUSD alloc path) | `:313` discards; but `:373 NV_ASSERT_OR_RETURN(...)` → `:95` → `NV00DE` alloc fails |
| `INTERNAL_PERF_GPU_BOOST_SYNC_GET_INFO` | `0x20800a80` | 10 | yes | **PROCEEDS** — swallowed | `perf/kern_perf_gpuboostsync.c:78-79 return NV_OK;` |
| `INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE` | `0x20802a0f` | 10 | yes | **STOPS locally, laundered** | `ce/kernel_ce.c:1020 NV_ASSERT_OK_OR_RETURN` → `gpu/gpu.c:2574-2575` converts `0x56`→`NV_OK` at engine granularity |
| `INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE` | `0x20800a38` | 5 | yes | **PROCEEDS** | `gr/fecs_event_list.c:1623 NV_ASSERT_OR_RETURN_VOID` in a `void` fn |
| `kgrobjPromoteContext` (`GPU_PROMOTE_CTX`) | `0x2080012b` | 5 | yes | **STOPS** the GR object | `gr/kernel_graphics_object.c:224 NV_CHECK_OK_OR_RETURN` — "Check" changes logging, not control flow |
| `GspRmAlloc NV01_MEMORY_VIRTUAL` | `0x0070` | 5 | yes | **STOPS** the watchdog heap | `rc/kernel_rc_watchdog.c:669-678` |
| `GspRmAlloc VOLTA_CHANNEL_GPFIFO_A` | `0xc36f` | 5 | yes | **STOPS** the watchdog channel | `rc/kernel_rc_watchdog.c:1013-1019` |
| `GspRmAlloc NV40_I2C` | `0x402c` | 5 | yes | **PROCEEDS** | I²C/DDC gateway; nothing on the compute path allocates it |
| `GspRmAlloc NV20_SUBDEVICE_DIAG` | `0x208f` | 1 | ⊘ **no** — burst 1 only | **PROCEEDS** (source only) | diagnostics gateway; no in-kernel allocator, allocated by nvidia-smi |
| watchdog init assert | `rc/kernel_rc_watchdog.c:1198` | 5 | yes | **PROCEEDS globally** | caller is `void` and logs at `LEVEL_INFO`: `rc/kernel_rc_watchdog_callback.c:201-207`; retried by the 1 Hz callback |
| `GspRmFree` | — | 48 | yes | **PROCEEDS** | `mem_mgr/mem.c:180`, `resserv/rs_client.c:844`, `rs_server.c:1375` are bare `NV_ASSERT`; object is freed regardless |
| **`GspRmAlloc UVM_CHANNEL_RETAINER`** | **`0xc574`** | **32** | ⊘ **NO** | ★ **STOPS** | `rmapi/nv_gpu_ops.c:10231-10232 if (status != NV_OK) goto error;` |
| `NVA06F_CTRL_CMD_STOP_CHANNEL` | `0xa06f0112` | 32 | ⊘ no — **teardown only** | n/a | bursts 3–4 only |
| `NV2080_CTRL_CMD_GPU_EVICT_CTX` | `0x2080012c` | 16 | ⊘ no — **teardown only** | n/a | bursts 3–4 only |
| `nv_gpu_ops.c:10328` assert | — | 32 | ⊘ no — **teardown only** | n/a | bursts 3–4 only |

⊘ **`0x208f` is the one PROCEEDS in the table not backed by burst 5** — it occurs once, in
burst 1, and its verdict rests on source alone (it has no in-kernel allocator). ★ It is also
the best available identification of the unattributed fifth process: `NV20_SUBDEVICE_DIAG` is
what **nvidia-smi** allocates. Suggestive, not established.

★ **The `0x0070` / `0xc36f` / `kgrobjPromoteContext` / `kernel_rc_watchdog.c:1198` /
`mem.c:180` / `rs_*` cluster is ONE event, not six.** It is the RC watchdog's init, in order:
allocate its virtual heap (`0x0070`), allocate its channel (`0xc36f`), promote a GR context,
fail, assert, free everything. Ranking those six separately would put five phantom items on a
worklist. The watchdog then retries at 1 Hz forever and nothing waits on it.

---

## 3. The oracle diff — 41 refused control ids, judged

Our refused-control set (union of `run_w221` and `run_w222`, per §1(a)) is **41 ids**. Against
`rpctrace_ga106_boot1.bin`:

| verdict | n | ids |
|---|---:|---|
| ★ **AGREE** — real firmware also answers `0x56` | **5** | `0x2080012f` `0x20800157` `0x20800a87` `0x20800b05` `0x20801357` |
| ✗ **DIVERGENCE** — real firmware answers `NV_OK` | **24** | `0x00800294` `0x2080013f` `0x2080014b`* `0x2080017e` `0x20800a2c` `0x20800a2e` `0x20800a30` `0x20800a34` `0x20800a38` `0x20800a3f` `0x20800a4b` `0x20800a70` `0x20800a80` `0x20800afe` `0x20800aff` `0x20800b03` `0x20802a0f` `0x2080852e` `0x20809009` `0x2080a612` `0x2080a618` `0x20810108` `0x208f1105` `0x402c0101` |
| **[NOT ESTABLISHED]** — absent from the trace | **12** | `0x00801814` `0x2080012c` `0x20800a1e` `0x20800a9a` `0x20800a9c` `0x20800a9e` `0x20800ab8` `0x20802068` `0x20802a12` `0x20808513` `0x20810110` `0xa06f0112` |

\* `0x2080014b` is the mixed row: `0x0` and `0x57`, never `0x56`.

★ Note `0x2080012f` (`GPU_QUERY_ECC_STATUS`) heads the AGREE column. It is **the same id** the
userspace census called "the forgiven one" — the one case where the boundary does *not* rename
the event, and it is hardware-exact on both sides.

### 3.1 The alloc plane — the oracle covers it, and `decode_rpctrace.py` did not

`--controls` decodes `GSP_RM_CONTROL` only. The 208 `GSP_RM_ALLOC` and 208 `FREE` elements were
decoded separately against `rpc_gsp_rm_alloc_v03_00` / `NVOS00_PARAMETERS_v03_00`
(`ogkm-580.159.04:.../generated/g_rpc-structures.h:1407-1418`, `g_sdk-structures.h:261-267`):

> **104 alloc replies and 104 free replies, inner status `0` on every single one.**

| class we refuse | oracle | verdict |
|---|---|---|
| `0x0070` `NV01_MEMORY_VIRTUAL` | n=2, status `0x0` | ✗ DIVERGENCE |
| `0xc36f` `VOLTA_CHANNEL_GPFIFO_A` | n=2, status `0x0` | ✗ DIVERGENCE |
| `0x402c` `NV40_I2C` | n=2, status `0x0` | ✗ DIVERGENCE |
| `0x208f` `NV20_SUBDEVICE_DIAG` | n=1, status `0x0` | ✗ DIVERGENCE |
| **`0xc574` `UVM_CHANNEL_RETAINER`** | **absent** | **[NOT ESTABLISHED]** |
| `GspRmFree` (48 of ours) | 104 replies, all `0x0` | ✗ DIVERGENCE |

⊘ `0xc574`'s absence is **explained, not mysterious**, and the explanation is why it may not be
repaired by capturing more: the oracle is an `RmInitAdapter` / `nvidia-smi` capture, and
`UVM_CHANNEL_RETAINER` is allocated only by UVM registering a channel
(`nv_gpu_ops.c:10224-10230`). No CUDA context, no retainer. **Judging our single highest-ranked
item needs a capture of a workload the oracle was never run on.**

### ⚠ 3.2 The `paramsSize`-with-no-body shape, stated as a limit

Alloc **replies** carry the 32-byte alloc header and **zero** params bytes, while still
declaring the request's `paramsSize` (120, 368, …). That is the same shape as the C oracle's
`dlen=0` rows — the shape CLAUDE.md records as *positively wrong* rather than merely blind.

⊘ Three readings are consistent with it: RM copies nothing back for allocs; RM copies back
out-of-band; or the recorder's element boundary excludes it. **This capture does not
distinguish them, and this document does not claim it does.** What it *does* establish is the
**status word**, which sits inside the captured header — and the status is `0` on all 104.
Everything in §3.1 rests on the status only. (Symmetrically: for *controls*
`decode_rpctrace.py` reports `replies declaring params with NO bytes present: 0`, so the
control-plane bodies genuinely are present.)

---

## 4. THE RANKING — by whether the guest stops, and it is a ONE-ITEM worklist

### ⓵ `GSP_RM_ALLOC hClass=0xc574 UVM_CHANNEL_RETAINER` — the only candidate

Four independent facts, three of them measurements:

1. **Exclusive to the failing stage.** 16 per `ctx` run; **0** in either `rc=0` `dev` run.
   It is the only refusal in the whole census with that distribution.
2. **It is the LAST kernel-side event before the silence.** The 16th failure is at
   `t=258.361672`; the next NVRM line of any kind is at `t=433`. That 175 s hole is the
   workload spinning to its 180 s kill.
3. **The userspace instrument agrees on the same instant, by a different route.** The MANIFEST's
   wall is `B[315]`: 175 × `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS`, a count that never appears
   on real hardware, from the one thread inside `cuCtxCreate`. **175 calls, 175 seconds.**
   Two instruments sharing no code, one event.
4. **It STOPS, by source.** `nv_gpu_ops.c:10231-10232 if (status != NV_OK) goto error;` aborts
   `nvGpuOpsRetainChannel` (`:10122`).

★ **And what the abort skips is the specific thing:** the statement immediately after the
retainer alloc is `pRmApi->Control(..., NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN, ...)`
(`nv_gpu_ops.c:10233-10239`). **The work-submit token is what a doorbell write carries.** So a
refused retainer means UVM never obtains a submit token for that channel — and the device's own
log for this boot ends in eight `DOORBELL-REFUSED … [Route::NotACopyEngineChannel]` for tokens
`0x07`–`0x0e`.

⊘ **Stated as adjacency, not as a proven chain.** `goto error` unwinds the channel entirely, so
those doorbells cannot be *from* a channel whose retainer failed. Whether they are the other
channels, or a UVM retry, is **not established by this capture** and wants the deciding rung's
bench, not more reading.

⊘ **And one tempting inference is refuted.** `nv_gpu_ops.c:10221` prints *"Channel duping is not
supported. Fall back to UVM_CHANNEL_RETAINER"*, which reads as a fallback we pushed the guest
into by refusing something earlier. It is not: in 580.159.04 that `NV_PRINTF` is
**unconditional** and the retainer alloc follows it on the straight-line path. There is no
duping branch to restore. The retainer is the only path.

### ⓶ Two cheap divergences worth fixing regardless — but neither is a wall

- **`0x20802a0f` `INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE`** — oracle: 2 replies, **28 bytes**,
  `NV_OK`. STOPS `kceStateLoad_GP100`, leaving `decompPceMask` and the top-level PCE→LCE
  mapping unwritten, then `gpu.c:2574-2575` launders it. Survivable (it is in burst 5) but it
  half-configures the copy engine — and `local_ce_is_the_only_executor` is the plane.
- **`0x2080012b` `GPU_PROMOTE_CTX`** — oracle: 4 replies, **560 bytes**, `NV_OK`. ★ Note this is
  a **decided** refusal, not an unclaimed id: `policy.rs:2214-2231` documents choosing `0x56`
  over `0x33`/`0x40` deliberately, because `gpuStatePostLoad` (`gpu.c:3437-3439`) launders only
  `0x56`. Do not "fix" this by changing the status.

### ⓷ PROCEEDS — 12 kinds, closed, no work

Everything in the §2.1 table marked "yes" and not listed above. All appear at full multiplicity
in a run that exits 0. Chief among them the RC-watchdog cluster (six lines, one event) and
`0x20800a80`, whose caller returns `NV_OK` unconditionally.

### ⓸ TEARDOWN — 80 lines that are consequences, not causes

`0xa06f0112` (32) · `0x2080012c` (16) · `nv_gpu_ops.c:10328` (32) and the bulk of the 48
`GspRmFree`. They occur **only** in bursts 3–4, i.e. only after the 180 s kill. ⊘ Ranking them
by count would have put the single largest block in the census at the top of the worklist.

---

## 5. What the oracle could and could not judge

**COULD:** 29 of our 41 refused control ids, and 4 of our 5 refused alloc classes, appear in it
and are judged against real firmware — including the five where we are already exact. It also
supplied R1, which is the finding that reshapes the plane.

**COULD NOT, and each has a reason:**

- ⊘ **`0xc574`, the ranked item.** Absent because the oracle is an `RmInitAdapter`/`nvidia-smi`
  capture and UVM never runs in it. Not a gap that re-reading fixes.
- ⊘ **The 12 [NOT ESTABLISHED] control ids**, including `0x20800a9a` — which is exactly the
  exemption the predecessor predicted would be needed.
- ⊘ **Whether an alloc reply carries a body** (§3.2). Only the status is established.
- ⊘ **Which of our refusals are *decided*.** All variants ride the same `0x56`
  (`policy.rs:2204-2212`), on purpose. The guest's dmesg cannot see the difference — and per
  §1(d) our own ledger cannot either, because a decided refusal never reaches it. Neither
  instrument alone enumerates the refusal set; the two must be unioned, and this document is
  the first thing that does it.
- ⊘ **Whether the ranked item is *the* blocker or *a* blocker.** This census establishes that it
  is the only refusal exclusive to the failing stage and the last event before the silence. It
  does not establish sufficiency — that is a bench question, and this is a census.

## 6. Reproducing this

```
python3 scripts/rpctrace/decode_rpctrace.py traces/rpctrace_ga106_boot1.bin --controls
python3 scripts/rpctrace/oracle_allocs.py   traces/rpctrace_ga106_boot1.bin
python3 scripts/rpctrace/dmesg_status_census.py \
    /workspace/nvidia-gpu-passthrough/traces/guest_mode2_vh2/dev_dmesg.log.gz
```

⊘ **No table row in `capability.rs` was touched and no refusal was changed.** This is a census
and a ranking; the deciding rung is on the other bench.

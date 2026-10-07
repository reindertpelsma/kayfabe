# w294 — `cup2` CROSSED. `^CUP2_RC=` 1 → **0**, and `nvd_prog`'s `801` is retired

**STATUS: LIVE, 2026-08-14.** Three boots at `b07e64f`, real GA106 `580.159.04`, `vh`, stamp
gate PASS. Relaxations carried and labelled — **a relaxed green is a MAP, not the milestone**;
the full list is §5.

> ⊘⊘⊘ **LEAD WITH THIS: THE BRIEF NAMED THE WRONG ID, AND SERVING IT WOULD HAVE BEEN A NO-OP
> THAT LOOKED LIKE A FIX.** The rung was briefed as *"serve `0x00801909`
> `PERF_CUDA_LIMIT_SET_CONTROL` — native serves it `NV_OK`."* `0x00801909` is `flags=0x118`
> (`ogkm-580: g_device_nvoc.c:920`) — **no `ROUTE_TO_PHYSICAL`** — so the guest's own kernel
> answers it out of a per-`Device` refcount and it **never reaches our GSP**. What reaches us
> is its *internal* consequence: **`0x00802009`** and its teardown half **`0x00802004`**.
> Serving those is what crossed.

---

## 1. THE RESULT — four facts, never a word

`run_w294cup2_probe.log`, `^CUP2_RC=` **anchored**:

```
ok   cuCtxCreate(&ctx,0,d)
CTX OK                                          ← 1. context created
ok   cuMemAlloc(&dp,4096)
MEMALLOC OK 0x7f263c200000                      ← 2. allocation
ok   cuMemcpyHtoD(dp,&hv,4)
ok   cuMemcpyDtoH(&rv,dp,4)
CE rv=0xabcd1234 want=0xabcd1234 -> PASS        ← 3. the value ROUND-TRIPPED, correct
DONE
CUP2_RC=0                                       ← 4. ^ANCHORED = 0   (baseline 1)
```

| | boot 1 `w294cup2` | **boot 3, confirmation** |
|---|---|---|
| `^CUP2_RC=` **anchored** | ★ **0** | ★ **0** |
| `CE rv=… want=…` | `0xabcd1234` / `0xabcd1234` → **PASS** | `0xabcd1234` / `0xabcd1234` → **PASS** |
| `MEMALLOC OK` | `0x7f263c200000` | `0x708a80200000` |

⇒ **2 / 2 `cup2` boots crossed, plus the `nvd` arm's `CTX OK`. Three boots, two instruments.**

> ⊘⊘ **AND THE CONFIRMATION NEARLY READ AS THE FIRST BOOT.** `w294_run.sh` set
> `export KAYFABE_TAG=w294${ARM}` **unconditionally**, so the caller's `KAYFABE_TAG=w294cup2b`
> was discarded and boot 3 **overwrote boot 1's `run_w294cup2_*.log`**. Nothing said so — the
> grading block printed a perfect result, under boot 1's filename, from boot 3's data. The
> only thing that separated them was that `MEMALLOC OK` carries an **ASLR-moved address**, a
> discriminator that exists by luck. Boot 1's artefacts survive only because they were pulled
> before the overwrite. ⚠ Fixed to `${KAYFABE_TAG:-…}`, and recorded because it is the same
> trap the parent script's own comment warns about, re-committed one layer up.

⚠ **The anchor trap, resolved rather than avoided.** The unanchored read is `0 0`, not the
`[0 1]` of the last seven rungs. Both zeros are real: `grep -oh 'CUP2_RC=[0-9]*'` also matches
*inside* the harness's `GCC_CUP2_RC=0` compile line, which `^CUP2_RC=` cannot match — which is
exactly why the anchored read returned `1` on the seven rungs where the run failed. The
anchored read is the result.

### The second, independent instrument agrees — different workload, different recorder

`run_w294nvd_probe.log` (`nvd_prog ce` under the `LD_PRELOAD` shim):

```
ok cuCtxCreate(&ctx, 0, d)   CTX OK      ← was:  FAIL cuCtxCreate -> operation not supported (801)
ok cuMemAlloc / cuMemcpyHtoD / cuMemcpyDtoH / cuMemFree / cuCtxDestroy
guest jsonl lines = 578                  ← was 572
```

⇒ **`801` is retired, and it was `0x00802009`.**

---

## 2. THE ID THAT ARRIVES — measured at BOTH boundaries, because neither sees both halves

| id | `flags` | `ROUTE_TO_PHYSICAL` | who answers | which instrument can see it |
|---|---|---|---|---|
| `0x00801909` | `0x118` (`g_device_nvoc.c:920`) | ⊘ **no** | the guest's **own kernel** | **only** the ioctl recorder |
| `0x00802009` | `0x1d8` (`:1025`) | ★ **yes** | **us** | **only** our QEMU `unserviced` ledger |
| `0x00802004` | `0x0c0` (`:1010`) | ★ **yes** | **us** | **only** our QEMU ledger |

`deviceCtrlCmdKPerfCudaLimitSetControl_IMPL` (`ogkm-580: kern_cuda_limit.c:94-137`) bumps
`pDevice->nCudaLimitRefCnt` and, **only on the 0↔1 edge**, issues the internal `0x00802009`,
returning its status verbatim. ⇒ the `0x56` the guest reported on `0x00801909` was **our**
`0x56` on `0x00802009`.

`[measured]` each id appears in **exactly one** instrument and in **zero** of the other:

```
run_w290pdrain_qemu.log     : unserviced fn 76 cmd 0x00802009 , 0x00802004     (0x00801909 ×0)
traces/nvdiff_w292/serve_r1 : i=412 cmd=0x00801909 status=0x56                 (0x0080200x ×0)
```

★★★ **A reader holding one instrument reaches the wrong id WITH A CORRECT CITATION.** That is
the seam this rung closed, and it is not an id.

### It is served the right number of times, on both edges

| boot | `0x00802009` | `0x00802004` |
|---|---|---|
| `w294cup2` | `result 0x00000000 **x1**` | `result 0x00000000 **x1**` |
| `w294nvd` | `result 0x00000000 **x2**` | ⊘ **never arrived** |

⊘ The `nvd` absence is **correct, not a gap**: `deviceKPerfCudaLimitCliDisable`
(`kern_cuda_limit.c:62-75`) fires `0x00802004` only when `nCudaLimitRefCnt > 0` at teardown,
and `nvd_prog`'s **two** `0x00802009` calls (`01` then `00`) leave it at zero. That matches a
native GA106 exactly: `host_reference_ga106/ce_r1` calls `0x00801909` twice, `01` @431 and
`00` @465. The harness prints `UNMEASURED, not served` for the absent row rather than `0`.

---

## 3. `0x20801210` — ⊘ THE REFUSAL STAYS. IT IS A MEASUREMENT, NOT AN OWNER QUESTION

The brief carried this as a live design question. It was already closed by committed evidence:

1. **The controlled pair says the answer is causally inert.** `run_s45_748a207_tsgsched_probe.log:449`
   (`0x56`) vs `run_s47_81582e3_ctxsw_probe.log:449` (`NV_OK`), identical request bytes:
   **456 records each, exactly ONE record differs — that status field.** Records 332…456
   byte-identical after pointer canonicalisation. `CUP2_RC=1` in both.
2. **Refusing CILP makes libcuda downgrade and retry, and we serve the retry.**
   `serve_r1` i=391 `cilpPreemptMode=2` → `0x56`; **i=392 `cilpPreemptMode=0` → `NV_OK`**,
   `psize=pgot=32` on both. ⇒ Serving CILP would **delete record 392** and leave the guest
   believing instruction-level preemption is armed.
3. **`ogkm` cannot supply an answer**: `subdeviceCtrlCmdKGrSetCtxswPreemptionMode_IMPL` has
   **no body in the open tree** (`flags=0x10348`, `ROUTE_TO_PHYSICAL`, `pFunc` compiles to
   `NULL`), and no `bCilpSupported`-style property exists anywhere.

★ It is still `0x56` on the crossing boot, and the boot crossed anyway.

---

## 4. ⊘ AND A TEARDOWN CANNOT DISCRIMINATE — "record 332 begins the `FREE` burst" is retired

`[measured, host_reference_ga106/ctx_r1.jsonl.zst]` a **native GA106 whose own stdout prints
`CTX OK`** also begins a large `FREE` burst two records after `0x20801210` (i=433). The burst
is `cuCtxDestroy`/exit unwind, and **both a successful and a failed run produce one.** That
signature has been load-bearing since §16.56 and cannot separate anything.

---

## 5. ⊘ EVERY RELAXATION THAT WAS ON — a relaxed green is a MAP, not the milestone

```
VAS-PUBLISH arm=drain  fb_join=shared  host_isolates=true
OPERAND-JOIN arm=join
PT-SWEEP tasks=2 skipped=2 ran=2
KAYFABE_PT_SWEEP=on   KAYFABE_OPERAND_JOIN=join   KAYFABE_FB_JOIN=shared
KAYFABE_VAS_PUBLISH=drain (the doorbelled-VAS drain)
KAYFABE_GR_ROUTE=passthrough  KAYFABE_GUEST_RING=ring  KAYFABE_GUEST_PUSHBUF=pin
KAYFABE_GUEST_SEMA=pin  KAYFABE_GUEST_OPERAND=pin
KAYFABE_ISOLATES=real  KAYFABE_CE_EXECUTOR=host
```

⚠ **`cup2` is a CE round-trip, not compute.** FIRST compute is `cup3`. This is the milestone
the campaign has stood at for eight rungs, not the definition of done.

### Regression check — the address plane is UNCHANGED, single variable

| | last rung (`w290pdrain`) | **this rung** |
|---|---|---|
| `Xid` | 0 | ★ **0** (`HOST_DMESG_XID=0`, watermark 1106→1106, delta 0 lines) |
| `already_host` / `total` on the doorbelled VAS | 18 295 / 18 309 | ★ **18 295 / 18 309** |
| `not_granular` | 6 | 6 (out of scope, unchanged) |
| `published` / `refused` | 0 / 8 | 0 / 8 |
| distinct unserviced ids | 42 | **40** (the two that left) |

⚠ The w294 harness's own `host_rows=` grep is **wrong** and prints `0 of 6254`: the emission
spells it `already_host=18295 … total=18309`. The number above is read from the VAS-PUBLISH
line itself. ⊘ Recorded rather than silently corrected — a grading line that reads a real
`18295` as `0` is the class this tree calls *a falsifier that flags its own good news*.

---

## 6. THE NEXT WALL, BY IDENTITY — and it is NOT a control

The `nvd` capture's in-band refusals drop **5 → 4**, and `0x00801909` leaves the set:

| # | record | id | ours | native | ours? |
|---|---|---|---|---|---|
| 1 | 50 | `0x2080012f` `GPU_QUERY_ECC_STATUS` | `0x56` | `0x56` | ✔ **AGREES with hardware** |
| 2 | 95 | `0x2080200a` `PERF_BOOST` | `0x56` | `NV_OK` | ⊘ **NOT ours** — 0 occurrences in our QEMU log; the guest's own `nvidia.ko` |
| 3 | 391 | `0x20801210` `cilp=2` | `0x56` | `NV_OK` | ⊘ **deliberate** — retried at 392 into WFI → our `NV_OK` (§3) |
| 4 | **422** | ★ **`RM_ESC_RM_FREE`** | **`0x56`** | `0x00` | ★★★ **OURS, AND UNEXPLAINED** |

★★★ **#4 is the next wall by identity, and it is in a plane no rung has looked at.** Traced
through our own boot:

```
serve_r1 i=366  RM_ALLOC hParent=0x5c000002 hObject=0x5c000072 class=0x83de  st=0x00
serve_r1 i=390  CONTROL  cmd=0x83de0309 on 0x5c000072                       st=0x00   ← we SERVE it (w292)
serve_r1 i=417  RM_FREE  hObject=0x5c000072                                 st=0x56   ← we REFUSE the free
run_w294cup2_probe.log:
  NVRM: rpcRmApiFree_GSP: GspRmFree failed: hClient=0xc1d0000c; hObject=0x5c000072; status=0x00000056
  NVRM: nvAssertFailedNoLog: Assertion failed: (status == NV_OK) … @ rs_client.c:844
run_w294cup2_qemu.log:
  bridge refusal BridgeRefusal::AllocClassNotPermitted::Refused x3 id=0x0000402c,0x000083de
```

⇒ **Class `0x83de` `GT200_DEBUGGER` is in `capability.rs`'s `DENIED_CLASSES`, while
`0x83de0309` — a control ON that class — is in `OBJECT_CONTROLS` and served.** We deny the
class, the guest's kernel builds the object locally anyway, we answer its control, and then we
refuse to free it. ⊘ **An internal inconsistency introduced by w292 and not noticed**, because
it is expressed as an `RM_FREE` status and every gate in this tree quantifies over *controls*.
⚠ It is **post-verdict** — `cuCtxCreate` has already returned — so it did not block this rung.
It is left for an owner decision because `DENIED_CLASSES` is a security-surface question.

### Other named, non-blocking refusals in the crossing boot's dmesg (all after `DONE`)

`NVA06F_CTRL_CMD_STOP_CHANNEL` `0x56` ×6, `NV2080_CTRL_CMD_GPU_EVICT_CTX` `0x56` ×6,
`NV2080_CTRL_CMD_INTERNAL_INIT_USER_SHARED_DATA` `0x56`, and the fault-buffer unregisters
(`status=0x56`, *"proceeding…"*). ⊘ Every one is teardown; none precedes `CUP2_RC=0`.

---

## 7. THE DIVERGENCE FROM A REAL GA106, RANKED BY KIND — the whole set, 613 vs 578 records

Aligned `host_reference_ga106/ce_r1.jsonl.zst` against `w294nvd_ce_r1.jsonl.zst`:

- **36 records missing in `cuInit`** (`0x215` `GPU_ATTACH_IDS`, `0x201`, `0x202`, `0x205`,
  `0x216` `GPU_DETACH_IDS`) — ⊘ **ENVIRONMENTAL**: the reference host is a multi-GPU rig
  (`/dev/nvidia7` appears in its own stream) and this is its per-GPU attach/detach probe loop.
  ★ The tree's own rule: *rank divergences by KIND, never by index.*
- **1 extra ioctl of ours** at 90 (`nr=0x46`).
- **the four status rows of §6**, of which one is ours and unexplained.
- **a REORDER** of the 12-record block `[timeslice, ALLOC 0x40, uvm, 0x00801909, ALLOC 0x40,
  uvm, FREE, FREE]` — native at 427-440, ours at 407-420. **Same set, different position.**

⊘ Everything else — all ~540 remaining records — aligns with matching status.

---

## 8. ⊘⊘ THE PRE-EXISTING RED IS **SIX** TARGETS, NOT ONE — measured, not assumed

The brief named one inherited failure (`kayfabe-isolate-host executor_vas_census`). Measured
at **`origin/master @ eed8de7`, unmodified, in a clean clone and a clean target dir**
(`cargo test --workspace --no-fail-fast`):

```
error: 6 targets failed:
  kayfabe-isolate-host  executor_vas_census                      ← the one the brief names
  kayfabe-isolate-host  guest_ring_census
  kayfabe-tests         ce_representability_split
  kayfabe-tests         doorbell_reaches_the_completion_observer
  kayfabe-tests         gpfifo_schedule                          ← deterministic, and FIXED here
  kayfabe-tests         ring_out_of_our_own_framebuffer
```

⚠ **`gpfifo_schedule` is not environmental — it is a stale list.**
`the_control_claim_is_exactly_these_ids` pins `OBJECT_CONTROLS` by **full membership**, and
neither w288 (`NV906F_CTRL_CMD_GET_MMU_FAULT_INFO`) nor w292 (the four input-only ids) updated
it. It has been red for two rungs. ⇒ Fixed **loudly** rather than inherited silently; the list
now carries all thirteen ids, each with its rung.

### Two gates this rung's own change turned red, and both resistances were RIGHT

| gate | why it fired | the fix |
|---|---|---|
| `capability::the_ported_surface_is_the_reviewed_size` + `each_origin_is_represented` + `each_boundarys_resolved_delta_is_materialised` | counters must move **with arguments** | `all_controls` 161→163, `Mode2Rpc` 7→9, and **all eight boundary rows +2 together** |
| `bind_channel::every_claimed_control_is_decided_even_when_malformed` | it sent **one garbage byte** to every claimed id, relying on *"1 is the wrong size for every control"* — true until `0x00802009`, whose params really **are** one `NvBool` | the probe now picks a length **against the id's own measured size** |

★ And the boundary table has **eight** rows: I bumped six by hand, the gate named
`550.90.07`; I bumped seven, it named `555.42.02`. ⇒ A hand enumeration of a list is precisely
what a gate quantifying over that list exists to replace.

### One more red, and it is a FLAKE — measured, not assumed

`kayfabe-linux-raw --lib` appeared in the w294 run and not in the baseline. `[measured, machine
idle]` it **passed, then failed, then passed at baseline** — and it failed on a *different test
each time* (`a_child_runs_from_an_image_with_no_path_at_all`, then
`one_image_spawns_more_than_once`). ⊘ This rung's diff touches **zero** files in that crate
(`git diff --name-only origin/master...HEAD | grep -c kayfabe-linux-raw` → `0`). ⇒ *Flaky
indicts the system, deterministic indicts the test* — it is a pre-existing process-spawn flake,
not this rung's.

### ⇒ NET: the red set goes 6 → 5, and this rung adds none

---

## FILES

| file | what |
|---|---|
| `run_w294cup2_{probe,dmesg,serial,qemu}.log` | ★ the crossing boot |
| `w294cup2_harness.log` | its full grading block, verbatim |
| `w294nvd_ce_r1.jsonl.zst` | the ioctl capture, 578 records, `ppre`/`ppost` on both sides |

Decode with `scripts/nvdiff_inband.py <capture>`; four states, never a bool.
★ **KNOWN-POSITIVE, RUN FIRST:** `scripts/nvdiff_inband.py ../nvidia-gpu-passthrough/traces/host_reference_ga106/ctx_r1.jsonl.zst`
→ exactly one `Refused`, seq 86 `0x2080012f`. Without it, *"no divergence"* is what a blind
reader prints.

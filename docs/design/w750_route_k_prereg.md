# w750 PRE-REGISTRATION — route K, phase 1: the birth-client probe

**STATUS: LIVE AS A REFERENCE, 2026-09-21 (w822).** Cited by `THE_DESIGN.md`, which is the
current architecture. ⊘ Where this file's *architecture* disagrees with that one, **that one
wins** — this predates it. ★ What stands here regardless: its **measurements**, its **ogkm
findings**, and the **reasoning** behind a constraint.


> ### STATUS — 2026-09-16 / **LIVE — PRE-REGISTERED, NOT YET RUN, NO BOX EXISTS YET.**
> Written and committed **before** the probe is built and before any GPU box is rented, per the
> brief's *"pre-register predictions and commit them before any box exists"*.
> Spec: `THE_CONSTRAINTS.md` **constraint 32** (the ruling) and
> `fable_leg_b_solution_space.md` **§3.1 / §6 / §7 / §9** (the discriminators).
> Base: `single-store` @ `8a842d90`, worktree `/workspace/kf-w750`, branch `w750-route-k`.
> ⊘ **No constraint is relaxed here, and none is proposed relaxed.**

---

## 0. What phase 1 is, and what it is NOT

It is a **userspace-only** probe: no guest, no KVM, no QEMU, no kayfabe device model. One binary
that forks, talks to `/dev/nvidiactl` + `/dev/nvidia0` with raw RM ioctls, and answers four
questions. It can **kill route K** — which is why the brief forbids writing the phase-2
integration until it has answered.

⊘ It is **not** a measurement of anything on the guest path, of BAR1/BAR2, of the raw client, or
of throughput. A green phase 1 says exactly four things and nothing else.

⊘ **A, B and the sysmem-USERD family are not probed.** They are refuted with evidence in
`fable_leg_b_solution_space.md` §1 and §3.5; re-deriving them would waste a box.

---

## 1. The sequence under test (constraint 32, restated so the probe can be graded against it)

Roles: **I** = per-guest-process isolate (child, unprivileged), **S** = scratchpad isolate
(parent, **caps dropped**), **B** = the birth client I mints, **the store** = a vidmem object S
holds.

1. I opens a **second** `/dev/nvidiactl` (**fd2**) and allocates client **B** (`NV01_ROOT`) on it.
   ⇒ `B.ProcID = I's tgid` (`client.c:112` `pClient->ProcID = osGetCurrentProcess()`;
   `nv-linux.h:532` `NV_GET_CURRENT_PROCESS() = current->tgid`).
2. I sends **fd2** to S over `SCM_RIGHTS` and **closes its own copy**. Only S holds fd2.
   ⇒ the client rides the `struct file`: `escape.c:309` sets
   `secInfo.clientOSInfo = nvfp->ctl_nvfp`, and STRICT validation
   (`g_system_nvoc.c:104`) compares it against `pClient->pOSInfo` (`client.c:91`).
3. S builds Device/Subdevice in B, dups **the store** into B (share or export/import fd), and
   dups **I's VAS** into B — **no grant needed**, default same-`ProcID` `DUP` policy
   (`sharing.c:341-352`, comparator `client_resource.c:217-228`).
   ★ Verified from source for this prereg: `serverCopyResource` validates **only the
   destination** client against the caller's fd — `rs_server.c:1694`
   `clientValidate(pClientDst, pParams->pSecInfo)`; the source client is never validated against
   the caller. That is what makes step 3 possible at all with S holding only fd2.
4. S births the channel **in B** on fd2, submitting `flags` with bit 5 **clear**.
   ⇒ `kernel_channel.c:277-294`: `privLevel` is **S's capability at this ioctl**;
   `ProcessID = pRmClient->ProcID` is **B's**, i.e. **I's**.
5. S **frees the store dup in B**. The channel's USERD sub-memdesc holds its own parent ref
   (`mem_desc.c:2676`), so the channel survives.

⚠ **S must not hold `CAP_SYS_ADMIN` at step 4.** `escape.c:304`
`secInfo.privLevel = osIsAdministrator() ? USER_ROOT : USER`, and `NV_IS_SUSER()` is
`capable(CAP_SYS_ADMIN)` (`nv-linux.h:537`) — **not** uid 0. On a root bench box that is the
default, so the probe drops caps in S before step 4 and **records the capability set it dropped
to**. A probe that forgot this would measure the known-positive and call it the test.

---

## 2. THE FOUR ROWS — each with its refuting value written down FIRST

Every row is a `KEY=VALUE` line on stdout. A row is **HELD** only if its predicted value is
observed **and** its known-positive/negative-control fired. Anything else is **REFUTED** or
**UNMEASURED**; those are different verdicts and the report must not merge them.

### Row 1 — `NVOS04_FLAGS_PRIVILEGED_CHANNEL` (bit 5) comes back CLEAR

| | |
|---|---|
| observable | `K_BIT5` = `(params[+20] >> 5) & 1`, read out of the **same** `NV_CHANNEL_ALLOC_PARAMS` buffer after `NV_ESC_RM_ALLOC` returns `NV_OK` |
| why it is readable | RM writes its verdict into the caller's params (`kernel_channel.c:281`, `:286`) and copies them back on success (`alloc_free.c:207-211`, copy-out skipped only `if (status != NV_OK)`) |
| **prediction** | **`K_BIT5=0`** — confidence ★★★★ **0.85** |
| **refuting value** | **`K_BIT5=1`** ⇒ the driver itself says the channel is privileged ⇒ **constraint 30 is violated at birth and route K is compromised.** Stop, report, do not integrate. |
| known-positive | `K_BIT5_KP=1`, from a forked child that **keeps** `CAP_SYS_ADMIN` and births the same channel in its own client. |
| ⚠ the trap | If `K_BIT5_KP=0` as well, the readback is reading nothing — the bit never moves — and row 1 is **UNMEASURED, not held**. A zero with no known-positive is exactly `a_census_zero_needs_a_known_positive`. |
| also recorded | `K_S_CAPS_SYSADMIN=0` (S's effective set at step 4) and `K_KP_CAPS_SYSADMIN=1`. Row 1 is void if these are not what they say. |

⊘ `NVOS04_FLAGS_PRIVILEGED_CHANNEL` is bit `5:5` (`alloc_channel.h:141`). **INFERRED** (from
`fable_leg_b_solution_space.md` §6, not re-derived here): nothing between `:294` and the return
rewrites `flags` on the USER arm.

### Row 2 — `ProcessID` lands as **I's**, not S's

> ### ⊘⊘ CORRECTED 2026-09-16, BEFORE THE BOX EXISTED — **THE INSTRUMENT NAMED BELOW CANNOT
> ### SEE A CHANNEL, AND WOULD HAVE ANSWERED AN EMPTY LIST THAT READ AS A REFUTATION.**
> The row below says *"`id` = the GPFIFO channel class"*. Read further into the handler and
> that is wrong: `subdeviceCtrlCmdGpuGetPids_IMPL` maps **every** class id that is not
> `NV20_SUBDEVICE_0` or `MPS_COMPUTE` to `classId(ChannelDescendant)`
> (`subdevice_ctrl_gpu_kernel.c:2289-2302`), and the match inside `gpuGetProcWithObject_IMPL`
> is `RES_GET_EXT_CLASS_ID(Object) == elementID` **on a `ChannelDescendant`**
> (`gpu_rmapi.c:797-810`). A `KernelChannel` is not a `ChannelDescendant`, so the query
> would have matched nothing.
> ⇒ **The probe allocates a copy-engine object under the channel in B and asks about THAT
> class** (`AMPERE_DMA_COPY_B` on this part, via `HostClasses::ce_object`). The object lives
> in **B**, so the pid reported is still `B.ProcID`, which is the stamp the row is about;
> only the handle the query can *see* changed.
> ⚠ The reading that produced the error was *"GET_PIDS takes a class id, the channel has a
> class id"* — a name that fits is not a mechanism that matches, and the failure mode would
> have been the quiet one: **an empty list is what a refutation looks like too.** This is the
> same class as `a_census_zero_needs_a_known_positive`, and it is why row 2 carries the
> subdevice query as its control.


| | |
|---|---|
| observable | `K_PIDS_CHAN` = the `pidTbl` from `NV2080_CTRL_CMD_GPU_GET_PIDS` (`0x2080018d`, `idType = _ID_TYPE_CLASS`, `id` = ⊘ **the CE OBJECT class, see the correction above** ⊘), issued from S's own subdevice; plus `K_PID_I`, `K_PID_S` (the two tgids) |
| why it reads the stamp | `gpuGetProcWithObject_IMPL` (`gpu_rmapi.c:699-800`) walks every client, matches objects of the class, and reports **`pClient->ProcID`** — the same field `kernel_channel.c:293` copies into the channel. The control is `RMCTRL_FLAGS_NON_PRIVILEGED` (`g_subdevice_nvoc.c:1126-1128`, flags `0x8`). |
| **prediction** | **`K_PID_I ∈ K_PIDS_CHAN`** and **`K_PID_S ∉ K_PIDS_CHAN`** — confidence ★★★★ **0.85** |
| **refuting values** | `K_PID_S ∈ K_PIDS_CHAN` ⇒ the stamp follows the **driving** process, and `client.c:112` is not what governs ⇒ K's identity claim is false. `K_PID_I ∉ K_PIDS_CHAN` with the channel alive ⇒ a third answer; record it, **do not explain it away** (§3.1's own instruction). |
| negative control | `K_PIDS_DEV` = the same control with `id` = `NV20_SUBDEVICE_0` **must contain both** `K_PID_S` and `K_PID_I`. If S's pid is absent there too, the instrument cannot see S at all and row 2's *"S absent"* is **vacuous** ⇒ UNMEASURED. |

⊘ Limit, stated up front: I's client **A** also carries I's `ProcID`, so *"I's pid appears"* is
only informative because the query is **keyed on the channel class**, and only B holds a channel.
S's client holds the store and no channel. That asymmetry is the whole discriminator.

### Row 3 — the freed dup is unmappable **while `GP_GET` still advances**

Both halves, or it proves nothing.

| | |
|---|---|
| observables | `K_DUP_MAP_PRE_RC` (map the store dup in B **before** the free), `K_DUP_FREE_RC`, `K_DUP_MAP_POST_RC` (map the same handle after), `K_GPGET_BEFORE`, `K_GPPUT`, `K_GPGET_AFTER` |
| **prediction** | `K_DUP_MAP_PRE_RC=0`, `K_DUP_FREE_RC=0`, **`K_DUP_MAP_POST_RC != 0`** (expected `0x57 NV_ERR_OBJECT_NOT_FOUND` or `0x33 NV_ERR_INVALID_OBJECT_HANDLE`; the row asks only for **non-OK**, and the exact code is recorded), and **`K_GPGET_AFTER == K_GPPUT` ≠ `K_GPGET_BEFORE`** — confidence ★★★ **0.8** |
| **refuting values** | `K_DUP_MAP_POST_RC=0` ⇒ the dup outlived the free ⇒ the transient full-use window inside S **does not close**, and §3.1's third cost is real rather than transient. **OR** `K_GPGET_AFTER == K_GPGET_BEFORE` ⇒ the channel never ran ⇒ "unmappable" is consistent with "the whole thing is dead" and proves nothing. |
| known-positive | `K_DUP_MAP_PRE_RC=0` **is** the known-positive for the post-free refusal: without it, *"unmappable"* cannot be told from *"we could never map it"*. |
| also recorded | `K_DUP_OUTSTANDING` — a count of live store dups in B at the end of the probe. Predicted `0`. §3.1 asks for a known-positive here too: the probe's `--skip-free` arm must make it `1`. |

### Row 4 — ⚠ UVM registration of a B-owned channel from **I's** UVM fd

This one is **INFERRED viable** in the survey (`uvm_user_channel.c:945-948` carries the vendor's
own *"TODO: Bug 1624521: This interface needs to use rmCtrlFd to do validation"*), and the brief
says: **if it fails, stop and report** — K needs a UVM answer before it can carry CUDA, and that
is an owner ruling, not something this lane may decide.

| | |
|---|---|
| observable | `K_UVM_REG_CHAN_RC` — `UVM_REGISTER_CHANNEL { rmCtrlFd = I's own ctl fd, hClient = B, hChannel }` issued on **I's** UVM fd, after `UVM_REGISTER_GPU` + `UVM_REGISTER_GPU_VASPACE` on `(A, I's VAS)` |
| **prediction** | **`K_UVM_REG_CHAN_RC=0`** — confidence ★★ **0.6**. The lowest confidence of the four, on purpose: this is the only row with no source line that *asserts* the behaviour, only one that admits the check is missing. |
| **refuting values** | `0x1B` `NV_ERR_INSUFFICIENT_PERMISSIONS`, `0x23` `NV_ERR_INVALID_CLIENT`, `0x33` `NV_ERR_INVALID_OBJECT_HANDLE`, or any other non-zero ⇒ **STOP AND REPORT.** Do not work around it, do not hand S the UVM fd, do not relax anything. |
| negative control | `K_UVM_REG_CHAN_NEG_RC` — the same call with a deliberately **wrong** `hClient` must be **non-zero**. If it is zero, UVM validates nothing at all and a green row 4 is **vacuous** ⇒ UNMEASURED. |

⊘ And an expiry condition, recorded now rather than discovered later
(`a_capture_derived_table_expires_as_a_vendor_regression`): row 4 depends on a check the vendor's
own comment says is **missing and intended**. A future driver that adds it breaks K silently. If
row 4 holds, the phase-2 integration must carry a fail-closed assert that names this, not a
comment.

---

## 3. Row 5 — recorded, NOT a gate: does birth zero a **vidmem** client USERD?

`fable_leg_b_solution_space.md` §7 asks it and says it orders every route. It is free here, so it
is taken; it gates nothing.

- observable: `K_USERD_POISON_SURVIVED` ∈ {0,1} — poison 512 B at the USERD offset in the store
  through S's own CPU view, birth, read back.
- **prediction: `1` (the poison survives)** — ★★★ 0.8. `kernel_channel.c:2342-2356` calls
  `kfifoSetupUserD_HAL` **only** when the USERD memdesc is `ADDR_SYSMEM`, or `ADDR_FBMEM` under
  full SR-IOV. Our store is FBMEM on a PF host ⇒ neither arm.
- ⚠ **It must be paired with `K_GPGET_AFTER` movement.** An unchanged page is also what a *wrong
  `userdOffset`* produces — the birth would then have programmed a different page and the poison
  would survive for the wrong reason. Unchanged + GP_GET moving ⇒ vidmem USERDs are not zeroed.
  Unchanged + GP_GET stuck ⇒ **UNMEASURED**.

---

## 4. What would make this probe lie, and what is done about each

| failure mode | the check that catches it |
|---|---|
| S still holds `CAP_SYS_ADMIN` ⇒ row 1 measures the known-positive | `K_S_CAPS_SYSADMIN` printed from `capget` **after** the drop and asserted `0` before step 4 |
| I and S share a tgid (a thread, not a fork) ⇒ row 2 is vacuous | `K_PID_I != K_PID_S` asserted; both printed |
| the bit-5 readback reads a stale buffer | the known-positive arm must print `1` from the **same** code path |
| the probe exits 0 having skipped a row | every row prints its key **always**; a missing key is a failure, and the runner greps for all of them. Exit status is **not** the instrument (`a_check_that_reports_is_not_a_check_that_gates`) |
| the job is killed and the empty file reads as "still running" | the probe prints `K_START=<utc>` first and `K_EXIT=<rc>` last; absence of `K_EXIT` is a **distinct** state from a missing file |
| `pgrep` used to check liveness | not used. Liveness is `K_EXIT` plus the box's own `ps` by `/proc/PID/comm` |

---

## 5. Infrastructure decisions, pre-committed

- **KVM template, not a CUDA container.** Nothing here needs `/dev/kvm`, but the row-1
  known-positive needs a task that **has** `CAP_SYS_ADMIN`, and a CUDA container has none
  (`rent_a_kvm_template_unless_the_test_is_bare_metal`). The brief's own carve-out.
- ALL code stays local; the box pulls. No executable is copied back. No secrets on the box.
- Destroy with `echo y | vastai destroy instance <id>` and verify with `vastai show instances`,
  verbatim, in the report. Only ids this session created.

---

## 6. Grading

Phase 1 **passes** iff rows 1, 2, 3 and 4 are all **HELD** by the definition in §2. Any REFUTED
row, and any UNMEASURED row, stops phase 2 and is reported as such — `w729b`: **a negative result
with its measurement is a full deliverable**, and it is not to be rescued by relaxing anything.

---

# ★★★★★ RESULTS — measured 2026-09-16, GA106, open kernel module 580.159.04

> ### STATUS — 2026-09-16 / **ANSWERED. PHASE 1 PASSES. All four rows HELD.**
> Box: vast `51207931`, RTX 3060 **GA106** (`10de:2503`), host driver swapped to the
> **open** module **580.159.04** — the exact version every `ogkm` citation in this file is
> read from. Binary `kayfabe-rm-ladder --route-k`, rev stamped in each log.
> Evidence committed: `traces/w750_route_k/` (`route_k_phase1.log` = run 1,
> `route_k_run2.log` = run 2, `route_k_run3.log` = **the graded run**,
> `box_provenance.txt`).
> ⊘ **No constraint was relaxed, and none is proposed relaxed.**

## The four rows, verbatim from run 3

| row | predicted | measured | verdict |
|---|---|---|---|
| **1** bit 5 clear | `K_BIT5=0` | **`K_BIT5=0`**, `K_CHAN_FLAGS_READBACK=0x00000000` | **HELD** |
| **1** known-positive | `K_BIT5_KP=1` | **`K_BIT5_KP=1`**, readback `0x00000020` | **FIRED** |
| **2** `ProcessID` is I's | I ∈ chan, S ∉ chan | **`K_PIDS_CHAN=76347`** = `K_PID_I`; `K_PID_S_IN_CHAN=0` | **HELD** |
| **2** negative control | both in the device query | **`K_PIDS_DEV=76342,76347`** — S **and** I | **FIRED** |
| **3** dup gone | `K_DUP_MAP_POST_RC != 0` | **`0x57 NV_ERR_OBJECT_NOT_FOUND`**; skip-free arm `0x23` | **HELD** — see the ⊘ below |
| **3** channel live | `GP_GET` advances | **`K_GPGET_AFTER=1`**, `K_SEM=0x57500001` (the engine's own release), `K_CHAN_ALIVE_AFTER_FREE=1` | **HELD** |
| **4** UVM takes a B-owned channel | `0x0` | **`K_UVM_REG_CHAN_RC=0x0`**, and `K_UVM_SCHED_RC=0` after it | **HELD** |
| **4** known-positive | `K_UVM_REG_CHAN_KP_RC=0x0` | **`0x0`** | **FIRED** |
| **4** negative control | non-zero | **`0x33 NV_ERR_INVALID_OBJECT_HANDLE`** | **FIRED** |
| **5** (recorded, not a gate) | poison survives | **`K_USERD_POISON_SURVIVED=0` — the page is ZEROED** | ⊘ **PREDICTION WRONG** |

★ Row 4 is stronger than the row asked for. `K_UVM_SCHED_RC=0` means the B-owned channel in
the externally-owned space **scheduled**, and `kchannelIsSchedulable_IMPL` refuses an
externally-owned channel whose allocations are unbound — the only userspace writer of
`bIsContextBound` is UVM's register (`uvm_user_channel.c:686`). ⇒ the registration did its
work; it did not merely return `NV_OK`.

## ⊘ ROW 3 — THE PREDICTED VALUE IS STRUCTURALLY UNOBTAINABLE, AND WHY THAT IS A FINDING

The prereg predicted `K_DUP_MAP_PRE_RC=0` and called it row 3's known-positive. **It measured
`0x23 NV_ERR_INVALID_CLIENT`, and the instrument is not at fault:**

```c
/* ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:2378, rm_create_mmap_context */
if (pRmClient->ProcID != osGetCurrentProcess())
    rmStatus = NV_ERR_INVALID_CLIENT;
```

★★★ **`NV_ESC_RM_MAP_MEMORY` refuses whenever the client's `ProcID` is not the CALLING task's.**
Under route K that is **S calling into a client stamped I**, so **S can never CPU-map any object
in B** — by construction, for every object, forever.

The instrument's own known-positive fires: `K_MAP_INSTRUMENT_KP=0` runs the identical
`map_memory` code path against a client whose `ProcID` **is** the calling task's. So `0` is
reachable and `0x23` is RM's answer.

⇒ Row 3 holds on evidence that replaces the unobtainable control with three others, two of
them pre-registered:
1. **The skip-free arm** (pre-registered as `K_DUP_OUTSTANDING`'s known-positive): with the dup
   **present** the same call answers `0x23`; with it **freed**, `0x57 OBJECT_NOT_FOUND`. *"We
   could never map it"* would give `0x23` in both. It does not.
2. **The dup was demonstrably usable while it existed** — the birth named it as
   `hUserdMemory[0]`, the `MAP_MEMORY_DMA` through it produced `K_STORE_VA`, and the channel
   ran over that VA.
3. `K_CHAN_ALIVE_AFTER_FREE=1` — a `GET_WORK_SUBMIT_TOKEN` control **on the channel** still
   resolves after the free, which `K_GPGET_AFTER_FREE` alone could not have shown (S reads that
   page through its **own** handle).

## ★★★ WHAT THIS GATE COSTS ROUTE K — stated, because it is a new constraint on the design

**Nothing, and here is the argument with its measurement.** The only pages S must touch are
USERD (`GP_GET`/`GP_PUT`) and its own doorbell window. Both are reachable through **S's own**
handles: the USERD is a slice of **the store**, which S owns, and the run reads and writes it
that way — `K_STORE_WRITE_OK=1`, `K_GPGET_BEFORE=0 → K_GPGET_AFTER=1` on the very page the
channel's USERD is in. S never needs a CPU view of an object in B.

★★ And it cuts in kayfabe's favour twice: it is an **independent, RM-side enforcement of
constraint 26** that nobody put there, and it means a compromised S cannot turn a birth client
into a CPU window onto the isolate's memory.

⚠ **It is also an expiry condition.** The gate is `ProcID`-based, so anything route K later
wants S to CPU-map *inside B* is refused with a status that names the client, not the mapping.
Design accordingly rather than discover it.

## ★★★★★ ROW 5 — THE PREDICTION WAS WRONG, AND THE CORRECTION ORDERS EVERY ROUTE

§7 reads `kernel_channel.c:2342-2356` as scrubbing a client USERD **only** for `ADDR_SYSMEM`
(or `ADDR_FBMEM` under full SR-IOV), and this file predicted at ★★★ 0.8 that a **vidmem**
USERD's poison would survive. Measured, on a PF host, open 580.159.04:

```
K_USERD_BEFORE_BIRTH=a5a50000 a5a50001 a5a50002 a5a50003 a5a50004 a5a50005 a5a50006 a5a50007
K_USERD_AFTER_BIRTH =00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
K_USERD_POISON_INTACT_WORDS=0 of 128
```

⇒ **A vidmem client USERD IS zeroed at birth.** The read-back **before** the birth is what makes
this a measurement rather than a guess — it rules out *"the write-combining store never landed"*,
which produces an identical `0 of 128`. And it is paired with `K_CHANNEL_LIVE=1`, which rules out
*"a wrong `userdOffset` left the poison somewhere else"*: the channel ran, so the page the birth
programmed is the page that was poisoned.

⇒ **Adoption must precede the guest's first `GP_PUT`, for every route.** Route K does by
construction (it births at the guest's alloc RPC). Any `UPDATE_CHANNEL_INFO`-shaped route must
re-point **before the first doorbell** or it wipes the guest's cursor mid-flight.
⚠ §7 of `fable_leg_b_solution_space.md` must carry this correction; the source reading there is
not wrong about the CPU-RM arm, so **something else does the scrub** and this probe does not say
what. That is the next question, not a settled one.

★★ **And one thing the run already narrows it to, for free.** The scrub is **targeted at the
USERD extent, not at the object.** The same store carries the GPFIFO entry at offset `0` and the
pushbuffer at `0x2000`, both written before the birth — and `K_CHANNEL_LIVE=1` with
`K_SEM=0x57500001` means hardware **fetched that entry and executed that pushbuffer** after the
birth. ⇒ those bytes survived; only the 512 B at `userdOffset` did not. That is exactly
`kfifoSetupUserD_HAL`'s shape (`kernel_fifo_gm107.c:797-808`, 512 B).
⇒ **The open question is narrowed from *"what wrote zeros"* to *"which arm reaches
`kfifoSetupUserD_HAL` on a PF host with the open module"*, given that
`kernel_channel.c:2342-2356`'s guard reads `ADDR_SYSMEM || (ADDR_FBMEM && bFullSriov)` and a PF
host is neither. ⊘ Still not settled — but it is one question now, not two.

## ⊘ TWO HARNESS DEFECTS THE CONTROLS CAUGHT — both would have produced a false report

1. **Row 2's first instrument could not see a channel at all.** `GET_PIDS` with the GPFIFO class
   matches `classId(ChannelDescendant)`, which a `KernelChannel` is not. Caught **by reading**
   before the box existed; the probe allocates a CE object and asks about that class. An empty
   pid list is exactly what a refutation of row 2 looks like.
2. ★★★★★ **Row 4 read as REFUTED for two whole runs, and the known-positive is what saved it.**
   Runs 1 and 2 measured `K_UVM_REG_CHAN_RC=0x31 NV_ERR_INVALID_OBJECT`. Run 2 added the control
   — a channel role I owns **itself**, same session, same space — and it answered **`0x31` too**.
   ⇒ not about the foreign client; about the harness. The cause is in this tree already:
   `kchannelGetEngine_GM107` resolves a channel's engine from its **runlist** and takes the first
   engine on it (`kernel_channel_gm107.c:722-727`), and on GA106 **CE0 shares runlist 0 with
   GR** — so a CE0 channel is graded by the graphics rule and RM looks for context buffers it
   does not have. `--uvm-mean` P2's known-good channel is **COPY(2)**, runlist 1. Both UVM arms
   moved to COPY(2) (`K_UVM_ENGINE_TYPE=0xb`, `K_UVM_KP_TOKEN=0x00010008 runlist=1`) and both
   went green.
   ⚠ **Without that control this lane would have told the owner that route K cannot carry CUDA.**
   That is the whole value of the rule, and it is the second time in this file that *a name that
   fits is not a mechanism that matches*.

# w750 PRE-REGISTRATION — route K, phase 1: the birth-client probe

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

# w289 RESULT — the probe BUILDS, the guest LEARNS, and both host `Xid`s are now ATTRIBUTABLE BY ADDRESS

`[measured 2026-08-13, vh, real GA106, driver 580.159.04, rev 738545dc]`
Binary `md5 4597dee0aaabac271d8760b61ca3fa5c` — **the same file on the native and the guest arms.**

---

## ⊘⊘ LEAD: THREE THINGS IN THE BRIEF ARE CONTRADICTED BY THIS RUN

1. **`0x80000016` is not one refusal, it is TWO of ours.** It decodes as `errno 22` (`EINVAL`), and
   fixing the half the previous sweep tested moves the refusal to the other end rather than
   removing it. Detail below.
2. **`w288nc1`'s host `Xid 31 CE0 @ 0x1_20000000` WAS attributable** — to **arm 1**, not arm 4.
   It is arm 1's `src` operand, exactly. The `RESULT` recorded it as unattributable because the
   client printed every field except an address; that is now fixed and the join is a string
   comparison.
3. **The GR fault is not the smallest reproduction of the wall.** Two host `Xid`s on this boot
   are **plain CE source-operand reads** at addresses this client dictated, with **no libcuda,
   no GR, no `cuCtxCreate`, and 82 ioctls**. See §4 — it reshapes task 2.

---

## 1. TASK 1 STEP 1 — `Other(2147483670)` DECODED, WITH ITS PRODUCER CITED

`2147483670 = 0x8000_0016`. `rm::ioctl_error` builds `RmError::Other(0x8000_0000 | errno)`
(`crates/kayfabe-isolate-host/src/rm.rs:1340`), and the named refusal constants are all
`0x4B4x` with the top bit clear (asserted at `rm.rs:8418`). ⇒ **`errno 22`, `EINVAL` — a Linux
syscall status, never an RM status.**

⚠ **The run's own artefact already named the producer and nobody read it.** `w288nc1`'s ioctl
census, line 47 of 59:

```
47: nr  39 RM_ALLOC_MEMORY  size 56  R33 arm4 hw-fault  errno 22
```

⇒ Same class as the banked `status: 56` trap: **the value was decoded by guessing at the
number instead of by finding the line that produced it.** The census had the answer in the
same file as the question.

### The two defects, one at each end of the same object

| end | what refused | why, from the driver's own source |
|---|---|---|
| **allocation** | `EINVAL` from `NV_ESC_RM_ALLOC_MEMORY` | `alloc_notifier_mem` left `_MAPPING` at `_DEFAULT`. For `NV01_MEMORY_SYSTEM` with `_ALLOC != _NONE` **and `_MAPPING != _NO_MAP`**, `RmIoctl` builds a CPU mmap context immediately, around `pApi->fd` (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:341-359`) → `nv_add_mapping_context_to_file` → `nv_get_file_private(fd)` returns NULL for our `fd: -1` (`kernel-open/nvidia/nv-usermap.c:44-46`) → `NV_ERR_INVALID_ARGUMENT` → the frontend's `-EINVAL`. |
| **CPU map** | `NV_ERR_INVALID_ARGUMENT` from `NV_ESC_RM_MAP_MEMORY` | A **system-memory** mapping is associated with the **control device** (`.../osapi.c:2266-2289`), so `nv_get_file_private(fd, ctl = NV_TRUE)` requires minor `NV_MINOR_DEVICE_NUMBER_CONTROL_DEVICE` (`kernel-open/nvidia/nv.c:4102-4106`). `map_cpu` hardcoded `/dev/nvidia<N>`. |

⊘⊘ **FIXING EITHER ALONE LOOKS IDENTICAL TO NOT FIXING IT.** That is exactly how the pair
survived a deliberate two-arm sweep at `rev f7a74bc` and was written up as *"the sysmem arm
may not survive natively"* — a verdict about the **aperture**, which was never the variable.

★★★ **THE INSTRUMENT LESSON:** the experiment varied *the flag the hypothesis named*, and
nothing else. **A sweep over one suspect can only indict or exonerate that suspect; it cannot
see that the other call in the pair is independently wrong**, so it reports *"neither setting
works"* where the true state is *"I have not found the variable yet."*

⚠ **`map_cpu`'s own doc stated the driver's rule correctly and then broke it** — *"RM chooses
the device node's state for an address inside a BAR and the control node's for system memory
… Everything mapped here is device-local, so it is always the per-GPU node."* True when
written; falsified the moment `alloc_notifier_mem` allocated sysmem; noticed by nobody,
because the node was a literal three lines into the body. It is a parameter now (`rm::MapNode`).

Both halves are pinned by `crates/kayfabe-isolate-host/tests/sysmem_notifier_node.rs` (4 tests,
green), whose failure text names the *other* half.

---

## 2. TASK 1 STEP 2 — THE PROBE IS BUILT. NATIVE KNOWN-POSITIVE, SAME BINARY.

`traces/boots/w289/w289_native.log.gz`. Two arms of one binary, minutes apart, SYSMEM first.

**ARM SYSMEM (`--ce-client-fault`) — the arm `w288nc1` could not construct:**

```
★ R33 arm 5 CONTROL  = the SAME 16 bytes read QUIET (status 0x0000 info32 0x00000000)
                       AFTER the positive control retired and BEFORE the fault was issued
★ R33 arm 5 NOTIFIER = PLANE A FIRED — status 0xffff, info32 0x0000001f, info16 engine 0x0001
★ R33 arm 5 WHERE    = PLANE D SPEAKS — GET_MMU_FAULT_INFO addr=0x0000000900000000
                       faultType=0x0 faultString="FAULT_PDE"
                       | VA-IDENTITY HOLDS: asked 0x0000000900000000, reported 0x0000000900000000
★ R33 arm 4 FAULTED  = pointed at 0x0000000900000000, did NOT retire, while its positive
                       control on the SAME channel did
  census: total=86 failed=0 — and NO ioctl carries an errno
```

Host `dmesg`, same seconds:
`Xid 31 … name=kayfabe-rm-ladd … CE0 HUBCLIENT_CE1 faulted @ 0x9_00000000 … FAULT_PDE ACCESS_TYPE_VIRT_READ`

**The join, natively — every field:**

| field | host `dmesg` | the client's own process |
|---|---|---|
| Xid code | `31` | `info32 0x0000001f` = **31** |
| address | `0x9_00000000` | `reported 0x0000000900000000` — **and `asked` is the same number** |
| fault type | `FAULT_PDE` | `faultString="FAULT_PDE"` |
| engine | `ENGINE CE0` | `info16 engine 0x0001` ⚠ engine **type**, not instance — the disclosed limitation |
| access | `ACCESS_TYPE_VIRT_READ` | (the notifier has no access field) |

★ The **VIDMEM** arm (w287's carried known-positive) fired identically ⇒ **the fix did not
regress the arm that already worked**, and the sysmem arm is not a fallback for it.
⊘ `R33_RC` is empty on both native arms and that is correct: it is written by the *guest hook*,
not the client. The native grade is `ARM_*_RC`.

---

## 3. TASK 1 STEP 3 — CRITERION 1 IN THE GUEST: **THE CODE MATCHES. THE ADDRESS DOES NOT EXIST.**

Boot `w289g`, carried arming 6/6 PASS, `traces/boots/w289/w289_guest.log`.

**★ MET — the probe BUILDS in the guest and the guest LEARNS, in its own process:**

```
★ R33 arm 5 NOTIFIER = PLANE A FIRED — status 0xffff, info32 0x0000001f, info16 engine 0x0001
★ R33 arm 5 IOCTL    = PLANE C SPEAKS — the next ioctl on the faulted channel refused
  census: total=82 failed=0  — and NO ioctl carries an errno  (`nr 39` is gone)
  PROBE-COULD-NOT-BE-BUILT lines = 0
  ERROR-NOTIFIER built = 3, REFUSED = 0, all naming the GUEST'S OWN pages
```

Host `dmesg` for this boot carries `Xid 31`. **`info32 0x1f` = 31 = the host's Xid code, joined
in one run.** ⇒ *"at least the error reporting works"* — **it does now**, and it did not before.

**⊘ NOT MET — and three separate reasons, none of which may be read as a pass:**

- **`PLANE D UNMEASURED = 2`.** `NV906F_CTRL_CMD_GET_MMU_FAULT_INFO` did not answer, so the
  run carries the fault's **code and not its address**. The device log contains **zero**
  `MMU-FAULT-INFO` relay lines ⇒ the guest's RM never RPC'd it to us. The control is
  allowlisted (`kayfabe-rmrpc/src/policy.rs:1758`); why RM answered it locally is unmeasured.
- ⇒ **The VA-identity oracle is UNMEASURED in the guest.** `VA-IDENTITY HOLDS = 0` and
  `VA-IDENTITY BROKEN = 0`. **Both zeros are VACUOUS.**
  ⚠ **AND MY OWN VACUITY GUARD WAS NECESSARY-NOT-SUFFICIENT.** The runner prints
  *"if PROBE-COULD-NOT-BE-BUILT is not 0, every zero below is vacuous"* — it **is** 0 here, so
  the guard passed while the zeros were vacuous anyway, for the *other* reason. A guard that
  covers one route to vacuity reads as covering all of them. Same shape as the trap it was
  written against.
- **The deliberate fault was NEVER ISSUED.** `arm 4 control = the POSITIVE CONTROL did not
  land`, so the probe is skipped by design. ⇒ the notifier that fired is attributable to the
  **control's** fault, not to a deliberate one — which the client says itself
  (`?? arm 5 CONTROL = the pre-fault read did not happen`).
- ⚠ **PLANE C "SPEAKS" WITH OUR OWN VOICE.** The refusal is `Other(19270)` = `0x4B46` =
  `rm::NOT_ON_THIS_RUNG` (`rm.rs:158`) — **our constant, not RM's status**. A plane that fires
  carrying our own refusal is not the driver telling the guest anything. Natively the same
  call returned `NV_OK` (PLANE C SILENT), so the two sides **diverge here** and neither
  reading is the driver's.

⇒ **Criterion 1: MET for the CODE, UNMET for the ADDRESS.** The blocker has moved from *"the
probe could not be built"* to *"the address plane has no answer in the guest"*, which is a
different and much narrower question.

---

## 4. ★★★★★ THE HEADLINE — BOTH HOST `Xid`s ARE ATTRIBUTABLE BY EXACT ADDRESS, AND THEY RESHAPE TASK 2

`traces/boots/w289/run_w289g_hostdmesg.log`, watermarked to this boot alone:

```
Xid 31 … name=memfd:kayfabe-i, channel 0x6 … CE0 HUBCLIENT_CE1 faulted @ 0x1_20000000 … FAULT_PTE ACCESS_TYPE_VIRT_READ
Xid 31 … name=memfd:kayfabe-i, channel 0x7 … CE0 HUBCLIENT_CE1 faulted @ 0x7_00100000 … FAULT_PTE ACCESS_TYPE_VIRT_READ
```

| host address | what it is, by identity | source |
|---|---|---|
| `0x1_20000000` | **arm 1's `src` operand**, printed by the client this run: `info R33 arm 1 OPERANDS = src 0x0000000120000000 dst 0x0000000120010000` | the new join line |
| `0x7_00100000` | **`CTRL_SRC_AT`** = `REACH_PROBE_WINDOW.0 + 0x10_0000` = `0x7_0000_0000 + 0x10_0000` — arm 4's **positive-control source** | `rm.rs`, compile-time constant |

Both matches are exact, not approximate. ⇒ **and `0x1_20000000` is the SAME address `w288nc1`
faulted at**, so it is deterministic across two boots.

### What this says

- **Both faults are a copy engine READING ITS SOURCE OPERAND** — `FAULT_PTE`,
  `ACCESS_TYPE_VIRT_READ` — at a VA this client dictated and mapped with `raw_map_dma` into
  the guest-bound VAS.
- **The identical binary, the identical addresses, works natively** (§2: arm 1 `★ COPY`,
  operands at the same `0x1_2000_0000` / `0x1_2001_0000`).
- ⇒ **The wall reproduces with 82 ioctls, no libcuda, no GR engine, no `cuCtxCreate`, and one
  4 KiB copy.** It is not GR-specific and it is not about `0x72a5_fee00000`.
- ⇒ `FAULT_PTE`, not `FAULT_PDE`: the descent **reached the page-table level and found no valid
  PTE**. The directory existed; the leaf did not.

### ⊘ THE OWNER'S DIFFERENTIAL IS DOMINATED — a better-keyed version of the same question exists

The proposal is: run `cuCtxCreate` natively, dump the host's GPU-VA→phys table, diff against
our GPGA→GPU-VA table. **The join key does not exist.** A native `cuCtxCreate` builds its own
VAS through its own allocator; under UVM unified addressing the GPU VA *is that process's*
process VA, so the native VA set and the guest VA set are **two different draws from one
space**. Aligning them by position is precisely the banked *"a discrepancy can be an artefact
of a JOIN"* failure, and there is no key to fix it with.

**The same question, keyed perfectly:** ask whether **`0x1_2000_0000`, in the VAS handle we
ourselves named (`range 0xcafe0005`), has a PTE on the host.** Same address space, both sides,
one number. `NV0080_CTRL_CMD_DMA_GET_PTE_INFO` (`0x801801`) answers exactly that:

- it takes `gpuAddr` **and `hVASpace`** (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl0080/ctrl0080dma.h:415-445`);
- it is `deviceCtrlCmdDmaGetPteInfo_IMPL` — **a direct `_IMPL`, not a `_DISPATCH`**
  (`src/nvidia/generated/g_device_nvoc.c:733-745`), so the HAL trap does not apply: there is
  one implementation and it is the one that runs;
- it has **no privilege check**, unlike `deviceCtrlCmdDmaUpdatePde2_IMPL` immediately below it
  which demands `RS_PRIV_LEVEL_KERNEL` (`src/nvidia/src/kernel/gpu/mem_mgr/dma.c:315-318`) ⇒
  our unprivileged host client can call it;
- it is a **read on the RM control plane** — the owner's place (2), never the data path.

⊘ **STATE ITS BLIND SPOT BEFORE TRUSTING IT.** It resolves through `vaspaceGetPteInfo` on RM's
own `OBJVASPACE` (`dma.c:281-285`), so it reports **what RM believes it mapped** — i.e.
**populate source (1), bind-time RPC/ioctl bindings, only.** It is structurally blind to
source (2), the observed CE page-table write that `mode2_address_table.md` names as co-equal.
**That blindness is the instrument's value here**: if `0x1_2000_0000` has no PTE by source (1),
we know which source was supposed to own it and that it did not fire.

⚠ `NVA080_CTRL_..._VGPU_DEV_CAPS_GET_PDE_INFO_CTRL_DISABLED` exists (`ctrla080.h:348`), so the
control is disable-able under vGPU. Not our configuration; recorded so a later reader does not
find it absent and call it a bug.

**Not built this rung.** Naming it and stopping is deliberate: the brief's own instruction was
not to build a half-oracle, and the pointwise version needs one boot to be worth anything.

---

## 5. LEDGER

| | |
|---|---|
| Task 1 step 1 — decode `0x80000016` | **DONE**, producer cited to the driver line |
| Task 1 step 2 — provoke a fault in the guest | **DONE natively** (`Xid 31 @ 0x9_00000000`, joined). **In the guest a real fault WAS provoked and observed** — but by the probe's *control*, not deliberately |
| Task 1 step 3 — same fault BY IDENTITY | **CODE: MET** (`info32 0x1f` = Xid `31`). **ADDRESS: UNMET** — plane D unmeasured, and the two VA-identity zeros are VACUOUS |
| Task 2 — the GR fault | **REFRAMED, not answered.** A smaller, deterministic, address-attributed reproduction exists on the CE path; the proposed native differential has no join key; a pointwise, perfectly-keyed replacement is named with its blind spot |
| Owner's four places | **Not crossed.** Everything here is RM control plane (2) and diagnostics. `MapNode` changes which `/dev` node an ioctl is issued on — it does not put us in any data path |

**Disclosed and untouched:** `--all-targets`, `fmt` and three census tests were red at `727a112`
before this rung. Not touched.

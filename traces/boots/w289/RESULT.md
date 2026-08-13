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

---

# ADDENDUM — the coordinator's two follow-ups, and what they produced

## 6. ⊘ PRIORITY 1 IS REFUTED: `GET_PTE_INFO` IS TEST-ONLY AND A RELEASE DRIVER REFUSES IT

`[measured 2026-08-13, vh, real GA106 580.159.04]` `NV0080_CTRL_CMD_DMA_GET_PTE_INFO` answered

```
Other(126) = NV_ERR_TEST_ONLY_CODE_NOT_ENABLED (0x7E)
```

for **every** address — including the ring, which arm 2 had just proved occupied.

★★★★★ **THE IN-RUN CALIBRATION IS WHAT CAUGHT IT.** Without it the arm would have printed six
`ABSENT` rows and they would have read as *"we found the missing mappings"* — the single most
dangerous false positive available on this rung. Instead it printed `FAIL CALIBRATION` and
refused to treat its own rows as measurements. **The guard was built one commit before it
fired.**

Explained from source **after** the measurement:
`flags 0x100008` (`g_device_nvoc.c:733-745`) ∧ `RMCTRL_FLAGS_RM_TEST_ONLY_CODE = 0x00100000`
(`inc/kernel/rmapi/control.h:323`) ∧ refused unless `PDB_PROP_SYS_ENABLE_RM_TEST_ONLY_CODE`
(`src/kernel/rmapi/control.c:855-861`).

★ **Two things the flags settle for free**, both of which the brief asked to be resolved rather
than assumed:
- Neither `GET_PTE_INFO` nor `GET_PDE_INFO` sets `ROUTE_TO_PHYSICAL` (`0x40`), so
  `NVOC_EXPORTED_METHOD_DISABLED_BY_FLAG` is false for both ⇒ **the bound implementation IS the
  `.c` one reads.** The standing nvoc-HAL trap is closed for this family, in both directions.
- `0x8` = `RMCTRL_FLAGS_NON_PRIVILEGED` — an independent confirmation of the "no privilege
  check" reading I had taken from the function body alone.

## 7. ⊘ ARM 6 IS BUILT AND IS **NOT YET AN ORACLE** — stated plainly

Swapped to `GET_PDE_INFO` (`0x801809`, flags `0x10008`, **not** test-only). Two further failures,
both caught by the same calibration:

| attempt | result | cause |
|---|---|---|
| v2 | `Other(51)` on every address | `NV_ERR_INVALID_OBJECT_HANDLE` (**`0x33`** — 51 *decimal*; ⚠ `0x51` *hex* is `NV_ERR_NO_MEMORY`, a different code, and that hex/decimal slip is the banked `status: 56` trap). I passed the `NV01_MEMORY_VIRTUAL` **range** as `hVASpace`. **One `Vas` is TWO host objects** and `alloc_vaspace` returns the range; the `FERMI_VASPACE_A` is its companion. ⊘ `alloc_vaspace_raw`'s own R7b comment records paying for **the mirror image of this**, with the same status. |
| v3 | ring ⇒ `PDE PRESENT`; **all four others ⇒ `Other(19270)`** | `19270 = 0x4B46 = NOT_ON_THIS_RUNG` — **our own sentinel again**, not RM's answer. Unexplained. |

⊘⊘ **AND THE ONE "PRESENT" IS NOT TRUSTWORTHY EITHER.** It reports `pageSize 0x20000000` — **512
MiB** — which is the *whole-FB alias* shape the C's own decoder singles out and refuses to back
(`nvkvm_gpu_emul.c`: *"512 MiB leaf (FB alias): never back"*). A calibration positive that may be
an alias artefact is not a calibration.

⇒ **Arm 6 answers nothing about the two faulting addresses this rung.** Recorded as built,
wired, and **uncalibrated** — not as a result. ★ The value delivered is the **guard**: three
different failure modes on three consecutive runs, and not one of them produced a false finding.

## 8. ★★★★★ THE C's SOURCE LIST — SETTLED FROM THE C's SOURCE, AND OUR TWO DOCS DISAGREED

`CLAUDE.md` said the C was **PDB-derived only** (doorbell PT sweep + observed CE write);
`mode2_address_table.md` said one of the two co-equal sources is a **bind-time RPC/ioctl
binding**. **`mode2_address_table.md` is right.** There is a **third source and it is an RPC
capture**:

- `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` (`0x2080012b`) snooped in flight —
  `nvkvm_snoop_promote_ctx` (`src/qemu/nvkvm_gpu_emul.c:2446-2472`);
- its `{gpuPhysAddr, gpuVirtAddr, size, physAttr}` entries folded into a side table by
  `nvkvm_record_va_map` (`:2417-2440`);
- and that table is what `nvkvm_chan_translate` **consults FIRST** (`:305-309`);
- backed by `nvkvm_m2_back_and_map` (`:3902`).

⇒ **THREE sources.** Correction folded into `CLAUDE.md` **above the text it corrects**
(nvidia-gpu-passthrough `67b7ed3`).

### ★★★★★ AND THE LINE THAT MATTERS FOR THE FIX

The C rounds **every** promote-derived mapping **up to 64 KiB** before mapping it
(`nvkvm_gpu_emul.c:7920`). This port binds at the **declared length** — there is no rounding
anywhere in `crates/kayfabe-core/src/promote.rs`.

That is precisely what produces w277's `0x8600`-long, non-page-aligned rows and the **sub-page
hole** w277 recorded and left open: *"2 560 bytes our own `resolve` answers `Miss` for inside a
page the guest has mapped"*, held that way by the `CrossesEnd` refusal.

⇒ **The C could not have that hole. We have it by construction.** ⚠ **Not measured against a
fault** — a mechanism with both sides cited, and the first thing to test on the 82-ioctl CE
reproduction.

⊘ One more semantic to match: the C treats `st == 0x51` (`NV_ERR_NO_MEMORY`) on a **FIXED** map
as **success** — *"the VA is ALREADY mapped in the host VASpace"* (`:7935-7938`). A port that
reads that as failure refuses exactly the ctx buffers the host RM already placed.

## 9. THE TWO SILENT BUGS, CLOSED

- **`scripts/bench/w288n_notifier_run.sh` had `finish 0` above its "SHARPENED BAR" join ⇒ that
  section had never executed on any run**, while the runner exited 0 and the log ended tidily.
  Moved below. ⇒ It is why `w288nc1` needed an ad-hoc `crit1` script: the join it wanted was
  right there, unreachable. What it produces when it runs is §3 of this document.
- **arm 4's `ControlFailed` line now prints the control's own operand addresses** — the arm that
  actually fired in the guest, printing every field except the one the host log names.

## 10. ⊘ WHAT I DID NOT DO

`cap3` mining. It is `traces/mode2_c_reference/cap3_matmul_forwarding.rec.zst` and it is
committed, so the question *"does it record address-table population at all?"* is answerable
without a boot — but the answer now matters less than it did: the population **mechanism** has
been settled from the C's source directly, which is stronger evidence than a trace and is not
subject to cap3's stated non-hermeticity (`pci_dma_map` is uninstrumented). ⚠ What cap3 could
still add is **which VAs** the promote path actually carried on the green run. Named, not done.

---

# ADDENDUM 2 — BOTH CANDIDATES REFUTED FOR THIS REPRO, AND CRITERION 1'S GUARD REBUILT

## 11. ⊘⊘⊘ LEAD: **THE PROMOTE PATH IS NOT ON THIS REPRO'S PATH AT ALL**

Rank-1 (parked promote halves) and rank-2 (the 64 KiB rounding) are **both** properties of the
`GPU_PROMOTE_CTX` source. Measured on boot `w289g`, the same boot that produced the faults:

```
promote lines naming the client (proc=2) ....... 0
0xc7c0 (AMPERE_COMPUTE_B) anywhere on the boot .. 0
```

⇒ **The raw CE client never allocates a compute class, so it has no GR context, so it never
issues `GPU_PROMOTE_CTX`.** Its operands are its own `alloc_device_local` + `map_dma_both`, i.e.
guest `NV_ESC_RM_MAP_MEMORY_DMA` bindings — **a different source entirely**.

⇒ **Neither candidate can explain `0x1_20000000` or `0x7_00100000`.** ⚠ And the coordinator's
own separation test is what settles it, so the answer is *"neither"*, not *"unclear"*.

## 12. THE COORDINATOR'S SEPARATION TEST, ANSWERED: **NO BINDING AT ALL**

> *"no binding at all ⇒ parked half. A binding that ends before the fault offset ⇒ extent policy."*

**Neither — there is no binding, and the promote source is absent, so it is a third thing.**
From the device's own log, `run_w289g_qemu.log:90` and `:131`:

```
OPERAND-TABLE: 2 page(s) asked, 0 resolved in guest RAM, 2 MISS
   [va=0x120000000 : Miss { pdb: Pdb(0x6000), va: 0x120000000 }
    va=0x120010000 : Miss { pdb: Pdb(0x6000), va: 0x120010000 }]
OPERAND-TABLE: 2 page(s) asked, 0 resolved in guest RAM, 2 MISS
   [va=0x700100000 : Miss { pdb: Pdb(0x4000), va: 0x700100000 }
    va=0x700200000 : Miss { pdb: Pdb(0x4000), va: 0x700200000 }]
```

★★★ **FOUR of four operand pages MISS — including both WRITE destinations, not only the READ
sources the `Xid`s name.** And the faulting offset is **`0x0`** in both cases: the fault is on
the operand's **own first page**, not past its end.

⇒ **THE 64 KiB ROUNDING IS REFUTED TWICE OVER.** Rounding extends a mapping's *end*; it cannot
turn a `Miss` at offset 0 into a hit, and there is no binding to extend. ⊘ The native control
seals it: the **same binary, same lengths, same addresses, passes** — so the declared length is
demonstrably sufficient.

⇒ The live question is not *"how long did we bind"* but **"why is there no row at all for a VA
the guest's own `MAP_MEMORY_DMA` established?"** — the RM-capture fork, on the
`MAP_MEMORY_DMA` source rather than the promote source.

## 13. ★★★ BUT THE RANK-1 LEAD IS CORROBORATED — **FOR THE OTHER FAULT**

The same boot's promote tally, by identity:

```
promote-ctx ACCEPTED: bound=4  joined=0  joined_global=0  already=0  parked=5  half_already=0
promote-ctx TALLY:  {bid=0x0 phys=0 va=0 complete=2} {bid=0x2 phys=0 va=0 complete=2}
                    {bid=0x3 phys=0 va=2 complete=0} ...
bridge refusal PromoteFault::UnknownContextObject x2
```

★ **`joined=0`, `parked=5`** — and `bid=0x3` carries **two virtual halves, zero physical,
`complete=0`**: a half that arrives and publishes nothing, by identity, exactly the shape the
coordinator described and exactly w276's *"aimed right, binds ZERO."*

⇒ **Two faults, two mechanisms, and they must not be merged:**
| fault | source | state |
|---|---|---|
| the 82-ioctl CE repro (`0x1_20000000`) | guest `MAP_MEMORY_DMA` | no row at all — **open** |
| the GR / `cuCtxCreate` fault | `GPU_PROMOTE_CTX` | `joined=0 parked=5` — **rank-1 lead corroborated** |

⚠ The coordinator's candidates are not wrong; they are **aimed at the other fault**. Refuting
them here must not be read as refuting them there.
⊘ And I did **not** verify that any parked half covers the GR fault's VA — that is the check
still owed, and `bid=0x3` is where to start.

## 14. ★★★★★ CRITERION 1'S GUARD IS REBUILT — AND THE GUEST RUN NOW SAYS WHY

`rm::Crit1State`: five named exits, exactly one printed per run, on **every** path, beside
`VA-IDENTITY MEASURED = yes|no`. A test asserts exactly one state may license a VA-identity
claim.

Verified on all three polarities at rev `f589f12`:

```
native --ce-client        ★ R33 CRIT1 STATE = ARM-NOT-SELECTED             MEASURED = no
native --ce-client-fault  ★ R33 CRIT1 STATE = FAULT-PROVOKED-ADDRESS-READ  MEASURED = yes
GUEST  --ce-client-fault  ★ R33 CRIT1 STATE = CONTROL-NEVER-LANDED         MEASURED = no
```

★★★ **`CONTROL-NEVER-LANDED` is precisely the state the old boolean guard could not see** — the
one `w289g` was in while its guard reported "not vacuous". The run now labels its own numbers.

⊘ **Criterion 1's address half is still UNMET, and it is now unmet *legibly*:** one anchored
token says the deliberate fault was never issued, so the two VA-identity zeros are vacuous **by
name** rather than by a reader's inference. The blocker is unchanged — the CE control cannot
land in the guest — and it is the same wall as §12.

⚠ Third consecutive boot, same two addresses, same `FAULT_PTE / VIRT_READ`: `0x1_20000000` and
`0x7_00100000`. Deterministic.

---

# ADDENDUM 3 — **WE FILE THE RING AND NOTHING ELSE**, and rank-1 is refuted into something sharper

## 15. THE MEASUREMENT (boot `w289g` @ `4e59899`, `TABLE-HOLDS` ungated)

```
OPERAND-TABLE: 2 asked, 0 resolved, 2 MISS [va=0x120000000 … va=0x120010000]
TABLE-HOLDS:   [proc=2 gpu=0 pdb=0x6000 rows=1 runs=1  0x120020000+0x10000]

OPERAND-TABLE: 2 asked, 0 resolved, 2 MISS [va=0x700100000 … va=0x700200000]
TABLE-HOLDS:   [proc=2 gpu=0 pdb=0x4000 rows=1 runs=1  0x700000000+0x10000]
                                                                    ^ PROBE_RING_AT
```

★★★★★ **Both VA spaces hold EXACTLY ONE ROW, and in both it is the channel's RING.** Every
operand is absent — including both write destinations.

⊘ **`rows=1` is the built-in known-positive.** The table is not broken and `resolve` is not
broken: the ring's row is there and resolves. So `MISS` on the operands is **an unfiled row**,
not a failed lookup — which is the distinction a bare `MISS` could never make, and the reason
this instrument had to exist before the finding could.

★ Joined with §12/§14: the three VAs `0x120000000` / `0x120010000` / `0x120020000` are 64 KiB
apart **inside one 2 MiB page table** which our own descent decoded as **`lf3` — three leaves —
attributed to `byBAR2#85`** (`ceresolve.rs:742`, `lf{}` = `d.leaves.len()`). ⇒ the guest wrote
three leaves through BAR2; we filed one row, and it is the one that does **not** come from the
page-table decode.

## 16. ⊘ RANK-1 AS BRIEFED IS REFUTED — BAR2 writes DO bind

*"Do we watch BAR2 as a READ path only?"* — **No.** There is a complete, default-armed path:

```
plane.rs:3092  BAR2 store lands in FbStore, tagged /byBAR2
plane.rs:3119  the 4 KiB frame(s) enter `pt_witness`   ← marks a PAGE, never decodes a value
shim.rs:8956   decode_cpu_pt_writes → drain_pt_witness  ← NOT gated; runs every doorbell
device.rs:3452 pt_page_owner(gpu,page) → vas.pt_pages.insert(page)
ptdecode.rs:601 decode_subtree re-reads the page from the SAME FbStore and decodes the PTEs
ptdecode.rs:781 apply_settlement → walker.rs:1071/1101 → AddressTable::bind
```

⇒ The binding is **deferred and address-keyed**, not payload-keyed. Rank-1's premise is wrong.

## 17. ★★★★★ BUT THE RESIDUE IS THE ANSWER — A BOOTSTRAP GAP, AND IT IS UNSWEPT BY DEFAULT

`pt_page_owner` is bootstrapped **only from declared roots plus whatever a prior decode already
published** (`device.rs:3386-3393`). ⇒ **A BAR2-written page whose parent has never been decoded
is never attributable**, so it recirculates through `requeue_pt_witness` (`shim.rs:9031`)
**forever and never binds**. Attribution requires a decoded parent; the parent is only decoded
if it was itself attributable.

★★ The mechanism that closes it is the **sweep**, which starts from the VAS's *installed root*
rather than from the witness set — `sweep_cpu_pt_tables` (`shim.rs:9094`) — and it is
**gated OFF by default** (`selected_pt_sweep()`, `shim.rs:13884`, false when `KAYFABE_PT_SWEEP`
is unset). ⊘ **Every boot of this repro ran with it unset** — `w289_guest.sh` says
`unset KAYFABE_PT_SWEEP` explicitly.

⇒ **That is consistent with every number above**: the only row we hold is the ring, and the ring
is bound by the channel path, not by the PT decode. **The PT-decode path has bound ZERO rows for
both VA spaces.**

## 18. PRE-REGISTERED — the one-boot, ZERO-CODE-CHANGE test

Run the identical arm with **`KAYFABE_PT_SWEEP=on`**, one variable, everything else carried.

| outcome | reading |
|---|---|
| the operands bind (`rows>1`, runs covering `0x120000000`) | **bootstrap gap CONFIRMED.** The fix is to attribute from the root, not only from the witness set — and the sweep must not be opt-in. |
| the sweep runs and still binds zero for these VAs | **w276 generalises to the CE plane.** *"The sweep is aimed right and binds ZERO"* stops being a `cup2` fact and becomes reproducible in 82 ioctls — a much cheaper harness for it. |

⚠ **w276 is real counter-evidence and is registered as such before the boot**: it measured the
whole-VAS sweep publishing **zero**. I am not predicting a pass. ⊘ Either result is a full
result, and the second is arguably the more useful one because of the harness it hands over.

⊘ **And note what this does NOT explain:** the native arm passes with no sweep at all, because
natively there is no emulated address table in the path — the host RM owns the VAS directly. The
native control brackets the guest path; it does not share this mechanism.

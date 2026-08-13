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

---

# ADDENDUM 4 — **THE PRE-REGISTERED TEST CONFIRMS THE BOOTSTRAP GAP, AND THE FAULT DOES NOT MOVE**

Boot `w289s` @ `8b8b8a3`, one variable: `KAYFABE_PT_SWEEP=on`. Everything else carried.

## 19. THE SWEEP RAN, AND IT BOUND EVERY MISSING ROW

```
PT-SWEEP tasks=3 skipped=0 ran=3 truncated=0 pages=43      ← the arm REPORTS ITS OWN EXECUTION
```

| VAS | sweep OFF (`w289g`) | sweep ON (`w289s`) |
|---|---|---|
| `pdb=0x6000` (arm 1) | `rows=1 runs=1  0x120020000+0x10000` | **`rows=3 runs=1  0x120000000+0x30000`** |
| `pdb=0x4000` (arm 4) | `rows=1 runs=1  0x700000000+0x10000` | **`rows=3 runs=3  0x700000000+0x10000, 0x700100000+0x10000, 0x700200000+0x10000`** |
| operand MISS count | **2 + 2** | **0 + 0** |

★★★★★ **`MISS` went 2 → 0 on both channels, and the arm-1 VAS coalesced into ONE contiguous run
covering all three 64 KiB objects.** The pre-registered *"operands bind"* branch is the one that
happened.

⇒ **THE BOOTSTRAP GAP IS CONFIRMED.** `pt_page_owner` seeded only from declared roots + prior
decodes cannot attribute a BAR2-written page whose parent was never decoded; the root-seeded
sweep can, and does — 3/3 on both address spaces.

⊘ **AND IT SCOPES `w276`.** *"The sweep is aimed right and binds ZERO"* is **not** a property of
the sweep: here it binds everything that was missing. w276's zero belongs to its own VAS and
workload, and this 82-ioctl repro is a far cheaper harness for re-asking it.

## 20. ⊘⊘⊘ **AND THE FAULT IS BYTE-FOR-BYTE UNCHANGED**

```
w289g (sweep off):  Xid 31 … faulted @ 0x1_20000000 … FAULT_PTE ACCESS_TYPE_VIRT_READ
                    Xid 31 … faulted @ 0x7_00100000 … FAULT_PTE ACCESS_TYPE_VIRT_READ
w289s (sweep on ):  Xid 31 … faulted @ 0x1_20000000 … FAULT_PTE ACCESS_TYPE_VIRT_READ
                    Xid 31 … faulted @ 0x7_00100000 … FAULT_PTE ACCESS_TYPE_VIRT_READ
Xid count: 2 vs 2.  arm 1 COPY: FAIL on both.  CRIT1 STATE: CONTROL-NEVER-LANDED on both.
```

⇒ **COMPLETING OUR TABLE CHANGED NOTHING THE HARDWARE CAN SEE.** ★ That is the sharpest thing
this rung measured: **our address table is our own bookkeeping, and a row in it does not
establish a host-side mapping.** A fix that only fills the table is a fix that changes no Xid.

⚠ **This is a `NECESSARY-BUT-NOT-SUFFICIENT` result and must be reported as one** — the same
shape as w260's join. The gap was real, closing it was right, and the wall is one layer deeper.

## 21. ★★★ THE NEXT LAYER, NAMED BY THE INSTRUMENT ITSELF

With the rows present, the operand line changes its refusal:

```
OPERAND-TABLE: 2 asked, 0 resolved in guest RAM, 0 MISS, 2 NOT-IN-GUEST-RAM
               [va=0x120000000:Vidmem@0x10000  va=0x120010000:Vidmem@0x20000]
```

⇒ The operands **are bound, and their aperture is `Vidmem`** — the client allocated them with
`alloc_device_local`. The pin path publishes **guest RAM** and **refuses Vidmem by name**
(`shim.rs`: *"an operand that binds in the framebuffer is a real and served case … calling its
`Binding::phys` a guest-physical address would be the one reinterpretation `pin_ring_guest_ram`
refuses"*).

⇒ **Nothing was ever published into the host VAS for these VAs, which is exactly why the host CE
faults at them.** ★ And it explains the native control cleanly: natively the host RM owns that
VA space directly, so no publication step exists to be skipped.

⊘ **Not yet established:** whether publishing a Vidmem-aperture operand is the right fix or
whether these operands should not be forwarded at all. That is a design question for the owner's
four places — it would put us in the operand path — and this rung **stops here and reports**
rather than deciding it.

⊘ One unexplained row, recorded not chased: a `pdb=0x0` VAS also gained `rows=2 runs=1
0x120000000+0x20000` under the sweep. A PDB of zero is not a page-directory base; it should be
accounted for before anyone builds on these numbers.

---

# ADDENDUM 5 — THE OWNER'S QUESTION: **ALL 82 PASS ON BOTH ARMS. NOTHING IS SILENTLY REFUSED.**

> *"but since you have the 82 ioctls, do they all pass on host, and at which point does it crash?"*

## 22. THE INSTRUMENT HAD TO BE BUILT FIRST — `failed=0` COULD NOT ANSWER THIS

The census recorded **only the syscall `errno`**. RM writes its refusal **into the parameter
struct** while `ioctl(2)` returns `0`. ⇒ every prior *"N ioctls, 0 failed"* — including w278's
*"the guest fails with every ioctl served"* — **was reading a field that cannot see a refusal.**

`IoctlRecord` now retains a 72-byte reply prefix; the ABI-owning consumer decodes the status per
escape from `kayfabe-abi`'s own `ct_assert`ed offsets (`NVOS00@12 NVOS21@28 NVOS54@28 NVOS02@40
NVOS33@40 NVOS47@40 NVOS46@56`). Four states, never a bool: `Ok / Refused(status) /
NoStatusField / Truncated` — a truncated snapshot never decodes to `Ok`.

★ Captured **after** the indirect-pointer scrub, so a host pointer cannot be retained in a log.

## 23. ★★★★★ THE MANDATORY KNOWN-POSITIVE FIRED ON BOTH ARMS

An `RM_CONTROL` carrying a command that does not exist, issued **before any row it will judge**:

```
★ R33 IN-BAND CAL = KNOWN-POSITIVE FIRED — cmd 0x20800ffe returned `errno == 0` from the
  syscall AND a non-zero RM status in the parameter struct ([Refused(86)])   ← 0x56
```

⇒ The reader **can** see a silent refusal. Without this, *"no divergence"* is exactly what a
blind reader prints — the `GET_PTE_INFO` lesson, one week, twice.

## 24. THE RESULT — **NO DIVERGENCE**

| | native | guest |
|---|---|---|
| total / failed | `87 / 0` | `83 / 0` |
| **in-band refusals** | **2** | **2** |
| by identity | `[8] RM_CONTROL 0x56` (the calibration) · `[38] RM_MAP_MEMORY_DMA 0x51` | **identical — same indices, same statuses** |

★★★ **Indices 1..64 are IDENTICAL on both arms in `(nr, name, size, errno, RM-STATUS)`.** The
only in-band refusals are the **two the program deliberately provokes**: the calibration itself,
and arm 2's `OCCUPIED` probe (`0x51` = `NV_ERR_NO_MEMORY`, which *is* arm 2's pass condition).

⊘ **`failed=0` vs 2 refusals is the census's own blind spot, measured for the first time.**

## 25. WHERE IT ACTUALLY DIVERGES — **INDEX 65, AND IT IS OUR OWN BRANCH**

```
[64] NAT nr 78 RM_MAP_MEMORY  ok  RM ok   |  GST nr 78 RM_MAP_MEMORY  ok  RM ok
[65] NAT nr 78 RM_MAP_MEMORY  ok  RM ok   |  GST nr 41 RM_FREE        ok  RM ok   <<<
```

The guest starts **tearing down** where native continues. That is `probe_guest_reachability`'s
`ControlFailed` early return — the program taking a different branch **because the control copy
did not land**. ⇒ **a consequence of the wall, not a cause**, and the sequence-length difference
(87 vs 83) is entirely this.

## 26. ⇒ **OUTCOME B. THE PUBLICATION STORY SURVIVES, WITH A MEASURED FLOOR UNDER IT**

★★★ **Arm 1's ioctls all lie inside the identical `1..64` prefix, and every one was served — by
RM's own status field, on both arms.** Arm 1's copy still fails in the guest.

⇒ **The failure is not caused by any refused, missing, extra or reordered ioctl. It happens
after every ioctl has succeeded — at the doorbell**, where the host CE faults at
`0x1_20000000`. The RM control plane is **not** where the guest and host differ.

⊘ The assumption the coordinator flagged is now **removed rather than confirmed by hope**: the
arms *are* ioctl-equivalent, and the `NOT-IN-GUEST-RAM [Vidmem]` finding is not a symptom of a
silently-refused call.

⚠ Scope: this says the **ioctls** agree. It says nothing about the GSP RPCs underneath them —
a guest ioctl is served by our emulated GSP, and RM answering `NV_OK` to the guest is not the
same as our having done the work. That is the next unmeasured layer, and it is **not** claimed
here.

**Vidmem publication remains STOPPED pending the owner's ruling.**

---

# ADDENDUM 6 — **Q1: NOTHING NEEDS PROMOTING. THE JOIN ALREADY EXISTS AND MY BOOTS HAD IT OFF.**

## 27. THE REFUSAL IS KEYED ON THE **DECLARATION**, NOT THE BACKING — cited

`crates/kayfabe-mmu/src/lib.rs:741-746`:

```rust
pub fn is_guest_ram(&self) -> bool {
    matches!(self.aperture, Aperture::SysmemCoherent | Aperture::SysmemNonCoherent)
}
```

⇒ It asks **what the guest declared the aperture to be**. It never asks whether a host object
exists behind the page. The refusal at `shim.rs` (`Ok((b, _)) if !b.is_guest_ram()` → the
`NOT-IN-GUEST-RAM` arm) therefore fires on **every** Vidmem operand, real-backed or not.

★ **This is a policy keyed on a declaration, exactly as the owner suspected.**

## 28. ★★★★★ AND THE MECHANISM IS ALREADY BUILT — `w282`'s ARM, WHICH I NEVER ARMED

`shim.rs:3345-3352` — the **sixth** selector:

> *"**w282's arm** — whether a CE operand page that lands in the emulated framebuffer has its
> leaf **JOINED**, so the executor stays `HostCe`. See [`OPERAND_JOIN_ENV`] and
> [`SharedDoorbell::join_operand_fb_leaves`]. … the pin and the join serve **disjoint operand
> populations (guest RAM vs framebuffer)** and a boot must be able to arm either alone."*

- `OPERAND_JOIN_ENV` = **`KAYFABE_OPERAND_JOIN`** (`shim.rs:13479`), arms `off` / `assert` / `join`
  (`:13496-13507`), **default `off`**.
- `join_operand_fb_leaves` (`shim.rs:8485`) joins every framebuffer leaf a CE operand names.
- **w282 measured it working:** *"THE HOST COPY ENGINE MOVED THE GUEST'S BYTES, AND THE GUEST
  READ THEM BACK."*

⊘⊘ **EVERY BOOT ON THIS RUNG RAN WITH IT `off`.** My runner arms
`FB_JOIN / GUEST_RING / GUEST_PUSHBUF / GUEST_SEMA / GUEST_OPERAND / GR_ROUTE / PT_WITNESS` and
**never sets `KAYFABE_OPERAND_JOIN` at all** — it is not in the carried-arm list I inherited, and
I checked the six carried arms rather than the seven that exist.

⇒ ★★★ **`KAYFABE_GUEST_OPERAND=pin` is the guest-RAM arm and it is the WRONG ONE for these
operands.** The doc says so in as many words — *disjoint populations* — and I armed the pin,
saw `NOT-IN-GUEST-RAM`, and read it as a missing mechanism. **The mechanism was one env var away.**
⚠ Thirty-two consecutive lanes have found their premise already built. This is thirty-three.

## 29. ⇒ THE ANSWER TO THE OWNER'S QUESTION

> *"do you think the promotion path is still needed then?"*

**No — and it is not a close call.**

- The operands' backing does not need to be *made* real; the FB **leaf-join** path already
  exists to make a framebuffer operand reachable by the host CE, and it is proven.
- The refusal blocking them is keyed on a **declared aperture**, not on a fact about backing.
- ⇒ The owner's decision is **not** *"should we copy guest device memory"*. It is *"should the
  armed-by-default set include the operand join"* — and it may not need a ruling at all, since
  the arm is already shipped, already tested, and already had a green rung.

⊘ **Still not claimed:** that arming it makes the fault go away. The sweep arm taught exactly
this lesson — a necessary step that changed no Xid. Pre-registered below.

## 30. PRE-REGISTERED — one variable, zero code change

`KAYFABE_OPERAND_JOIN=join`, everything else carried from the sweep arm.

| outcome | reading |
|---|---|
| `OPERAND-JOIN` lines appear, operands stop being refused, arm 1's copy completes | **the fix is an arming/default change** — no promotion, no publication design |
| the join runs and the Xid is unchanged | the join is **necessary-not-sufficient too**, and the wall is past the operand plane |
| the join refuses these leaves by name | **the refusal's own name is the next finding**, and it will be a fact about the backing rather than the declaration |

⚠ I am not predicting a pass. ⊘ And the `assert` arm exists precisely so the classification can
be read **without** joining anything — if `join` is ambiguous, `assert` is the control.

---

# ADDENDUM 7 — THE JOIN ARM: **IT JOINS, THEN QEMU ABORTS ON OUR OWN LOCK ASSERT. ARM IS VOID.**

Boot `w289j` @ `b66aa11`, `KAYFABE_OPERAND_JOIN=join`, one variable.

## 31. ★★★ THE ARM RAN AND THE JOIN WORKED

```
3 × OPERAND-JOIN token=0x00000003 arm=join            ← the arm reports its own execution
OPERAND-JOIN-TABLE: 2 asked, 0 MISS, 0 in guest RAM, 0 ALREADY JOINED,
                    2 CANDIDATE(S) in the emulated framebuffer
                    [va=0x120000000:Vidmem@0x10000/FakeFramebuffer
                     va=0x120010000:Vidmem@0x20000/FakeFramebuffer]
CE-OPERAND(fb_phys=0x10000) leaf va=0x120000000 → JOINED (shared) memory=0xcafe000c
    host_va=0x120000000 placed_as_asked=true established=4096 bytes, 4092 NON-ZERO
CE-OPERAND(fb_phys=0x20000) leaf va=0x120010000 → JOINED (shared) memory=0xcafe000d
    host_va=0x120010000 placed_as_asked=true established=4096 bytes, 3072 NON-ZERO
```

★ **Both faulting operands were joined, at IDENTICAL host VAs (`placed_as_asked=true`), with
their content present.** The mechanism does exactly what Q1 predicted.

⊘ **And it settles Q1's backing question precisely:** the operands are
`Vidmem@…/**FakeFramebuffer**` — the raw backing **is** fabricated. But the remedy is **not** a
promotion/copy-and-swap: the existing join establishes a **real shared host object at the same
VA**. ⇒ *"Is promotion needed?"* — **no**: the thing promotion would build already exists as
`join`, is shipped, and fires.

## 32. ⊘⊘⊘ AND THEN QEMU ABORTED — THE ARM IS VOID FOR THE FAULT QUESTION

```
thread '<unnamed>' panicked at crates/kayfabe-util/src/lockwitness.rs:152:5:
  R1 no-blocking-under-lock violation (l1_concurrency.md §3.3): munmap (dropping a host
  mapping) while holding rank(s) [0]
panic in a function that cannot unwind → thread caused non-unwinding panic. aborting.
  … kvm_cpu_exec → address_space_write → flatview_write → (our MMIO write handler)
```

⇒ **Our own R1 witness caught the join path calling `munmap` under a rank-0 lock.** The panic is
in an `extern "C"` callback, so it cannot unwind and **aborts the process**.

⚠ **This is guest-reachable**: the trigger is the guest's own MMIO store. A guest-driven abort of
the VMM is a hostile-guest concern in its own right, independent of this rung.

## 33. ⊘ EVERY DOWNSTREAM NUMBER ON THIS BOOT IS VACUOUS — stated, not glossed

The guest went unreachable (`ssh: … No route to host`), `boot_capture.sh:286` recorded
`Aborted (core dumped)`, **the R33 client never ran**, and:

```
run_w289j_hostdmesg.log = 0 bytes      HOST_DMESG_XID=0
```

⊘⊘ **`HOST_DMESG_XID=0` IS NOT "THE FAULT IS GONE."** It is *"the guest died before the client
could provoke anything."* Zero bytes is a state that needs its own check — and it is the exact
trap this campaign has banked twice. ⚠ Note also `hook finished: rc=0`: **the hook reported
success while producing nothing**, the same silent-no-op class as the `finish 0` fixed earlier
in this rung.

⇒ **Pre-registered outcome: none of the three.** The arm neither passed, nor left the Xid
unchanged, nor refused by name — **it crashed before the question could be asked.** Recorded as
VOID.

## 34. WHERE THIS LEAVES IT

- **Q1 is answered and does not need a boot:** no promotion path. The refusal is keyed on a
  declared aperture (`is_guest_ram`, `kayfabe-mmu/src/lib.rs:741-746`); the join that makes an FB
  operand real already exists (`KAYFABE_OPERAND_JOIN`, `shim.rs:13479`), and it demonstrably
  joins these two operands at identical VAs.
- **The next blocker is ours and it is small and specific:** the join path must not `munmap`
  under a ranked lock. `l1_concurrency.md` §3.3 already prescribes the fix — *"drop every guard,
  round-trip on the checked-out worker, then re-acquire and RE-VALIDATE (R5)"*. ⊘ Not attempted
  here: it is a concurrency change in the data plane's locking, and this rung stops at reporting.
- **Q2 (does the client spin on the semaphore in the guest?) is NOT done** — it needs the named
  waited-vs-never-waited exits, and the boot that would have measured it aborted.

---

# ADDENDUM 8 — ⚠⚠ **THE DELIBERATELY-RELAXED DIAGNOSTIC ARM** (owner-authorised)

> ⊘⊘⊘ **NOTHING IN THIS SECTION IS A SHIPPING CONFIGURATION OR A MILESTONE.** It is an
> instrument whose only purpose is to find out **whether anything else is broken beyond the two
> named layers.** A green here is *"no further discovery"*, not *"it works"*.

## 35. THE RELAXATIONS, NAMED

| # | relaxation | why it is not a default |
|---|---|---|
| 1 | `KAYFABE_PT_SWEEP=on` | measured to clear the table (`rows=1→3`, operand `MISS` 2+2→0+0). Off by default; the bootstrap gap it papers over is the real fix. |
| 2 | `KAYFABE_OPERAND_JOIN=join` | `w282`'s arm. Joins an emulated-FB CE operand leaf to a real shared host object at the identical VA. Off by default. |

⊘ **Q1 is answered by measurement, and it is the second case:** the operands are
`Vidmem@…/**FakeFramebuffer**` (`w289j` `OPERAND-JOIN-TABLE`) — the raw backing **is** fabricated.
**But no promotion path was written**: the "simplest thing that makes it reachable" already
exists as `join`, and `w289j` showed it establishing both operands at
`host_va == leaf.va` (`placed_as_asked=true`) with content carried.
★ **Known-positive for the backing query:** the same classifier reports the **ring** as
`JOINED`/reachable, and hardware independently proved the ring real by advancing `GP_GET`.

## 36. ⊘ AND A THIRD THING HAD TO BE FIXED FIRST — it was not optional

`w289j` **aborted the VMM** (`R1: munmap while holding rank(s) [0]`, non-unwinding panic on the
guest's own MMIO path). Fixed at `18f9f02` — `install_join` hands the region back so `join_fb`
unmaps **after** releasing the lock. ⚠ Guest-reachable, so this is a real defect, not a
workaround; it is the only thing outside the two relaxations that this rung changed.

## 37. PRE-REGISTERED, BEFORE THE RUN

Two boots, **one binary**, `RELAXED` vs `CONTROL` (both relaxations off) so the delta is
attributable.

**Raw client (82 ioctls):** does the copy complete — bytes moved, semaphore == declared payload,
`GP_GET` caught `GP_PUT`? ⚠ Read `met_the_whole_bar()`, never `copied()`.

| outcome | reading |
|---|---|
| **crosses** | everything remaining is **hardening a known-working path**; re-tighten the relaxations one at a time against a green target. ⊘ Report it as a RELAXED green, naming both relaxations, never as the milestone. |
| **does not cross** | **there is a further wall and we know it tonight.** Name it by identity — Xid, address, engine, access type. Worth as much as a pass. |
| CONTROL also crosses | the delta is not the relaxations; **the run is void for attribution** and the binary/arming is the suspect. |

⊘ **`Crit1State` and the in-band ioctl reader both run on every arm**, so a vacuous pass has two
independent named exits to fall through. ⚠ **`failed=0` is not "nothing refused"** — the in-band
verdict is printed beside it.

---

# ADDENDUM 9 — ★★★★★ **THE COPY CROSSES IN THE GUEST** (relaxed arm) — and the control proves it is the relaxations

> ⚠⚠ **THIS IS A RELAXED-ARM RESULT AND IT IS NOT THE MILESTONE.** Two off-by-default
> relaxations were on. It says *"no further discovery on this path"*, not *"it works"*.

Boots `w289j` (RELAXED) and `w289c` (CONTROL), **one binary**, `rev 277f03f`, back to back.

## 38. THE BAR — ALL FOUR FACTS, IN THE GUEST

```
RELAXED:
★ R33 arm 1 COPY = 4096 bytes moved: dst[0] 0x3f0011cc -> 0xc0ffee33,
    dst[last] 0xc0fff232 (want 0xc0fff232), engine semaphore 0x00000001 (declared 0x00000001),
    GP_GET 1 caught GP_PUT 1 — read back through an INDEPENDENT mapping

CONTROL (same binary, both relaxations off):
FAIL R33 arm 1 COPY = dst[0] 0x3f0011cc -> 0x3f0011cc (want 0xc0ffee33), … semaphore 0x00000000
    (want 0x00000001), GP_GET 1 GP_PUT 1 — the entry WAS fetched and the methods did nothing
```

★ **`met_the_whole_bar()`, not `copied()`** — bytes moved, **the LAST word correct** (so not a
truncated copy), the **semaphore carrying the DECLARED payload at the DECLARED address**, and
**`GP_GET` caught `GP_PUT`**. Read back through a mapping that is not the one written.

⇒ **The guest's raw CE client moved device memory end-to-end for the first time**, with no
libcuda in the process.

## 39. THE DELTA IS ATTRIBUTABLE, AND THE FAULT IS GONE BY IDENTITY

| | RELAXED | CONTROL |
|---|---|---|
| arming in force | `OPERAND-JOIN arm=join`, `PT-SWEEP ran=3` | `OPERAND-JOIN arm=off`, no sweep |
| arm 1 COPY | **★ all four facts** | FAIL, semaphore `0x0` |
| host `Xid` count | **1** | **2** |
| `Xid` addresses | `0x7_00100000` only | `0x1_20000000` **and** `0x7_00100000` |

★★★ **`0x1_20000000` — the fault this whole rung has chased across five boots — IS GONE**, and
the arm that removes it is named. The surviving `0x7_00100000` is **arm 4's own control operand**,
in the **third** VAS arm 4 allocates; the join fired on token `0x3` (arm 1's channel) and arm 4's
channel is a different token. ⊘ Same defect, different channel — not a new wall.

## 40. ⊘ WHAT STILL DOES NOT PASS, AND WHY — no vacuous green

- **`R33_RC=1`** and `FAIL R33 raw CE client` — because **arm 4** still fails (its operands were
  not joined) and **arm 6** still fails calibration. The client's own verdict is honest.
- **`CRIT1 STATE = CONTROL-NEVER-LANDED`** on **both** arms ⇒ **criterion 1's address half is
  still UNMET**, and every VA-identity number is still vacuous **by name**. The blocker is now
  precisely located: arm 4's control operands at `0x7_001…` are the un-joined ones.
- **In-band verdict: 2 refused on both arms**, the same two deliberate ones. **No ioctl diverged.**
- ⊘ The `arm 5 NOTIFIER` firing is still attributable to the **control's** failure, not to a
  deliberate fault — `?? arm 5 CONTROL` says so on the same run.

## 41. ⇒ THE READING, STATED AS THE OWNER ASKED

**It crosses.** ⇒ On the CE data plane there is **no further discovery** between the doorbell and
a completed, semaphore-signalled copy: ring fetch, method decode, operand resolution, engine
execution, completion write and cursor advance all work **once the operand is reachable and the
table is complete.** What remains on this path is **hardening a known-working path** — turning
the two relaxations into the real fixes:

1. the **bootstrap gap** (attribute page-table pages from the VAS root, not only from the witness
   set) so `PT_SWEEP` is not needed;
2. the **operand join** applied to every CE channel's operands rather than the one token, and
   armed by policy rather than by env var.

⊘ **Neither is written here.** ⚠ And the relaxed green must not be quoted without both
relaxations named beside it.

---

# ADDENDUM 10 — ⊘ **CUP2 WAS NOT RUN THIS RUNG. Saying so before running it.**

The brief pre-registered `CUP2_RC` and it is **the owner's goal metric**. My report carried no
number **because no cup2 boot happened on this rung** — every boot was the 82-ioctl raw CE
client. ⊘ It was **not run and omitted**; it was **not run at all**.

⇒ *"It crosses"* in §38-41 means **the CE data plane crossed**. It says **nothing** about cup2,
and must not be read as saying anything about it.

## 42. PRE-REGISTERED, BEFORE THE BOOT

Same two relaxations (`KAYFABE_PT_SWEEP=on`, `KAYFABE_OPERAND_JOIN=join`), same carried arming,
`cup2_hook_gdbspin.sh`. Baseline **`CUP2_RC=1`** (`rev aea02a52`; it was **124** before the
notifier work turned the harness timeout into cup2's own exit code).

| outcome | reading |
|---|---|
| **`CUP2_RC=0`** *and* the matmul verifies (`bad=0`) | cup2 crosses **on the relaxed arm**. ⊘ Still not the milestone — both relaxations named beside it, always. |
| **`CUP2_RC` moves off 1** (e.g. to 124, or a new code) | the wall **moved**; report the new last print and the new fault identity. A different failure is a result. |
| **`CUP2_RC=1` unchanged** | the CE fix does **not** reach cup2's path ⇒ **a further wall exists beyond the CE plane**, and the hardening list is premature. ★ This is the outcome that most changes what to do next. |
| no `CUP2_RC` line at all | **the measurement did not happen** — printed explicitly as *"NOT 0"*, never inferred. |

⊘ **The last print is captured either way** (`ok|FAIL|cu*|totalMem|bad=|maxerr` tail), so *"where
it got to"* is answerable without a second boot. ⚠ `^CUP2_RC=` anchored **with** the unanchored
contrast printed beside it, and `GCC_CUP2_RC` counted separately — unanchored has printed
`[CUP2_RC=0 CUP2_RC=1]` on two consecutive rungs and would report the headline success value on
a failing arm.

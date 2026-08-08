# The replay audit — every value this port serves the guest, graded

> *"let's hope our project does not become replay tables."* — the owner

That worry is testable, so this file tests it rather than reassuring anyone about it. Every
constant, table and magic value the port can put in front of a guest NVIDIA driver was
classified into three levels, the levels were counted, and the count is stated with its
denominator.

**Read §1 for the number, §5 for the ranked list of things that are wrong-shaped, §6 for what
would fix each one, and §7 for the four sites that need an edit today.**

---

## 0. The grading rule, exactly as applied

| level | meaning | what a row must carry |
|---|---|---|
| **A — derived** | an `ogkm`/`nvproxy`/spec citation that **consumes or produces** the value, and our value follows from it | a `file:line` citation that explains *why that value* |
| **B — measured** | read from real hardware or a named capture, **and the consuming RM code path is named** | capture/run reference **plus** the consumer — the model row being `0x20802a08` → 20480, RTX 3060 / open 580.159.04, 2026-08-01 |
| **C — REPLAY** | captured or copied with **no explanation of why that value** | nothing. This is the category under audit |
| **P — port policy** | a bound *this port* chose, declared as ours, never as a claim about silicon | a written rationale; not a hardware claim at all |

⊘ **B and C are separated only by whether the consumer is named.** *"The real GA106 answered
20480"* is B only when it is followed by *"and RM DMAs CE fault records into a buffer of
exactly that size"* (`kceGetFaultMethodBufferSize_IMPL` → `memdescCreate`,
`ogkm-580: kernel_ce.c:846`, `mem_desc.c:239-241`). Without the second half it is C.

**P was added during the audit and it is not a way of avoiding C.** A value is P only if the
site says it is *ours* — `MAX_PT_LEARNED`, `MAX_PUSH_SPANS`, `SPARSE_FB_RESIDENT_CAP`. A
number copied off a capture and presented as the chip's is C no matter how it is spelled.

### What counts as "served"

A value the guest can observe: a control reply body field, an MMIO read result, a PCI
config-space field, a table row the guest walks, a page-table or doorbell encoding that
determines bytes in guest-visible memory.

⊘ **Wire-layout offsets and `sizeof`s are excluded** — they are decode-side, they are pinned
by `const { assert!(offset_of!(…)) }` against rustc, and grading them would inflate the A
column with values that are not answers to anything. Roughly **300** were seen and skipped.
Host-facing values (`bringup.rs`, `invariant_classes.rs`, `host_classes.rs` selection) are
noted where they appear but are not the guest's.

---

## 1. The verdict

**1 496 served values.** Method and denominator honesty in §2.

| level | count | share |
|---|---|---|
| **A — derived** | ~937 | 62.6 % |
| **B — hardware-read (RTX 3060 / 580.159.04, 2026-08-01) with a named consumer** | 42 | 2.8 % |
| **C — REPLAY** | **477** | **31.9 %** |
| **P — declared port policy** | ~40 | 2.7 % |

★★★ **The headline is not the 32 %. It is that 406 of the 477 — 85 % — are three tables.**

| table | served scalars | what it is |
|---|---|---|
| GR geometry (`grstatic.rs` + `grinfo.rs`) | 167 | TPC/PES/SM interleave, litter constants, 26 context-buffer sizes |
| `GA106_INTR_TABLE` + `GA106_INTR_SUBTREE_MAP` (`kayfabe-device/src/ga10x.rs:783`) | 103 | 24 rows × 4 fields, verbatim from one captured reply |
| `GA106_ENGINES` + `GA106_DEVICE_INFO` (`ga10x.rs:616`, `:1349`) | 136 | 6 rows × 22 wire fields plus 6 PRI bases |

**Strip those three and the rest of the port is 1 090 values at 86 % A, 6.5 % C.**

⇒ The honest answer to the owner is: **we did not become replay tables. We have three of
them, they are named, and they are the entire problem.** The other ninety-something percent of
the surface carries a citation into NVIDIA's own source that says what the value is for.

★ And the port already demonstrates it knows the difference, twice, in the direction that
costs work: `GA106_USER_REGISTER_ACCESS_MAP` (`ga10x.rs:1226`) **refuses** the oracle's
captured 6 809-range map and serves RM's own "not published for this chip" answer instead
(`ogkm-580: gpu_register_access_map.c:261-267`), and `GA106_CONSTRUCTED_FALCONS` (`:1270`)
**overrules** the oracle's eight falcons with `NONE`. Neither is a replay that got lucky;
both are captures that were read, argued against and discarded.

---

## 2. ⚠ The denominator, honestly

⊘ **I cannot claim to have enumerated every served scalar in the workspace.** What I can
claim is enumeration **by surface**, where the surfaces were chosen to cover every crate that
can put a byte on a guest-visible wire, and where each surface carries its own count.

- **Read in full:** all of `kayfabe-abi/src/*.rs`, `kayfabe-device/src/*.rs`,
  `kayfabe-chips/src/*.rs`, `kayfabe-gsp/src/`, `kayfabe-arch/src/`, `kayfabe-rmrpc/src/`,
  `kayfabe-fwd/src/`, `kayfabe-core/src/`, `kayfabe-mmu/src/`, `kayfabe-completion/src/`,
  `kayfabe-qemu-raw/src/`, `kayfabe-vmm*/src/`, `kayfabe-shell/src/`,
  `qemu/hw/misc/nvkvm/nvkvm.c`.
- **Sampled, with the rule stated:** `capability.rs` — 281 rows. Every non-`Nvproxy` row (42)
  read individually; of the 239 `Origin::Nvproxy` permit rows, ~90 read individually and the
  remainder spot-read. They are uniform one-line `{cmd, name, origin}` triples with **no
  per-row citation**, resting on a block-level "we ported nvproxy's table" claim. Graded A at
  the block, and §5 says why that grade is thinner than it reads.
- **Counted as a class:** `crates/kayfabe-abi/src/generated/` — 101 value constants, each
  emitted with an `ogkm <header>` provenance line by a generator whose requests carry **no
  value field** (`gen/src/main.rs`; every scan miss is a hard error). Graded A, 101/101.
- **Granularity:** counts are per *scalar the guest can read*, so a 24-row × 4-field table is
  96, not 1. Big tables therefore dominate the denominator, which is the point — a wrong
  scalar in row 17 is not less wrong for having 23 siblings.
- **Double counting resolved by:** grading at the *port* level, counting the value once at
  its **declaration** site. Several chip facts live twice — the row in
  `kayfabe-device/src/ga10x.rs`, the encoder and the argument in `kayfabe-abi/src/<x>.rs`.
  Those are graded where the consumer is named (the `abi` module) and counted once.

★ A shortened list is a smaller true statement, not a better score. If the real population is
1 700 rather than 1 496, the three-table concentration does not move, because the missing
values would be in surfaces that are already ~90 % A.

---

## 3. The table, by surface

Per-surface denominators. `Σ` is that surface's served-scalar count.

| # | surface | file(s) | Σ | A | B | C | P |
|---|---|---|---|---|---|---|---|
| 1 | GR static/info geometry | `abi/grstatic.rs`, `abi/grinfo.rs` | 178 | 5 | 6 | **167** | 0 |
| 2 | Kernel interrupt table + subtree map | `device/ga10x.rs:783,:938` | 103 | 0 | 0 | **103** | 0 |
| 3 | Engine + device-info table | `device/ga10x.rs:616,:1349` | 138 | 2 | 0 | **136** | 0 |
| 4 | Static-info scalar rows | `abi/{chipinfo,deviceinfo,gspstaticinfo,bifstatic,fmbsize,gmmustatic,memsysconfig,falconinfo}.rs` | 61 | 35 | 10 | 16 | 0 |
| 5 | Other control-reply modules | `abi/{l2evict,faultbuffer,fifochannels,confcompute,guestsysinfo,gvaspacepdes,regaccessmap,notifier,rc,eventnotify}.rs` | 29 | 27 | 0 | 2 | 0 |
| 6 | Synthetic VBIOS / ROM | `abi/vbios.rs` (+ `generated/vbios.rs`) | 79 | 62 | 2 | 15 | 0 |
| 7 | PCI identity, BARs, apertures, MSI-X | `abi/pcibars.rs`, `device/ga10x.rs`, `qemu/hw/misc/nvkvm/nvkvm.c` | 55 | 43 | 7 | 5 | 0 |
| 8 | Capability / allowlist tables | `abi/capability.rs` | 281 | 264 | 1 | 16 | 0 |
| 9 | GSP boot regs, LibOS args, msgq/RPC protocol | `device/ga10x.rs`, `gsp/src/`, `arch/src/` | 59 | 54 | 0 | 5 | 0 |
| 10 | GMMU format, doorbell, USERD, pushbuffer | `chips/ga10x.rs` | 46 | 45 | 1 | 0 | 0 |
| 11 | Non-GA106 arch profiles | `chips/{ad10x,gh100,host_classes}.rs` | 90 | 84 | 0 | 6 | 0 |
| 12 | Generated ABI constants | `abi/src/generated/` | 101 | 101 | 0 | 0 | 0 |
| 13 | Version table, submit methods, envelope, statuses | `abi/{versions,submit,view,wire,lib}.rs` | 181 | 171 | 8 | 0 | 2 |
| 14 | Register plane, FB window, BAR2, doorbell, CPU intr | `device/{plane,fbwin,bar2,doorbell,cpuintr,nonstall,guestsysinfo,sticky}.rs` | 61 | 47 | 5 | 4 | 5 |
| 15 | RPC policy, shim, forwarding, core | `rmrpc/`, `qemu-raw/`, `fwd/`, `core/` | 34 | 15 | 2 | 2 | 15 |
| | **total** | | **1 496** | **937** | **42** | **477** | **40** |

### Surfaces that serve nothing, and say so

`kayfabe-mmu` has **no PTE/PDE encoder at all** — it is a decode seam, so no page-table byte
this port authors exists. `kayfabe-completion` observes fence values and never produces one.
`kayfabe-device/src/{census,faultbuffer,gvaspub}.rs` return `None` on every path;
`the_census_changes_no_byte_of_any_reply` pins it. `abi/oracle.rs` serves nothing — it is the
refusal predicate described in §4.

### The strongest-evidenced blocks in the tree, for calibration

- **The GMMU format** (`chips/ga10x.rs:500-830`). Level shifts, the two *different* aperture
  tables, page sizes and the PD1-is-a-512 MiB-leaf trap are each cited to the ogkm file that
  *binds* the enum to the value (`kern_gmmu_fmt_gm10x.c:165-182` vs `:184-201`), and then
  differentialled against NVIDIA's own compiled `gmmuFieldGetAperture` by
  `tests/tests/gmmu_fmt_oracle.rs`, which carries a test literally named
  `the_pde_and_pte_aperture_tables_are_not_the_same_table`. The recorded past bug — two
  encodings agreeing on exactly the first values anyone spot-checks — cannot recur in this
  shape: `kayfabe_arch::Aperture` is decode-only vocabulary whose declaration order is
  explicitly not ABI, and nothing casts it.
- **The doorbell token** (`chips/ga10x.rs:436`). Cited to
  `kfifoGenerateWorkSubmitTokenHal_GA100` **and** measured against real GA106 tokens whose
  expected chids came from `FIFO_GET_ALLOCATED_CHANNELS` rather than through this function or
  its inverse. Two independent instruments, both directions, consumer named.
- **The synthetic VBIOS** (`abi/vbios.rs`). No real firmware is embedded anywhere in the
  workspace; the image is built to satisfy inequalities the driver's own parser states, and
  `tests/tests/vbios_real_parser_oracle.rs` compiles NVIDIA's unmodified
  `kgspExtractVbiosFromRom_TU102` chain and feeds the image through it. That turns "we read
  the C right" into "the C itself accepts it".

---

## 4. The refusal that already exists, and its limit

`kayfabe_abi::oracle::captured_row_evidence` encodes the lesson the C oracle's empty rows
taught, as a **predicate on `(psize, dlen)` rather than a list of ids**:

- `dlen > psize` → `KeptMoreThanExists`
- `psize == 0` → `NoBodyExists` — the only checkable "empty"
- `psize > 0 && dlen == 0` → `BodyNeverCaptured` — ⊘ **never** `vec![0; psize]`
- `0 < dlen < psize` → `BodyTruncated`, with per-*field* capture checks

This is the right instrument and it is quantified over a predicate, not a list. ⚠ **Its limit
is that it grades the row, not the citation.** A module doc can still argue *from* an empty
row in prose, and §7 shows four sites that do — three of them in `kayfabe-abi` itself, where
`oracle.rs`'s own table already records the contradicting hardware bytes.

★★ **The truncated class is larger than the empty one and gets far less attention.** 16 of 56
C rows (28.6 %) are `0 < dlen < psize`, against 11 empty. The comfortable hypothesis — that
the capture merely trimmed trailing zeros — is refuted from the header's own bytes: all 16
kept-prefixes *end in zero bytes*, `0x20800a40` in 15 833 of them. No trimmer can emit that.

---

## 5. ★ The C list, ranked by blast radius

Ranked by what breaks if the value is wrong, not by how many values there are. A wrong DMA
size, address or bound outranks a wrong string every time — that is the whole lesson of
`0x20802a08` decoding from an empty row as size 0 when a real GA106 answers 20480, into a
buffer a hardware writer DMAs into.

### ⓵ `GA106_CONTEXT_BUFFERS` — 26 sizes, no consumer named
`abi/grstatic.rs:790-817`. `0x000a_9700` for GRAPHICS, `0x0070_0000` for COMPUTE_PREEMPT,
`0x0085_1200` for GRAPHICS_ATTRIBUTE_CB, and twenty-three more. The module says outright that
these are *"not a function of the geometry — the sizes are the chip's own context-buffer
table"*, so nothing derives them and nothing consumes them in writing.

**If wrong:** RM `memdescCreate`s a context buffer of exactly this size and the **GPU writes
into it**. Too small is a buffer overrun with a hardware writer — byte for byte the
`0x20802a08` failure mode, at 26× the surface. ★ Mitigating, and it is why this is ⓵ rather
than a fire: the source row (`ctl_20800a32`, psize 1664 = dlen 1664) is a **complete**
capture, not an empty one, and complete rows are the class that matched real GA106 byte for
byte on 2026-08-01. The defect is that nothing says so at the point of use.

### ⓶ `GA106_ENGINES` — capture noise served in an indexing field
`device/ga10x.rs:616-761`, 132 scalars. Two fields are uninitialised on real silicon and are
reproduced anyway; the file says so. `engineData[7]` carries `0x82300100`, `0x77f2058f`,
`0x018e0102` across unrelated entries, and `SOFTWARE.pbdma_fault_ids` is
`[0x82300100, 0x77f2058f]`.

**If wrong:** a PBDMA fault id is an **index** into the fault-id ranges
`kgmmuInitCeMmuFaultIdRange_GA100` computes (`ogkm-580: kern_gmmu_ga100.c:255-296`). Noise
landing in a live range attributes a fault to the wrong engine. The only thing keeping it out
today is that `kfifoGetHostDeviceInfoTable_KERNEL` special-cases the SOFTWARE row when
reserving fault ids (`kernel_fifo.c:2160-2166`) — a single narrow arm carrying the whole
argument for serving noise.

### ⓷ `GA106_INTR_TABLE` — 96 scalars, zero ogkm citations, zero per-row explanation
`device/ga10x.rs:783`. Byte-identical to one captured `0x20800a5c` reply
(`cap1b_coldboot_hermetic_d6` record 142056, `paramsSize=2112`, `tableLen=24`). Exactly one
row gets a sentence — `vectorStall = 0x9b` on `MC_ENGINE_IDX_GSP` is *"the vector this device
would raise for its own message-queue doorbell"* — and that is intent, not derivation. Nothing
explains `0x40`, `0x83`, `0x48`, `0x81`, the eight all-invalid rows, or the non-stall numbers.

**If wrong:** a completion arrives on a leaf the guest is not scanning ⇒ silent hang; or the
guest services the wrong engine. ★ Partly self-defending — this is the same table the device
raises from (`device/nonstall.rs:117`), so device and guest agree *by construction*, and the
CE2 → `0x07` binding is `[measured]` (2026-08-08, boots `cebind_p35` / `cup2_p35` at
`5a035e0`). The residue is anywhere RM holds its own expectation of a vector.

⚠ **And its one stated safety argument is somebody else's.** The `UVM_OWNED` subtree mask must
match the access-counter vector's subtree or `intrCacheIntrFields_TU102` asserts — the file
records that as *the C author's claim, never observed to fire from this port*. That is the
honest form, and it means the table's justification for not being trimmed is currently
unverified.

**⚠⚠ A concrete inconsistency, found by this audit:** `kayfabe-fwd`'s `COMPLETION_VECTOR` is
`IrqSpec::Msix(0)` (`fwd/src/lib.rs:103`, self-labelled *"abstract placeholder"*), while the
table this device publishes gives CE2's non-stall vector as `0x07`. Two descriptions of the
same fact, disagreeing, one of them declared a placeholder. Nothing has booted the path that
would notice.

### ⓸ `GA106_DEVICE_INFO` PRI bases
`device/ga10x.rs:1349` — GR0 `0x400000`, CE0-3 all `0x104000`, read out of the C's captured
`ctl_20800a40`. **If wrong:** RM writes engine registers at an offset in our BAR0 that we do
not claim, and the write lands in `plane.rs`'s unclaimed arm. Bounded — unclaimed accesses are
counted and sampled — but the guest sees a register that never took its value.

### ⓹ The GR geometry tables — 167 scalars
`grstatic.rs` (TPC↔PES map, 14 TPC rows, GPC rows, opaque 23-byte `GA106_GR_CAPS`) and
`grinfo.rs` (48 of 58 litter constants transcribed with only a name). **If wrong:** these size
the golden context and the VEID/subcontext arithmetic. ★ Four of `grinfo`'s 58 *are* B with
named consumers (`MAX_SUBCONTEXT_COUNT` → `kfifoGetMaxSubcontextFromGr_KERNEL`,
`LITTER_NUM_GPCS` → `kgrmgrGetLegacyTpcMask_IMPL`'s bound), and `grinfo`'s `index` word is
**regenerated from array position**, not replayed, so a stride or endianness error fails a
test. The remaining 48 are genuinely unexplained numbers.

### ⓺ MSI-X table/PBA layout — an unguarded invariant with a *wrong* stated derivation
`qemu/hw/misc/nvkvm/nvkvm.c:1953-1954`, `:2097-2099`. Table at `0x0`, PBA at `0x800`, neither
cited. The comment justifying the 4 KiB BAR reads *"one page holds a 256-entry vector table at
0x0 and its pending bits at 0x800"* — but 256 × 16 B is 4096 B, which swallows the PBA. The
bound supports at most **128** entries. `MSIX_VECTORS = 8` makes it harmless today; the
sentence that makes `4 KiB` look derived does not derive it, and nothing refuses
`msix_vectors * 16 > 0x800`. Note the asymmetry: `nvkvm_identity_realize` **does** refuse a
`bar0/1/2-size` that disagrees with the chip row, with an explicit *"two maps would disagree"*
argument. The same argument applies here and is not made.

### ⓻ `REGS_APERTURE_LEN = 16 MiB`, `MSIX_VECTORS = 8`
`device/ga10x.rs:562`, `:567`. Both sourced only to `C: src/qemu/nvkvm_gpu_emul.c`, the second
quoting the C author's reasoning verbatim with no consumer on this side. BAR0's length is an
address bound; it is cross-checked against the QEMU property at realize, which is what keeps
it out of the top three.

### ⓼ The security axis: 9 unnamed control ids + 7 `Empirical` allowlist rows
`abi/capability.rs:1293` (`RULE_COVERED_C_ROWS`) and the `Origin::Empirical` permits. The nine
have **no name in nvproxy's map and none in either vendored ogkm tree**, so none could be
reviewed; the file says so. The seven were observed on a real host ioctl stream and admitted
by the C, whose argument lives in the C's comments and not in this tree. **If wrong:** this is
not a correctness bug, it is a guest-reachable path to host RM. Different axis, and the only
one on this list where "wrong" means a boundary crossing rather than a hang.

★ In the same file, and larger than all sixteen: `RM_GSS_LEGACY_MASK` passes **every command
with bit 15 set** — 2³¹ words, no table row, no review. It is graded **A** and correctly so
(it is nvproxy's own rule, ported with citation), but the *consequence* is derived from
nothing, and no amount of per-row evidence in the other 281 rows covers it.

### ⓽ GSP-plane read values that the module header claims are derived
`device/ga10x.rs`: `DMATRFCMD_IDLE = 0x2` (**no citation of any kind**), `IRQSTAT_SWGEN0 =
1<<6` (C-only, served three times as IRQSTAT/IRQMASK/IRQDEST), `PMC_BOOT_0 = 0x1760_00A1` and
`PMC_BOOT_42 = 0x176A_1000` (C-only; the C's own comment cites `dev_boot`/`nv_ref.h` and that
chain was not carried across). These sit *inside* a region whose header says *"★★ Every
constant here is DERIVED, never read off the trace"* (`ga10x.rs:22`) — a load-bearing false
rationale of the exact kind this codebase flags elsewhere. **If wrong:** `PMC_BOOT_0` is the
chip id from which RM picks every HAL, so wrong means everything; empirically it is pinned by
every boot that has ever bound, which is why it ranks here and not at ⓵.

⚠ Adjacent and unranked because nothing reads it yet: **`PMC_ENABLE` has no row at all** and
falls through to the unclaimed arm, reading `0` — indistinguishable in the counters from a
genuinely unknown offset. A guest that gated on it would see every engine disabled.

### ⓾ Declared-invented values on the non-GA106 profiles
`chips/ad10x.rs:112,114`, `chips/gh100.rs:212,214` — `WPR2_LO_UP`/`WPR2_HI_UP`, each ★-marked
**INVENTED**; `gh100.rs:105` `GSP_RISCV_CPUCTL`, ★-marked **ASSUMED**; `ad10x.rs:105`
`SEC2_BOOTER_UNLOAD = 0xff`, sourced only to the C artifact. **If wrong:** an Ada or Hopper
boot fails FWSEC's *exact* compare — `kgspExecuteFwsec_TU102` compares `WPR2_ADDR_LO` against
its own `frtsOffset` arithmetic (`ogkm-580: kernel_gsp_frts_tu102.c:514-524`), which GA10x
honours by *computing* the value and these two do not. ⚠ Worse than the number: `ad10x.rs` and
`gh100.rs` still carry the *pre-correction* rationale ("only non-zero is load-bearing"), which
`kayfabe-arch`'s own `GspReg` docs have since retracted. Zero Ada or Hopper hardware has run.

### ⑪ Cosmetic and low-consequence C
`vbios_version 0x9418_0000` / `0x9518_0000`, `vbios_oem_version`, `ucode_id`,
`engine_id_mask`, `signature_versions`, BIT `BCD_Version` — invented and **declared** invented:
`abi/vbios.rs` says outright that these are not versions any board reports and that quoting
them back would be illegitimate, and **no card was measured** for either number. Worst case is a wrong
string in `nvidia-smi`. `GA106_FB_REGIONS[0].performance = 6` — self-declared unfalsifiable.
`deviceinfo`'s `groupId` / `ginTargetId` / `deviceBroadcastPriBase` / `groupLocalInstanceId` —
zero-valued, from the *truncated* `0x20800a40` row, and `ogkm-580: gpu_vgpu.c:313` only prints
`devicePriBase`. `FW_CARVE_OUT_BYTES = 0x1042_0000` — consumer reads only `!= 0`
(`ogkm-580: mem_mgr_gsp_client.c:96-100`), so the magnitude is unconsumed. `PT_DECODE_BUDGET = 300_000`
— the number is the C's, the behaviour is deliberately different (a loud
`WalkFault::BudgetExhausted` where the C silently stopped).

---

## 6. What would promote each C — the experiment or the citation, never the guess

⊘ **No value is guessed here.** Each row names the file to read or the experiment to run.

| C group | promote to | how |
|---|---|---|
| `GA106_CONTEXT_BUFFERS` (26 sizes) | **B** | Two probes on an RTX 3060 / open 580.159.04, the recipe already in `traces/real_ga106/README.md`: print `pKernelGraphics->ctxBuffersInfo` inside `kgraphicsInitializeDeferredStaticData` (`ogkm-580: kernel_graphics.c:2485`), and print the `memdescCreate` length at each context-buffer allocation site. Naming the second consumer is what converts the row; the first alone reproduces the capture. |
| `GA106_CONTEXT_BUFFERS`, cheaper first step | **A-partial** | Read `kgraphicsCreateGoldenImageChannel` / `kgrctxAllocCtxBuffers` and record, per `ENGINE_ID`, which sizes RM *derives* from GPC/TPC counts. Any buffer whose size is a function of the geometry stops being a table row. |
| `GA106_ENGINES` `engineData[7]` + `SOFTWARE.pbdma_fault_ids` | **A** | Read `kfifoGetHostDeviceInfoTable_KERNEL` (`ogkm-580: kernel_fifo.c:2117-2210`) and enumerate every slot it *reads*. Slots it never reads should be served as **zero with the citation**, not as reproduced noise. This is a source read; no hardware needed. |
| `GA106_INTR_TABLE` — the `engine_idx` column (24) | **A** | Already derivable: `MC_ENGINE_IDX_*` is `ogkm-580: engine_idx.h:54,73-74`, and `abi/inittables.rs:160,164` already imports two of them. Map all 24 and the column becomes checkable rather than copied. |
| `GA106_INTR_TABLE` — the 48 vector fields | **B** | An RTX 3060 with `intrInitInterruptTable_KERNEL` (`ogkm-580: intr.c`) probed to print each `(engineIdx, vectorStall, vectorNonStall)` *and* the site that consumes it — `_intrServiceNonStallLeaf_TU102` for non-stall, `intrCacheIntrFields_TU102` for the subtree constraint. The second half is the point: it would also settle the `UVM_OWNED` claim this port currently attributes to the C author. |
| `GA106_INTR_SUBTREE_MAP` (7) | **A** | Read `intrCacheIntrFields_TU102` (`ogkm-580: intr.c:1084-1085`) and `NV2080_INTR_CATEGORY_*` (`ctrl2080mc.h:337`) and state which categories the guest branches on. The C *synthesises* these seven words in code — that code is a derivation someone can transcribe with a citation. |
| `GA106_DEVICE_INFO` PRI bases (5) | **A** | `NV_PGRAPH` / `NV_PCE` base addresses are in the vendored `published/ampere/ga102/` headers. Cite the header and the capture becomes corroboration rather than the source. |
| GR geometry — 14 TPC rows, GPC rows, TPC↔PES map (112) | **A-partial then B** | First read `kgrmgr_ga100.c` and `kernel_graphics_manager.c` for every consumer of a per-GPC/per-TPC field and cite the ones that are read. Only what survives that needs an RTX 3060 probe at `kgraphicsInitializeDeferredStaticData`. |
| `grinfo` 48 litter constants | **A-partial** | Grep both ogkm trees for each `NV0080_CTRL_GR_INFO_INDEX_*` name and record which are read at all. The three with no name in `ctrl0080gr.h` should be served zero-with-a-refusal, not a copied number. |
| `GA106_GR_CAPS` (23 opaque bytes) | **A** | `ogkm-580`'s `NV0080_CTRL_GR_CAPS_TBL_*` bit definitions plus a grep for each bit's reader. Currently "deliberately not decomposed"; decomposing it is a source read. |
| MSI-X table/PBA offsets | **A** + a refusal | Cite the PCI Local Bus MSI-X capability layout for the table/PBA alignment rule, fix the arithmetic in the comment, and add a `const` assertion / realize-time refusal that `msix_vectors * 16 <= pba_offset`. Same shape as the `bar0-size` refusal three functions away. |
| `REGS_APERTURE_LEN = 16 MiB` | **B** | `lspci -v` on the RTX 3060 records region 0's length directly; one line in `traces/real_ga106/`. The consumer is already named (`kbusInitBarsSize_KERNEL` → `pciBarSizes[]`). |
| `MSIX_VECTORS = 8` | **A** | Read `nv_init_msix` in `kernel-open/nvidia/nv.c` for what the driver requests and what it does on a short grant. The number then follows from a requirement instead of from the C's comment. |
| 9 `RULE_COVERED_C_ROWS` + 7 `Empirical` permits | **A or DENY** | For each id, resolve `class << 16 \| cmd` against `ogkm-580`'s `ctrl*.h` and nvproxy's map. ⊘ Any id still unnamed after that should move to `DENIED_CONTROLS` with `DeniedBecause::Unreviewable` — an unnamed permit is the one row type that cannot be argued for. |
| `DMATRFCMD_IDLE`, `IRQSTAT_SWGEN0`, `PMC_BOOT_0/42` | **A** | All four are in vendored headers: `turing/tu102/dev_falcon_v4.h` for the first two, `dev_boot.h` / `nv_ref.h` for the boot registers (the C's own comment names them; the citation simply was not carried across). Pure source read. |
| `PMC_ENABLE` | **A** | Read `gpuStateInit` / `kmcStateInit` for whether RM reads it on the GSP-offload path at all. If it does not, add a row that says so; if it does, the value follows from what it tests. |
| `ad10x`/`gh100` `WPR2_LO_UP`/`_HI_UP` | **A** | Delete the constants and *compute* them, exactly as `device/ga10x.rs` does — `wpr2_reg(frts_offset())` from that profile's own FB size, against `kgspExecuteFwsec_TU102`'s arithmetic (`ogkm-580: kernel_gsp_frts_tu102.c:514-524`). Also retract the stale "only non-zero matters" rationale in both files. |
| `gh100::GSP_RISCV_CPUCTL` base | **A** | Resolve `NV_PGSP` under `published/hopper/gh100/`; if the base genuinely is not defined there, the honest form is a refusal, not an assumed base. |
| `ad10x::SEC2_BOOTER_UNLOAD = 0xff` | **A** | Find the Booter-unload command encoding in `ogkm-580: kernel_gsp_tu102.c`'s SEC2 sequence; if it is genuinely a local convention, say so at the declaration and stop citing the C for it. |
| `FALCON_REGISTER_BLOCK_LEN`, `MAX_CTX_BUFFER_SIZE`, `PRI_REGISTER_ALIGN` | **P** | These are bounds, not chip facts. Relabel them as this port's policy with the rationale they already carry, and stop presenting a capture-derived granularity as a hardware property. |
| `vbios_version`, `ucode_id`, `engine_id_mask`, `signature_versions`, `BCD_Version` | **P** | Already declared invented; three of them name **no consumer at all**. Grep the driver's VBIOS/FWSEC parser for each field and either name the reader or record that nothing reads it. Cheap, and it closes the one gap in an otherwise consumer-complete module. |
| `COMPLETION_VECTOR` | **A** | Take it from `chip.intr_table`'s non-stall vector for the raising engine — the route `device/nonstall.rs:117` already implements — instead of a constant. That deletes the value rather than promoting it, which is the better outcome. |

---

## 7. ⚠ Four sites to fix now — a falsified citation, still being quoted

These are not level-C values. They are **level-B-shaped claims resting on evidence this
repository has already refuted**, which is the exact failure the claim ledger's §0 was written
about: *a correction that lives downstream of the claim does not stop the claim being quoted.*

`kayfabe_abi::oracle`'s own table records, from an RTX 3060 on open driver 580.159.04 measured
2026-08-01 (`traces/real_ga106/rpc_bodies_real_ga106.txt`, lines 25-30 and 56-58):

| control | the C's row | real GA106 | verdict |
|---|---|---|---|
| `0x20800af3` | `psize 2, dlen 0` | `01 01` | **contradicted** |
| `0x20800aac` | `psize 4, dlen 0` | `00 00 01 00` | **contradicted** |

And yet:

1. **`crates/kayfabe-abi/src/confcompute.rs:29-33`** argues *"the capture trims trailing
   zeros, so a real RTX 3060's GSP answered this control `NV_OK` with **both bits clear**"* —
   from the `0x20800af3` empty row. Hardware says `01 01`: **both bits set**. The trailing-zero
   hypothesis is refuted independently in `oracle.rs` (all 16 truncated prefixes *end* in zero
   bytes). The **served** value (`false, false`) is still right and still level A — it rests on
   `ogkm-580: mapping_cpu.c:215-235`, where either bit set deletes RM's own refusal, and on
   this port serving no CPR plane. But the module claims corroboration it does not have and is
   in fact **knowingly divergent from measured silicon**, which is a fact the header should
   carry rather than contradict. `l2evict.rs` received exactly this correction; this file did
   not.
2. **`crates/kayfabe-abi/src/bifstatic.rs:25-36`** makes the same argument from `0x20800aac`
   and concludes *"all four bits clear, `bPcieGen4Capable` included."* Hardware says
   `00 00 01 00`; byte 2 is `DEVICE_MULTI_FUNCTION_OFF` (`bifstatic.rs:72`), so
   **`bIsDeviceMultiFunction = 1`** on the part the argument invokes. `bPcieGen4Capable` does
   happen to be `0` in byte 0 — the conclusion survives, the evidence does not, and the file
   never cites the run that would prove it.
3. **`crates/kayfabe-device/src/ga10x.rs:1381-1383`** repeats the `0x20800af3` reasoning for
   `GA106_CONF_COMPUTE`.
4. **`crates/kayfabe-device/src/ga10x.rs:1399-1401`** repeats the `0x20800aac` reasoning for
   `GA106_BIF_STATIC`.

★ Note *how* these survive, because it is the same mechanism recorded in `CLAUDE.md`: a gate
demanding a `C:` citation is **satisfied** by a row citing an empty body as corroboration.
A citation gate checks that a claim is *sourced*; it never checks that the source says what
the claim says. `crates/kayfabe-abi/src/fmbsize.rs:87-89` already carries the corrected bytes
for both rows — the correction is in the same crate, two files away, and did not reach the
claims.

**The edit is small and it is not a value change.** Replace the trailing-zero argument in all
four sites with the hardware bytes from `traces/real_ga106/`, and state the divergence
(`confcompute`) or the survival-by-coincidence (`bifstatic`) explicitly. No served byte moves.

---

## 8. What this audit does not establish

- ⊘ **It does not check that any A citation is true.** It checks that a citation exists and
  explains the value. Three miscited `mode2_initctrl_ga106.h` line numbers were already found
  this way in `oracle.rs`'s `CAPTURE_RELIANCE` work — right values, wrong addresses, citation
  gate satisfied every time. The same class certainly survives elsewhere here.
- ⊘ **The A grade on the GMMU/doorbell/USERD block is a property of the run, not the file.**
  `gmmu_fmt_oracle`, `worksubmit_token_oracle`, `userd_chid_oracle` and `pushbuffer_abi_oracle`
  all use `require_oracle!`, which returns early when the ogkm checkout is absent. On a box
  without it those values fall back to citation-only.
- ⊘ **`capability.rs`'s 239 `Nvproxy` rows are graded at the block, not the row.** That is 16 %
  of the whole denominator resting on one claim ("we ported nvproxy's table"). It is a
  *reasonable* claim and it is checkable — nvproxy is vendored — but it has not been checked
  row by row, and this audit did not check it either.
- ⊘ **The generator's input set is pinned by a version string only.** `OGKM_VERSION` comes
  from `version.mk`; there is no hash, no submodule, no CI phase that re-runs the generator
  and diffs `src/generated/`. A hand-edited constant *value* would not be caught by the
  `RUSTC_OFFSETS` assertions, which check layout.
- ⊘ **Nothing here was booted.** Every level-B row inherits its run from the trace or boot it
  names; this audit ran no hardware and its own claims are readings.

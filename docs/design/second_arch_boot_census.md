# w295 — booting a SECOND ARCHITECTURE (Ada): the census, and why no box was rented

**STATUS: ANSWERED BY CENSUS, 2026-08-14.** Branch `w295-second-arch-boot`, from `origin/master`
at `72f902f`. **No vast box was rented. No boot was run. No host GPU was touched.**

> ## ⊘⊘⊘ LEAD WITH THE CONTRADICTION — every leg of this rung is already on disk
>
> The brief pre-registered three outcomes (boots / fails-at-a-named-stage / cannot-rent) and
> said: *"If the census says the boot outcome is already known, SAY SO AND DO NOT RENT."*
> **The census says exactly that, three times over:**
>
> 1. ★★★ **A REAL RTX 4090 (AD102) WAS ALREADY RENTED, BOOTED AND CAPTURED** — 2026-08-03,
>    driver 575.51.03. The trace is **committed**: `traces/ad102_boot1.bin` (1 140 256 bytes),
>    beside a GA102 control capture. **The Ampere→Ada compatibility delta is measured**, and
>    it is nearly nothing: **0 of 105 common controls differ by a single byte.**
> 2. ⊘ **The emulated GPU cannot be configured as Ada at all.** Ampere is pinned at **four
>    independent points**; `Ad10xArch` is constructed **only in tests**. Asking for the Ada
>    device id today yields a named refusal, `NoChipForDevice` — not an Ada boot.
> 3. ⊘ **A boot does not touch the host GPU.** So renting *any* card for a boot-only rung
>    varies nothing.
>
> ⇒ **Renting an Ada box would have booted the identical GA106 emulation, through the identical
> Ampere driver HAL, without issuing one ioctl to the rented card** — and a green would have
> been reported as *"kayfabe boots on Ada"* while meaning nothing of the kind.

---

## 1. ★★★ THE QUESTION WAS ALREADY ANSWERED ON REAL ADA SILICON

`docs/design/rpc_trace_capture.md` §8 records a campaign that **already did what this brief
proposed** — rented Ada hardware, booted it, captured it, and diffed it against Ampere at a
pinned driver version.

- **The parts:** RTX 4090 (**AD102**) and RTX 3090 (**GA102**), plus the RTX 3060 (GA106)
  reference. Driver held constant at **575.51.03**.
- **The artefacts, committed and verified present in this worktree:** `traces/ad102_boot1.bin`
  (1 140 256 B), `ad102_boot1.json`, `ad102_boot1_dmesg.log`, `ad102_boot1_smi.txt`,
  `ad102_boot1_restore_smi.txt` (+ the five-file GA102 set).
- **The capture is complete, not partial** — `ad102_boot1.json`: **1112 records, 551 requests /
  561 replies, `n_dropped: 0`, `wrapped: false`, `n_len_disagree: 0`**, span 2 848 ms, largest
  element 65 536 B (`GSP_RM_CONTROL`, rpc_fn 76).
  ⚠ Worth stating given this project's history with `dlen=0` oracle rows: **nothing here is an
  empty or truncated row.**
- **The boot succeeded** — `ad102_boot1_smi.txt`, `Mon Aug 3 19:53:26 2026`:
  `NVIDIA GeForce RTX 4090 … Driver Version: 575.51.03 … 24564MiB`. The stock **proprietary**
  module was restored and re-verified afterwards (`rpc_trace_capture.md:594`).

**The measurement** (`rpc_trace_capture.md:597-599`):

> **105 common, 17 only-GA102, 3 only-AD102, and 0 — zero — reply-size differences.**

**The conclusion** (`:670-677`), which is the direct answer to *"stress-test the compatibility"*:

> the entire observed difference is **NVLink** — 17 controls, gated on a probe the GPU answers;
> **ECC** — 6 controls, gated the same way; and **nothing else.** Not one control demanded
> merely because the die is Ada, and **not one byte** of reply-size difference across 105 common
> controls.
> ⇒ **`Arch` × capability**, and on this evidence the capability term is carrying all of it.

⇒ ★★ **The arch axis is nearly empty; the capability axis carries the variation.** A GA102
without an NVLink bridge and an AD102 demand the same sequence. And the derived rule
(`:604-607`): **reply size is keyed on driver version, never on arch.**

⚠ **Scope, held to what that run actually covered** (`:679-683`): three boards, two
architectures, two capabilities, two `nvidia-smi` boots each. **No CUDA context, no compute, no
refusal injection.** It bounds the **control plane at boot** — which is precisely the plane this
brief scoped the rung to.

★ **The operative consequence is a liability, not a feature** (`:686-693`): our emulated GSP
*chooses* these answers. Answering `0x20800a87` `NV_OK` summons 17 NVLink controls we must then
serve; answering the three ECC probes `NV_OK` summons three more. The measured answer on parts
lacking each capability is `NV_ERR_NOT_SUPPORTED` — *"the rare place where fidelity and
least-work agree."*

---

## 2. ⊘ THE EMULATED GPU HAS NO ADA CONFIGURATION — four independent hard-wires

Each of these alone makes the briefed experiment vacuous.

### 2.1 The `Arch` is a literal at the composition root
`crates/kayfabe-qemu-raw/src/shim.rs:13017-13022` — the **only** `Gpu::new(` in the file:

```rust
let gpu = kayfabe_core::gpu::Gpu::new(
    Box::new(kayfabe_chips::Ga10xArch::new()),
    ...
```

It takes no device id. The other production `Arch` constructions (`kayfabe-crec/src/ga10x.rs:42`,
`replay.rs:245`) are also `Ga10xArch`. **`Ad10xArch::new()` is constructed in exactly two test
files** (`tests/tests/arch_axis_second_generation.rs`, `crates/kayfabe-chips/tests/host_classes.rs`).
The MMU is the same: `shim.rs:9599`, `:9731`, `:11181` all hard-code `Ga10xGmmu::new()`.

### 2.2 The chip table has one row, and asking for Ada is a named refusal
`crates/kayfabe-device/src/lib.rs:665`:

```rust
pub static CHIPS: &[&ChipProfile] = &[&ga10x::GA106];
```

There **is** a runtime knob — `DEFINE_PROP_UINT32("chip-device-id", ...)`,
`qemu/hw/misc/nvkvm/nvkvm.c:3253` — and Ada dead-ends in it.
`docs/design/compatibility_matrix.md:111-118` already states the outcome:

> ★ **`AD106` has a VBIOS profile and no `ChipProfile`.** … an operator can ask for `0x2803`
> today and **will get `NoChipForDevice` rather than an Ada boot**. … **AD106 has one part of
> three.**

### 2.3 The guest driver is told it is a GA106, at two levels
- PCI: `crates/kayfabe-device/src/ga10x.rs:1692` — `pci_device_id: 0x2504` (RTX 3060).
- Silicon id: `ga10x.rs:517,521` — `PMC_BOOT_0_GA106_A1 = 0x1760_00A1`,
  `PMC_BOOT_42_GA106_A1 = 0x176A_1000`. **This is the register RM reads to pick every HAL.**

⇒ The stock driver binds `ChipHal: GA106` and runs the Ampere paths **regardless of what
silicon is in the machine.**

### 2.4 `Ad10xGspModel` — the one genuinely-Ada artifact — is unreachable by construction
The GSP model comes from the `ChipProfile`, never from the `Arch` (`kayfabe-device/src/plane.rs:1455`
`let model = (chip.gsp_model)();`). And `kayfabe-device` **does not depend on `kayfabe-chips`**, so
`CHIPS` cannot even name `Ad10xGspModel` today. (No cycle blocks the edge — it simply is not there.)

---

## 3. ⊘ A BOOT DOES NOT TOUCH THE HOST GPU — two independent off-switches, both default OFF

`scripts/bench/boot_nvkvm.sh` — the entire QEMU command line — references **no `/dev/nvidia*`,
no host GPU, no isolate to a real card**:

```
-device nvkvm-gpu,bar1-size=268435456,bar2-size=33554432,id=kf0
```

And forwarding is off twice over:

- **Build-time** — `KAYFABE_SHIM_FEATURES` (`scripts/build_qom_shim.sh:25-33`) defaults **empty**,
  so the shipping archive does not even *link* `kayfabe-isolate-host`.
- **Runtime** — `KAYFABE_ISOLATES` (`shim.rs:13134`, `:13227-13235`) unset ⇒
  `IsolatePlane::Stillborn`.

**Measured, five separate rungs** (`docs/design/boot_measured_2026_08_01.md:117,229,441,595,892`):
*"No host GPU. This box has none, forwarding is off, and the isolate factory is
`StillbornIsolates`."* And the driver **binds anyway** — `traces/guest_boots/run_vh2a_probe.log`
shows `MODPROBE_RC=0`, `SMI_RC=0` while the same boot's census reads `0 forwarded`, which
`bench_rebuild_notes.md:672-689` states *"is correct and not a defect."*

⇒ For a boot-only rung the rented card is **inert**. The host GPU is engaged only on the
forwarding/compute path (`^CUP2_RC=`), which this rung explicitly scoped **out**.

### ⚠ A CORRECTION TO THE BRIEF'S OWN TRAP LIST

The brief said: *"Persist `dmesg` to its own file and **ASSERT it is non-empty and contains
`NVRM`**."* **That assertion is measured-useless here, and this tree already knows it.**
`scripts/bench/boot_capture.sh:246-250`: the module-load banner is *itself* an `NVRM:` line, so
an `NVRM` check **passes on a capture where the adapter was never touched.**

The real criterion (`boot_capture.sh:246-257`, `assert_boot_evidence.sh:113-117`) is a
**disjunction**, and the disjunction is load-bearing:

```
grep -c 'RmInitAdapter\|rm_init_adapter'  $DMESG   -eq 0
   && ! grep -q 'SMI_RC=0'                $PROBE   →  die
```

⊘ `assert_boot_evidence.sh:106-112` records that the first tagged run of that gate **failed a
good boot** for lacking the second clause — `RmInitAdapter failed!` prints only on *failure*.
⇒ Same class as the brief's own `143`/`124` lesson: the obvious string is present in both the
good and the bad state.

---

## 4. ⊘ THE BRIEF'S ARCHITECTURE ARGUMENT IS REFUTED — Ada is the arch guaranteed to AGREE

The brief rejected Turing because *"GA10x runs Turing's GSP boot skeleton, so it is the least
informative"*, and chose Ada. **That reasoning applies verbatim to Ada.**

Measured from `research_clones/ogkm-580.159.04/src/nvidia/generated/g_kernel_gsp_nvoc.c` — the
driver's own generated HAL dispatch table, **253 `kgsp*` bindings**, of which **exactly 5**
separate Ada from Ampere:

| binding | Ada impl | what it is |
|---|---|---|
| `kgspGetBinArchiveGspRmBoot` (`:804`) | `_AD102` | firmware **blob** |
| `kgspGetBinArchiveBooterLoadUcode` (`:1548`) | `_AD102` | firmware **blob** |
| `kgspGetBinArchiveBooterUnloadUcode` (`:1581`) | `_AD102` | firmware **blob** |
| `kgspIsScrubberImageSupported` (`:1366`) | `_e661f0` → `return NV_TRUE` | predicate |
| `kgspExecuteScrubberIfNeeded` (`:1383`) | `_AD102` | ★ the one real mechanism |

Everything else — including **`kgspBootstrap` itself** (`:715`), `kgspExecuteBooterLoad` (`:1402`),
`kgspWaitForGfwBootOk` (`:1517`) — binds **`_TU102`** under the mask `0x01f0ffe0` =
`TU102 | TU104 | TU106 | TU116 | TU117 | GA100 | GA102 | GA103 | GA104 | GA106 | GA107 | AD102 |
AD103 | AD104 | AD106 | AD107`.

⇒ ★ **Ada and Ampere sit in the same HAL bucket for the entire GSP boot sequence**, and three of
the five differences are *binary payloads*, not logic. This is the objection **task #118** already
filed and the brief did not carry forward (`docs/design/open_questions_for_the_owner.md:365-372`):

> *"Ada holds … ⚠ But it is the **easiest member of the universe**, and provably so … Ada and
> GA10x are **byte-identical** … An experiment that selects its easiest case produces a green
> with no red available to it."*

★★ The generation with a *structural* disagreement is **Hopper** — four of eighteen `GspReg`
variants have no register on GH100, and it boots an FSP EMEM queue, not a falcon sequence.

---

## 5. THE NAMED STOP POINT — ⚠ and it was ALREADY ON DISK before I derived it

The brief's best outcome was *"name the stop point BY IDENTITY"*. I derived it from NVIDIA's
source — and then found the census had it already.

> ### ★★★ ALREADY ANSWERED — `docs/design/gsp_boot_gate_spec.md:294-322`
> > *"**Ada AD10x dispatches to `_TU102` or `_GA102` for literally every boot HAL slot**; the
> > only Ada-specific code in the whole GSP tree is `kgspExecuteScrubberIfNeeded_AD102`. …*
> > ***⇒ Adding Ada is a table row.** The only non-constant is emulating
> > `NV_PGC6_BSI_SECURE_SCRATCH_15[31:29] == 3` so `_kgspIsScrubberCompleted` short-circuits — a
> > one-register fake, not new logic. ⚠ **That register is not in `GspReg`'s vocabulary and not
> > in `Ad10xGspModel`.**"*
>
> ⚠ **This is the `check_whether_the_question_is_already_answered` class firing on me**, and it
> is worth recording because *I re-derived it from the C rather than reading forward.* My
> derivation is **corroboration, not discovery**. It adds exactly one thing, below.

**The stage.** `kgspBootstrap_TU102`, `.../gsp/arch/turing/kernel_gsp_tu102.c:503-508` — the
**first statement of GSP bootstrap, BEFORE FWSEC/FRTS**:

```c
// Execute Scrubber if needed
if (((bootMode == KGSP_BOOT_MODE_SR_RESUME) || (bootMode == KGSP_BOOT_MODE_NORMAL)) &&
    (pKernelGsp->pScrubberUcode != NULL))
{
    NV_ASSERT_OK_OR_RETURN(kgspExecuteScrubberIfNeeded_HAL(pGpu, pKernelGsp));
}
```

**★ What my derivation adds: the guard is UNCONDITIONALLY true on Ada, so the short-circuit is
the only way past.** `_kgspPrepareScrubberImageIfNeeded`, `kernel_gsp.c:3730-3737`:

```c
// WAR for Bug 5016200 - Always run scrubber from kernel RM for ADA config
if ((neededSize > prescrubbedSize) || kgspIsScrubberImageSupported(pGpu, pKernelGsp))
    ... kgspAllocateScrubberUcodeImage(pGpu, pKernelGsp, &pKernelGsp->pScrubberUcode);
```

and on AD102–AD107 `kgspIsScrubberImageSupported_e661f0` is `return NV_TRUE`
(`g_kernel_gsp_nvoc.h:1833-1835`). ⇒ `pScrubberUcode` is **always** non-NULL on Ada — NVIDIA's own
comment says so — so the call **always** happens, wrapped in `NV_ASSERT_OK_OR_RETURN`, a hard
abort. **You cannot dodge it by declining to load the ucode.**

**The register**, `kernel_gsp_ad102.c:43-45`:

| item | value | source |
|---|---|---|
| register | **BAR0 + `0x001180fc`** (`NV_PGC6_BSI_SECURE_SCRATCH_15`) | `published/ada/ad102/dev_gc6_island.h:27` |
| field | `_SCRUBBER_HANDOFF` = bits **31:29** | `dev_gc6_island_addendum.h:28` |
| required | `>= _DONE` = **0x3** | `dev_gc6_island_addendum.h:29` |

**Do we model it?** **No** — `0x1180fc` appears **zero times** in the tree. Every `scrubber` hit in
`crates/` is the unrelated *CeUtils FB-scrubber channel*.

**⇒ The predicted failure, verbatim from the driver.** The unmodelled register reads `0`, so
`handoff = 0 < 3`: `_kgspIsScrubberCompleted` → false; `kflcnReset_HAL` on SEC2; the AD102
scrubber HS ucode runs against our emulated falcon; the handoff bits are still `0`; the driver
prints **`"failed to execute Scrubber: done bit not set"`** and returns **`NV_ERR_GENERIC`**
(`kernel_gsp_ad102.c:79-82`), aborting `kgspBootstrap` **before FWSEC, FRTS, WPR2, the LibOS args
and the msgq.**

⚠ **This is a DERIVED prediction, not a measurement.** Nothing here has run an Ada-configured guest.

---

## 6. ⚠ THE TRAP IF SOMEONE JUST ADDS A ROW — the boot would be a LIE, not a result

`docs/design/support_matrix_seam_audit.md:253-257`, written for exactly this scenario:

> the moment a second row is added to `CHIPS`, that generation **silently inherits an invented
> MMU format, USERD model and pushbuffer ABI**.

`Ad10xArch` is `MockArch` + a real GSP model (`crates/kayfabe-chips/src/ad10x.rs:325-408`).
Of its eleven `Arch` methods:

| verdict | methods |
|---|---|
| **(a) genuinely Ada** | `gsp()` only |
| **(c) shared with Ampere, and *sourced* from RM's dispatch** | `vchid_from_userd_flags`, `decode_doorbell`, `host_classes` |
| **(b) MockArch — INVENTED** | `classify`, **`mmu`**, `userd`, `pushbuffer`, `is_case2_control`, `engine_of_object` |

Three of those are actively dangerous:
- **`mmu()`** (`ad10x.rs:385-387`, bare `self.inner.mmu()`, no doc) returns an **invented bit
  layout** where RM's own dispatch proves `_GA10X` is correct for `GA100…AD107`. The audit calls
  it *"the only place in this audit where the code states something demonstrably false about
  NVIDIA rather than merely incomplete"* (`:451-454`). ★ It is **one field assignment** from being
  right — `Ga10xGmmu` already implements the correct format.
- **`pushbuffer()`** — *"a real method header is decoded by switching on `header >> 24` against
  invented opcodes, so a real method run can decode to a `CeLaunchDma` with a **fabricated**
  destination, length and work kind"* (`mock_fidelity_audit.md:288-292`).
- **`userd()`** — `MockUserd` puts `GP_GET`/`GP_PUT` **8 bytes apart** where real GA10x/Ada has
  them **4** apart.

⇒ ★★ **This is why the brief was right to scope cup2 out — and it under-stated the reason.** The
danger is not that cup2 would fail; it is that **a boot could go green on invented internals** and
be read as an Ada result. `Ga10xArch::name()` spells its refusals into its own string; Ada's says
only `"AD10x (AD106)"` — and `name()` is what the multi-GPU homogeneity guard reports.

⚠ Also already flagged (`replay_audit.md:307-314`): `SEC2_BOOTER_UNLOAD = 0xff` and
`WPR2_LO_UP`/`WPR2_HI_UP` (`ad10x.rs:122,129,131`) are **invented and marked as such** — *"No Ada
card was measured for this number"* — and if wrong, an Ada boot fails FWSEC's **exact** compare.

---

## 7. What renting WOULD be good for — a different rung, with a real red available

The one axis a rented box genuinely varies is the **HOST**, and there is a documented gap
(`docs/design/host_driver_version_pin.md:186-206`):

> *"**We have exactly one host driver.** A version axis we cannot exercise is a mechanism with
> **no red available to it** … When a second host driver becomes available, the honest next step
> is one more interval in `host_driver.rs`."*

**Measured today** (`vastai search offers 'vms_enabled=true rentable=true num_gpus>=1
disk_space>=200'` → 62 KVM-capable offers; filtered `reliability2 >= 0.997`):

| offer | GPU | rel2 | $/hr | host driver | R2 verdict |
|---|---|---|---|---|---|
| `27743076` | RTX 4090 (AD102), Denmark | 0.9987 | 0.315 | **580.95.05** | ✅ inside `[580.65.06, 581)` |
| `27875865` | RTX 4080S (AD103), Denmark | 0.9985 | 0.149 | **580.95.05** | ✅ |
| `39855349` | RTX 4090, Romania | 0.9978 | 0.416 | **580.142** | ✅ |
| `46130698` | RTX 4090, Hungary | 0.9974 | 0.602 | **590.48.01** | ⊘ refused by name |
| `25318158` | RTX 6000Ada, Japan | 0.9991 | 1.321 | **570.133.20** | ⊘ refused by name |

⊘ **But the driver-version axis is a driver-INSTALL choice, not a card choice.** The standing
recipe purges the box's driver and installs the **580.159.04 open `.run`** on every rented box
regardless of card (`docs/reference/bench_rebuild_notes.md:88-95,353-358`), and that `.run` is
not card-specific. ⚠ `bench_rebuild_notes.md:706-712` §G also warns that **vast's advertised
`driver_version` describes the host machine, not the VM you get** — never select on it. ⇒ the
table above is a *starting* state, not a result.

★ Ada boxes are plentiful and cheap. ⇒ A *forwarding* rung on an Ada host has a real red
available. **It is not this rung, and it needs the MMU fix in §6 first.**

### ★★ AND IF A BOX IS EVER RENTED FOR THIS: run the LADDER, not the bench

The cheap, high-value host-side test needs **no QEMU, no guest image, no tap** — only a host
driver:

- `cargo build --release --bin kayfabe-rm-ladder` — 8 crates, **10.2 s**, no external deps
  (`bench_rebuild_notes.md:405-412`).
- `./kayfabe-rm-ladder --gpu 0 --engines`, diffed against the committed transcripts in
  `docs/reference/bench_evidence/rm-ladder-box{2,3}-*.out`.
- ★ **Across three different GA106s and a full campaign's revision gap the diff is ONE LINE** —
  the RM-allocated `hClient` (`:425-445`, `:649-666`). A genuine cross-machine regression oracle
  with exactly one line to mask.
- It reaches **R13b** (runlist routing `{0,1,2,8}`), **R14** (device memory via an independent
  mapping), **R15** (semaphore released), **R16** (sandboxed doorbell), **R17** (CE copy) — i.e.
  every host-side thing an Ada card could plausibly break — in **~23 min** of provisioning
  instead of the **~35–55 min** a full bench rebuild costs (`:65`, `:316-317`, `:594`).

⊘ **One thing a bigger card will not buy:** RM's ioctl lock is **device-wide, not per-client** —
0.93–0.96× from 4 workers *and* from 4 separate clients, measured on two boxes
(`bench_rebuild_notes.md:209-230`, `:454-466`). An isolate pool buys isolation, not verb
throughput.

### ⊘ And the host-class pin would not have fired on Ada either

`crates/kayfabe-chips/src/host_classes.rs:201-203` — `pinned_host_classes()` is `Ga10xHostClasses`,
**always**, and *nothing probes the host*; there is **no refusal at all** if the host card is the
wrong generation. But on Ada that pin is **provably identical**: `Ad10xHostClasses` (`:104-134`) is
row-for-row GA10x in all three roles, because *"Ada defines **no** `ADA_CHANNEL_GPFIFO_*`,
`ADA_USERMODE_*` or `ADA_DMA_COPY_*` at all"* — read from NVIDIA's per-chip table
(`ogkm-580: g_gpu_class_list.c:1738-1744`) and asserted at
`crates/kayfabe-chips/tests/host_classes.rs:269-285`. **Hopper is where all three diverge, and two
of the three wrong ids would be SERVED rather than refused.**

⚠ Runway at census time: **`VAST_BALANCE=$17.80  VAST_BURN=$0.241/hr  VAST_RUNWAY=74h`**, with two
sibling lanes live on `vh`/`vh2`. Adding a 4090 would have cut runway to ~32 h for a measurement
that could not have differed from `vh`'s.

---

## 8. What it would take to make an Ada boot real (named, NOT built)

In dependency order. **None of this was done** — the brief forbade fixing what the census found.

1. **A `ChipProfile` row for AD106** — needs a new `kayfabe-device → kayfabe-chips` dependency
   edge (or moving `Ad10xGspModel`), because `CHIPS` cannot currently name it.
2. **An `arch:` field on `ChipProfile`**, feeding the single `Gpu::new` expression. ⚠ Constraint:
   `crates/kayfabe-qemu-raw/tests/e2_doorbell.rs:477-482` asserts `Gpu::new(` appears **exactly
   once** in shim.rs — so the selector must be an *expression*, not a second call.
3. **AD106 `PMC_BOOT_0`/`PMC_BOOT_42`** — without these RM picks the Ampere HAL anyway.
4. **The scrubber observable of §5** — model `0x1180fc` bits 31:29, or bootstrap cannot pass its
   first statement.
5. **Real `GmmuFmt` / `UserdModel` / `PushbufferAbi`** — §6. The GMMU one is `Ga10xGmmu` reused;
   RM's own dispatch proves `_GA10X` covers `GA100…AD107`.
6. **Per-part measurements that only silicon can give** — fault-method-buffer size, PCE→LCE masks,
   GR static/info. ~30 `ChipProfile` fields, several `[measured]` on real RTX 3060 parts.

★ The VBIOS half **already exists**: `VBIOS_PROFILES` carries an AD106 row at `0x2803` and builds
a distinct image — though `vbios.rs:492-503` warns its FWSEC geometry is **GA106's, reused**, and
`vbios_version` is a placeholder.

---

## 9. Doc-hygiene defects found in passing (named, not fixed)

- `docs/design/compatibility_matrix.md:44-48` — *"`kayfabe-chips` is not linked"* is **stale**;
  `support_matrix_seam_audit.md:239-251` re-derived at `f760a4b` and found `kayfabe-chips` and
  `kayfabe-mocks` inside **both** shipping closures. ⇒ The mock-backed Ada seams **ship**; they are
  merely unreachable.
- `docs/design/support_matrix_seam_audit.md` line numbers have **drifted** — it cites
  `ad10x.rs:298/324/357…` and `shim.rs:3501`; current are `:325/351/385…` and `:13017`.
- `ARCHITECTURE.md:33` still lists `Arch` as *"trait-only (MockArch = 'Mockingbird')"*, predating
  all three chip impls.
- `crates/kayfabe-chips/src/ad10x.rs:126-131` still carries the **pre-correction** rationale
  ("only non-zero is load-bearing") that `kayfabe-arch`'s `GspReg` docs have since retracted.
- ⚠ **A provenance disagreement about which card the R2 bite ran on.**
  `crates/kayfabe-abi/src/host_driver.rs:301` and `docs/design/host_driver_version_pin.md:132` say
  **RTX 3090**; `bench_rebuild_notes.md`, `CLAUDE.md:161-164` and `host_classes.rs:189` all say the
  bench is an **RTX 3060 / GA106**. One is wrong. It does not affect the pin (which is keyed on
  *version*, not card) but it will confuse anyone reproducing it.
- `docs/design/rpc_trace_capture.md` §7.6 (`:523-538`) is a **retracted** "no AD102 capture" note —
  superseded by §8, and the capture exists. It reads as current to anyone who stops at §7.

---

## 10. Provenance

- **Source revision:** `72f902f` (`origin/master`); worktree `/workspace/kf-w295`, branch
  `w295-second-arch-boot`.
- **Driver source:** `research_clones/ogkm-580.159.04`.
- **Real-Ada evidence:** `traces/ad102_boot1.bin` — RTX 4090 / AD102 / **575.51.03**, captured
  2026-08-03, already committed.
- **Vast offer data:** `vastai search offers`, 2026-08-14, 62 KVM-capable offers.
- **Boxes rented: none. Boots run: none. Host GPUs touched: none.**
- ★ §1 is a real prior measurement; §4 is read from NVIDIA's source; §5 is a **derived
  prediction** and is labelled as such; §2/§6 are read from this tree.

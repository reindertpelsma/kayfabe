# Does the guest KERNEL ever run GR work on its own channel? — and what the C actually did on the userspace GR path

**STATUS: LIVE, 2026-08-11.** Answers a two-part owner question by **reading source only** (open kernel
modules + the C artifact); traces used for corroboration only, never as the basis of a claim.
Q1 (kernel GR channels) is **new material**. Q2 (what the C did) is **NOT new** — it was already
answered on **2026-08-10** in `mode2_channel_ownership_split.md` §5b; this page adds two facts and
otherwise **defers to that page**. Do not re-derive Q2 from here.

★★★ **§1.7 added 2026-08-11 — READ IT BEFORE TREATING §1.5's QUALIFICATION 2 AS OPEN WORK.** §1.5.2
says the watchdog's channel/object/pushbuffer *"are all built at `RmInitAdapter` regardless"*, which is
true of the **guest** and reads as a residual for **us**. It is not one: `nvkvm-rs@425c450` serves the
golden-image tree end to end and refuses the watchdog's three allocations **by name**, all measured on
committed boots. §1.7 has the port-side audit, the one thing that IS missing (a class on
`PushMethod::Opaque`), and the reason this was commissioned twice.

Source of truth for Q1: `research_clones/ogkm-580.159.04` (`version.mk:1` → `NVIDIA_VERSION =
580.159.04`; git HEAD `b81d58e`). ⚠ `research_clones/ogkm` is a **different** version, **610.43.02** —
not read for this page. `ogkm` is versioned, not a spec.

---

## 0. ★★★★★ LEAD — the three things that refute the brief that commissioned this

**⊘ (a) Q2 was already answered, one day before it was asked.** `mode2_channel_ownership_split.md`
§5b (committed **2026-08-10**) contains a full audit of the C's source with the answer, including the
exact framing this page was asked to settle. Re-deriving it cost a lane. ⇒ This is the tree's own named
failure mode — *"a doc committed the day before that already held the answer, then re-derived wrongly"*
— reproduced verbatim. **Check `docs/design/` for a dated STATUS block before commissioning a source
audit.**

**⊘ (b) The most important kernel GR channel is one nobody guessed.** The brief named golden-image
context init as the prime suspect. Golden-image init **is** a kernel GR channel — but it is the
*harmless* one: it allocates the channel and the GR object and then **submits no work at all**
(`gpFifoOffset = 0`, deliberately). The channel that actually **pushes graphics methods from CPU
kernel code** is the **RC watchdog** (`kernel_rc_watchdog.c`), which allocates a `FERMI_TWOD_A` object
on an `RM_ENGINE_TYPE_GR0` channel and writes `NV902D_*` methods into its pushbuffer. It was on
neither of our lists.

**⊘⊘ (c) The C artifact SAW both of these channels and MISLABELLED them in a comment.**
`nvkvm_gpu_emul.c:7008-7012` excludes handles `0xbaba0045` and `0x31415900` from USERD backing,
calling them *"libcuda PROBE/sentinel channels"*. They are neither libcuda's nor sentinels:
`0xbaba0045` is `hChannelId` in `kgraphicsCreateGoldenImageChannel_IMPL`
(`ogkm-580.159.04/src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:2149`) and `0x31415900` is
`WATCHDOG_PUSHBUFFER_CHANNEL_ID` (`.../gpu/rc/kernel_rc_watchdog.c:64`). ⇒ **The C's evidence for the
answer was inside the C, under a wrong name, for months.** Per this tree's own rule: *a comment that
names an exception is a bug report.*

---

## 1. Q1 — VERDICT: **YES.** The guest kernel allocates GR-engine channels, and one of them pushes graphics methods.

**Why this is a complete answer, not a sample** (owner's methodological point, and it holds): a GPFIFO
channel can only be created by `pRmApi->Alloc*` with a host/GPFIFO class, and every such class id in
the tree comes from exactly three producers — `kfifoGetChannelClassId()` (3 call sites), the watchdog's
private `gpfifoMapping[]` table (1 site), and `device->hostClass` in `nv_gpu_ops.c` (1 site, UVM).
All five were read. **[measured]**

### 1.1 The complete list of channels RM's kernel side allocates for itself

| # | what | function / `file:line` | engine | GR/compute object on it? | pushes GR methods? |
|---|---|---|---|---|---|
| 1 | **Golden context image** | `kgraphicsCreateGoldenImageChannel_IMPL` — `src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:2136` | **GR0** (`:2451`) | **YES** — `GR_OBJECT_TYPE_3D` or `_COMPUTE` (`:2505/2509`), class via `kgraphicsGetClassByType` (`:2514`), allocated `:2520` | **NO** — `gpFifoOffset = 0`, *"we only need this channel to be created, but will not submit any work to it"* (`:2421-2426`) |
| 2 | **Bug 4208224 WAR** ("GR scrubber") | `kgraphicsCreateBug4208224Channel_TU102` — `.../gpu/gr/arch/turing/kgraphics_tu102.c:296` | **GR0** (`:495`) | **YES** — `GR_OBJECT_TYPE_3D` (`:511`), allocated `:517` | **NO** from the guest kernel — `gpFifoOffset = 0` (`:487`); the WAR is completed by an **RPC to GSP-RM** (`NV0080_CTRL_CMD_INTERNAL_KGR_INIT_BUG4208224_WAR`, `:269-274`) |
| 3 | **RC watchdog** | `krcWatchdogInit_IMPL` — `.../gpu/rc/kernel_rc_watchdog.c:437` | **GR0** — `// 2d object is only suppported on GR0` (`:999-1000`) | **YES** — `FERMI_TWOD_A` (`:608-610`), allocated `:1090-1096`; comment at `:1089`: *"Create an object that will require a trip through the graphics engine"* | ★ **YES** — see §1.3 |
| 4 | `CeUtils` ×6 (PMA scrubber, global, sysmem scrubber, FB save/restore, CC FIPS ×2) | `ceutilsConstruct_IMPL` — `.../gpu/mem_mgr/ce_utils.c:164`; channel `_memUtilsAllocateChannel` — `.../mem_mgr/arch/maxwell/mem_utils_gm107.c:1203` | **COPY** — `RM_ENGINE_TYPE_COPY(ceId)` (`:1221-1222`) | no — `NVC0B5`-family copy class (`:1056-1074`); CE selection **explicitly excludes GRCEs** (`ce_utils.c:74`) | CE methods only |
| 5 | `Sec2Utils` | `sec2utilsConstruct_IMPL` — `.../gpu/mem_mgr/sec2_utils.c:179` | **SEC2** (`:226`) | no | ⊘ CC-only — hard `NV_ASSERT_OR_RETURN(CC_FEATURE_ENABLED)` at `:189-191` |
| 6 | FB save/restore (`fbsr*`) | `fbsrBegin_GM107` — `.../mem_mgr/arch/maxwell/fbsr_gm107.c:298` | **CE** | **no channel of its own** — on a GSP client it explicitly routes through `CeUtils` (`:309-314`) | — |
| 7 | UVM: `CE`, `CE_PROXY`, `SEC2`, `WLC`, `LCIC` | `kernel-open/nvidia-uvm/uvm_channel.h:120-147`; pools created `uvm_channel.c:3437/3797/3808/3814/3863` | **CE or SEC2 only** — two-way switch `uvm_channel.c:2648-2654` | **no** | no |
| 8 | ECC scrubber channels | `mem_utils_gm107.c:1464`, `:1490` | (would be CE) | — | ⊘ **DEAD** — HALs bind the no-op stub `_92bfc3` (`generated/g_mem_mgr_nvoc.h:1375-1389, 2549-2558`) |

⊘ **UVM never allocates a GR channel, and RM enforces it.** `nv_gpu_ops.c:5720-5728`, `:6185`,
`:6371-6373` reject any TSG engine that is not CE or SEC2. `UVM_GPU_CHANNEL_ENGINE_TYPE_GR` exists
(`nv_gpu_ops.c:9857-9858`) **only** in `retainChannel` — UVM *inspecting a channel CUDA usermode
already created*. **[measured]**

### 1.2 Why the answer is specifically about a GSP-CLIENT kernel — the guard is explicit

`kernel_graphics.c:476-478`:

```c
// Nothing to do for non-GSPCLIENT
if (!IS_GSP_CLIENT(pGpu) && !kgraphicsIsBug4208224WARNeeded_HAL(pGpu, pKernelGraphics))
    return NV_OK;
```

⇒ Golden-image channel creation is not merely *reachable* on a GSP client, it is **GSP-client-specific**.
`kgraphicsStateLoad_IMPL:366-371` says it in words: *"GSP_CLIENT creates the golden context channel GR
post load."* **[measured]**

★ **And there is no ambiguity about which half of RM this is.** `generated/rmconfig.h:260` →
`RMCFG_FEATURE_PLATFORM_GSP 0`; `:275` → `RMCFG_FEATURE_KERNEL_RM 1`; and
`generated/g_chips2halspec_nvoc.c:190-201` shows the tree has exactly two RM variants, `VF` and
`PF_KERNEL_ONLY` — **there is no physical-RM variant in the open tree at all.** Every site above is
compiled into the guest's `nvidia.ko`. **[measured]**

### 1.3 ★★★★★ The watchdog is the one that PUSHES — and it is the whole GR method vocabulary of the kernel

`krcWatchdogInitPushbuffer_IMPL` (`kernel_rc_watchdog.c:1212`, called unconditionally from
`krcWatchdogInit_IMPL:1184`) writes, from CPU kernel code, into the channel's pushbuffer:

| method | `kernel_rc_watchdog.c` |
|---|---|
| `NV902D_SET_OBJECT` (SET_OBJECT with a **GR-engine class**) | `:1246-1248` |
| `NV902D_SET_NOTIFY_A` / `_B` | `:1293-1298` |
| `NV902D_NOTIFY` (`_TYPE_WRITE_ONLY`) | `:1311-1312` |
| `NV902D_NO_OPERATION` | `:1313-1314` |
| `NV906F_SET_REFERENCE` (pre-Hopper) / `NVC86F_WFI` + `NVC86F_MEM_OP_A..D` `_MEMBAR` (Hopper+) | `:1328-1345` |

subchannel = `NVA06F_SUBCHANNEL_2D` (`:648`). Work is submitted by
`krcWatchdogWriteNotifierToGpfifo_IMPL` (`:1414`): GPFIFO entries at `:1370-1371`, GP_PUT at `:1380`,
then **`kfifoUpdateUsermodeDoorbell_HAL`** at `:1404` and `:1474`. Entry point is
`arch/nvalloc/unix/src/osinit.c:2160` — `krcWatchdogInit_HAL` inside `RmInitAdapter`. HAL: stubbed only
on Tegra `T234D | T264D` (`generated/g_kernel_rc_nvoc.c:266-274`); **live on every discrete GSP-client
GPU**. **[measured]**

### 1.4 ★★★★ THE OPERATIVE ANSWER — the emulated axis does **NOT** inherit the MME problem

The brief's real worry was: *if RM runs GR work on a kernel channel, the emulated axis inherits the MME
problem — guest microcode whose output is commands, which defeats any method allowlist.*

**It does not, and this is a strong, enumerable result:**

- A tree-wide grep for **every** graphics/compute method family emitted from kernel code —
  `NV9097_|NVA097_|NVB097_|NVC097_|NVC397_|NVC597_|NVC697_|NVC797_|NVCB97_|NV902D_|NVA0C0_|NVB0C0_|NVC0C0_|NVC3C0_|NVC5C0_|NVC6C0_|NVC7C0_|NVCBC0_|SET_SHADER|LAUNCH_DMA_QMD|SEND_SIGNALING_PCAS`
  over `src/nvidia/src` and `kernel-open` — returns **exactly five hits, all `NV902D_*`, all in
  `kernel_rc_watchdog.c` (`:1247, 1294, 1297, 1312, 1314`)**. **[measured]**
- A grep for `MME` / `_MME_` / `MACRO` over `src/nvidia/src/kernel/` and `kernel-open/nvidia-uvm/`
  returns **zero**. **[measured]**

⇒ **The kernel's entire GR method vocabulary is five 2D methods with constant operands and no MME, no
QMD launch, no SET_SHADER, no compute class methods anywhere.** The emulated axis therefore stays
statefully decodable — `{CE methods} ∪ {five NV902D_ methods}` — which is exactly the property the
two-axis split rests on. **The two-axis design survives Q1.**

### 1.5 The three qualifications, stated so nobody over-reads §1.4

1. ⚠ **The `SET_OBJECT` is real.** The watchdog puts a graphics-engine class on a kernel channel and
   makes the GPU take *"a trip through the graphics engine"* (`:1090`). Emulating that channel means
   producing a **GR-engine notifier write**, not just a CE completion. It is decodable; it is not free.
2. ⚠ **Enablement is client-driven, so "can" ≠ "does".** `osinit.c:2164` calls `krcWatchdogDisable`
   immediately after init (*"initialize the watchdog (disabled by default)"*, `:2159`); a client turns it on via
   `NV2080_CTRL_CMD_RC_ENABLE_WATCHDOG` (`kernel_rc_watchdog_ctrl.c:74`). `krcWatchdogChangeState_IMPL`
   documents the convention at `:134-141`: **X enables it, CUDA disables it.** Skipped entirely under
   MIG or if `FERMI_TWOD_A` is unsupported (`:471-474`), and disabled under Confidential Computing
   (`kernel_rc.c:208-211`). ⇒ On a headless CUDA guest the *submission* is normally off — but the
   **channel, the `FERMI_TWOD_A` object and the pushbuffer contents are all built at `RmInitAdapter`
   regardless.** A design that assumes "no kernel GR channel exists" is wrong even headless.
   ★ And the moment we ship graphics/display (Mode-1 already has Vulkan parity), X enables it.
3. ⚠ **Bug 4208224 is Turing-only, and Turing is the support floor.** `generated/g_kernel_graphics_nvoc.c:355-370`:
   `bBug4208224WAREnabled = NV_TRUE` **only** for `RmVariantHal: PF_KERNEL_ONLY` **and**
   `ChipHal: TU102 | TU104 | TU106`. Ampere is `NV_FALSE` — so our GA106 bench **cannot see this
   channel at all**, and a bench-derived "we never saw a kernel GR channel" would be a false negative
   on the exact chips the support matrix admits (`support_matrix_asymmetry.md`: Turing+ floor).

### 1.6 Corroboration from committed traces (⊘ corroboration only — the answer above is from source)

- `../nvkvm-rs/docs/reference/gsp_demand_list_cap1.tsv:66` — entry 61 of the demand list extracted from
  `cap1_coldboot_hermetic` is `alloc 0x0000902d FERMI_TWOD_A ... GSP_RM_ALLOC`. ⇒ The watchdog's 2D
  object is allocated during plain `nvidia.ko` load, in the **hermetic** capture, with no CUDA present.
- `../nvkvm-rs/docs/reference/bench_evidence/run_p35_1f88649_dmesg.log:20,22` (and six sibling logs) —
  `GspRmAlloc failed: hClient=0xc1d00008; hParent=0x31415903; hObject=0x31415900; hClass=0x0000c36f`
  then `Assertion failed: status == NV_OK @ kernel_rc_watchdog.c:1198`. ⇒ Live boots, this month, of the
  guest kernel building the watchdog GR channel.
  ⊘ **`[measured 2026-08-11]` the count is understated and the direction matters**: `grep -l 31415900`
  over `../nvkvm-rs/docs/reference/bench_evidence/` returns **19** files, **12** of them `*_dmesg.log`.
  Under-counting evidence is the benign direction, but the number was written from a sample, not a count.
- Same logs `:17` — `kgraphicsCreateGoldenImageChannel(pGpu, pKernelGraphics) @ kernel_graphics.c:508`.
  ⇒ The golden-image path is live on a GSP client, as `IS_GSP_CLIENT` predicts.

### 1.7 ★★★★★ THE PORT SIDE — what `kayfabe` actually does with both allocations `[measured 2026-08-11, nvkvm-rs@425c450]`

⊘ **This section exists because §1.5's qualification 2 reads as an open residual and it is not one.**
Everything below is read from the **consuming** crate (`kayfabe-rmrpc`, `kayfabe-fwd`) and corroborated
against committed boot logs, never from a text search alone.

#### ⊘⊘ (a) The golden-image channel `0xbaba0045` is SERVED, and its 3D object has been served since 2026-08-08

`0xbaba0045` never failed. In the one boot log that names it at all
(`run_pro1_423bf08_dmesg.log:11`) it appears **only as `hParent`** — i.e. the port had already accepted
it as a live object — and the alloc that failed was its child `0xbaba0046`, `AMPERE_B` `0xc797`. The
channel's own class is `AMPERE_CHANNEL_GPFIFO_A` `0xc56f`, which is permitted
(`nvkvm-rs: crates/kayfabe-abi/src/capability.rs:1080`) **and** decoded
(`crates/kayfabe-abi/src/versions.rs:1119` → `AllocParams::Channel`).

`0xc797` was then admitted the same day (`capability.rs:1146` `Origin::Empirical`,
`versions.rs:1176`), and `[measured 2026-08-08, boot amb1_ee1994b]` the whole 3D-object chain went
**silent** — five dmesg lines removed, zero added, `run_amb1_ee1994b_dmesg.log` contains no `0xbaba…`
line at all (`nvkvm-rs: docs/design/execution_plane_increments.md` §14.26).
⇒ **The golden-image tree is fully served today. There is nothing to make explicit.**

#### ⊘⊘ (b) The C's mislabel was **NOT** inherited into the Rust port

`git grep -nE '0xbaba|0x3141' 425c450 -- crates` finds **no handle-value special case anywhere**. The
only occurrences are prose: a doc comment in `crates/kayfabe-rmrpc/src/policy.rs:248` that quotes a
measured refusal. The port has **no** analogue of `nvkvm_gpu_emul.c:7010-7012`'s
`is_sentinel = ((hObject & 0xffff0000u) == 0xbaba0000u) || ((hObject & 0xffffff00u) == 0x31415900u)`.
★ The mislabel was also caught independently on the Rust side — `execution_plane_increments.md:9323`:
*"Only knowing that `0x31415900` is not a libcuda handle…"*.

#### ★ (c) The watchdog's allocations are refused **BY NAME**, on three different gates, and counted

Nothing silently defaults. `[measured 2026-08-08, boot amb1_ee1994b]`
(`nvkvm-rs: docs/reference/bench_evidence/run_amb1_ee1994b_qemu.log:83-89`) the boot summary prints the
whole refusal census, and it reconciles exactly with the three `GspRmAlloc failed` lines in the same
boot's dmesg:

| the watchdog asks for | our gate | named refusal | site |
|---|---|---|---|
| `0x0070` `NV01_MEMORY_VIRTUAL` (`kernel_rc_watchdog.c:669-676`) | permitted (`capability.rs:827`), **no decoder** | `BridgeRefusal::UnmappedAllocClass` | `crates/kayfabe-rmrpc/src/lib.rs:1276` |
| `0xc36f` `VOLTA_CHANNEL_GPFIFO_A` (`:1096-1101`) | **not on the allowlist** | `BridgeRefusal::AllocClassNotPermitted{denial: NotOnAllowlist}` | `crates/kayfabe-rmrpc/src/lib.rs:1268-1272` |
| its GR context promotion `0x2080012b` | client/object unknown to the graph | `PromoteFault::UnknownContextObject{client: 0xc1d00008, object: 0x31415900}` | `crates/kayfabe-rmrpc/src/policy.rs:248` |

The same log's control census shows `control 0x2080012b result 0x00000000 x2` **beside**
`result 0x00000056 x2 REFUSED` — the golden image's two promotions served, the watchdog's two refused.
⇒ `PromoteFault::UnknownContextObject` is not a shrug; it is the discriminator that separates the two
callers of one control id.

★ **Its `ClientKind` is `Kernel` and its declared channel kind is NOTHING — because the alloc never
reaches the object model.** `hClient=0xc1d00008` is a kernel-RM client, so it declares
`processID == KERNEL_PID` and classifies `ClientKind::Kernel` on a Linux guest
(`crates/kayfabe-abi/src/guest_os.rs:259,285`), which folds it into the one `SYSTEM_ANCHOR` component
(`crates/kayfabe-core/src/project.rs:107,1160-1170`). **Had** the channel been admitted it would
therefore be `GuestChannelKind::Emulated` (`project.rs:311-317` — the one derivation) and hence
`HostChannelKind::Scratchpad` (`crates/kayfabe-core/src/channel_kind.rs:309`). ⊘ But it is refused at
the bridge, so no `ChannelFacts` is ever materialised and **no kind is declared for it at all**. That
is the honest answer, and it is not the same as "Emulated".

#### ★★★ (d) `0xc36f` on GA106 is CORRECT stock behaviour, not an artefact of our device

⊘ Worth stating because it looks like a bug and is not. GA106's own class list carries **three**
`ENG_KERNEL_FIFO` GPFIFO classes — `AMPERE_CHANNEL_GPFIFO_A` (`g_gpu_class_list.c:1113`),
`TURING_CHANNEL_GPFIFO_A` (`:1166`) and `VOLTA_CHANNEL_GPFIFO_A` (`:1168`) — and the watchdog's private
`gpfifoMapping[]` is scanned **first-match-wins in ascending-arch order**
(`kernel_rc_watchdog.c:622-652`), so it stops at **Volta** and never reaches Ampere. Real silicon does
the same. ⇒ Admitting `0xc36f` would be admitting a class a real GA106 genuinely serves, not papering
over a wrong class list. (`nvkvm-rs: execution_plane_increments.md` §14.22 records this and the
contrasting `kfifoGetChannelClassId` numeric-maximum rule that gives the golden channel `0xc56f`.)

#### ★★ (e) If the watchdog ever DID submit — there are TWO gates before a method, and only the third is silent

The five `NV902D_*` methods cannot be reached today, and opening one gate alone changes nothing:

1. **Channel alloc** `0xc36f` — refused, `NotOnAllowlist` (above).
2. **`FERMI_TWOD_A` `0x902d` object alloc** — ⚠ **permitted** (`capability.rs:945`) but **undecoded**:
   `alloc_params` has no arm and falls to `_ => None` (`versions.rs:1308`) ⇒
   `BridgeRefusal::UnmappedAllocClass`. `capability.rs:2629` pins the decodable set at **16** classes
   and `0x902d` is not among them. So `krcWatchdogInit`'s `RmAllocObject` at
   `kernel_rc_watchdog.c:1089-1096` would fail even with gate 1 open.
3. **The methods themselves — and this is the ONE place that is silent.** `[measured]` the production
   decoder `Ga10xPushbuffer::decode_method` (`crates/kayfabe-chips/src/ga10x.rs:1486-1505`) dispatches
   on `(method_offset, arg_words)` with **three** arms and **no class gate at all**:
   - `NV902D_SET_OBJECT` (`0x0000`) collides with `NVC56F_SET_OBJECT` and decodes to
     `PushMethod::SetObject{class: 0x902d}` — which the consumer **deliberately ignores**
     (`crates/kayfabe-fwd/src/lib.rs:5636`, *"Routing confirmation only"*). The class value is carried
     and never validated.
   - `NO_OPERATION` `0x0100`, `SET_NOTIFY_A` `0x0104`, `SET_NOTIFY_B` `0x0108`, `NOTIFY` `0x0110` hit
     `_ => None` (`ga10x.rs:1502`) ⇒ `PushMethod::Opaque` ⇒ `out.opaque += 1`
     (`crates/kayfabe-fwd/src/lib.rs:5774`). **A counter. No name, no fault, no log.**
   - The GR census is worse than silent — it is *class-gated with a bare `continue`*
     (`crates/kayfabe-rt/src/completion_watch.rs:342-344`, `!= AMPERE_COMPUTE_B`), so a `0x902d`
     subchannel reports `operands=0`, **indistinguishable from "the guest named no addresses"**. That
     is this tree's own `no_counter_fired_is_not_no_record_exists` shape, one plane over.

   ⊘ There is no submission-time class allowlist anywhere; `DENIED_CLASSES` (`capability.rs:1562`) is
   the alloc-side list and `FwdFault::NotAnEngine` (`kayfabe-fwd/src/lib.rs:767`) is the
   doorbell-routing one. Neither is consulted by the method decoder.

#### ⇒ The smallest change that makes both allocations explicit

- **Golden image: none.** Already served end to end.
- **Watchdog: one line of intent, not one line of code.** The correct explicit answer today is a
  **`Denial::Refused{name: "VOLTA_CHANNEL_GPFIFO_A", why: …}`** row on `DENIED_CLASSES` for `0xc36f`,
  which upgrades `NotOnAllowlist` (*"nobody has ever seen this"*) to *"we saw it and decided"* — exactly
  the distinction `crates/kayfabe-rmrpc/src/lib.rs:384-408` exists to preserve, and the same treatment
  `0x402c` `NV40_I2C` already got (`4088589`). ⊘ **Do not admit it.** §14.20/§14.22/§16.24.1 each measured
  the watchdog's refusals **non-fatal** (the adapter initialises and `nvidia-smi` enumerates with all
  three refused), so admitting buys zero progress and costs a channel we would then have to execute.
- **The one thing that is genuinely missing is a NAME on the method plane**, and it is missing whether
  or not the watchdog ever runs: `PushMethod::Opaque` should carry the bound class so an unmodelled
  engine class on a subchannel is distinguishable from an unmodelled method on a modelled one. That is
  §1.4's invariant made checkable at runtime rather than by grep.

⚠ **And the meta-finding, which is worth more than any of the above.** This section was commissioned as
open work. `nvkvm-rs: docs/design/execution_plane_increments.md` §16.24.1 (dated **2026-08-09**) already
closed it in writing — *"§14.26 already closed the question the brief was re-asking … it was answered on
2026-08-08 and re-queued for a day afterwards because a `file:line` was read without its caller"* — and
records it as the **third** instance of `read_the_caller_not_the_id`. This is the **fourth**, and the
second in a *brief*. ⇒ The rule that would have caught it is this tree's own: **`git grep` the closing
section, not the failing `file:line`, before commissioning a residual.**

---

## 2. Q2 — the C on the guest-userspace GR path

### 2.1 ⊘ READ `mode2_channel_ownership_split.md` §5b FIRST. It is the answer, and it is dated 2026-08-10.

That page's four corrections (C1–C4) are the settled result and this page does not restate them. The
one-line summary, in its words: **C4 — "For userspace channels the C is a DOUBLE WRITER in the shipped
config."** In particular it warns that `nvkvm_gpu_emul.c:4265`'s *"User-CE / GR channels are EXCLUDED
(the host executes + releases those for real)"* scopes **only the kernel `finishPayload` forge** and is
**not** a statement that userspace completions were left to the host.

### 2.2 The brief's premise, graded

> *"a kernel launch costs ZERO RM ioctls ⇒ for the C to have run LLMs at parity, its userspace GR path
> must have been passthrough."*

**Half right, and the half that is wrong is the half that matters.**

**✔ RIGHT — the C did not re-issue GR work. The host GPU executed the guest's literal command bytes.**
The mechanism, read at source:

1. `nvkvm_m2_exec_doorbell` (`nvkvm_gpu_emul.c:8927`) walks the channel's new GPFIFO entries and, for
   each pushbuffer page, calls `nvkvm_m2_back_and_map(s, cc, pbbase, pbphys, sz, true, "pushbuf")`
   (`:9083`).
2. `nvkvm_m2_back_and_map_inner` (`:7903`) allocates a **real host vidmem object**, `map_dma`-FIXEDs it
   into the host VAS **at the guest's own VA** (`:7932`), copies the guest's current bytes in (`:7942`),
   and registers the guest FB range in `m2_fbback[]` (`:7946-7949`).
3. `m2_fbback` is consulted **first** by the FB accessors (`:1270-1274`), so from that moment **every
   guest FB access to those pages is served out of the host GPU object**. It is an **alias**, not a
   snapshot: the guest's later pushbuffer writes land in host vidmem directly.
4. `:9160` — `stl_le_p((uint8_t *)s->m2_usermode_qva + 0x90, tok)` rings the **host** channel's
   work-submit token. The host GPU then fetches and executes the guest's bytes.
5. The GR method stream is **never re-encoded**. `:6053-6055` states it: *"we honor the EXPLICIT release
   here … **WITHOUT running the GR/compute methods** — per the Phase-B design **we never emulate GR**,
   we only signal completion."* The CPU parser's `switch` (`:6187-6547`) decodes CE-class methods
   (`0x400/0x404/0x408/0x40C/0x418/0x41C/0x300`…) and semaphore methods only.

⇒ This is the precedent the brief hoped for, and it is real: **guest-userspace GR ran as an unparsed
byte stream on real hardware.** It is also *why* it was correct — `mode2_channel_ownership_split.md` §5c
gives the reason (the payload, flush scope, interrupt arming and structure size are all **literals** in
the guest's own bytes).

**✘ WRONG — it was not passthrough, in three specific ways, all in the shipped config.**

- ⊘ **The doorbell was trapped, not passed through.** Every BAR is `memory_region_init_io`
  (`:9794, :9807, :9818`) — full MMIO trapping. The guest's work-submit token write lands at
  `NVKVM_VF_DOORBELL = 0x00BB0090` (`mode2_regs_ga10x.h:98`), handled at `:3835`, and the value written
  is used **only for gated logging** (`:3841-3856`). The ring is a **GP_PUT-driven scan over every
  registered channel** (`:9050-9166`), not a token translation. `mode2_doorbell_chid.md:395` records the
  measurement behind that: *"DECISION: doorbell pass-through is INCORRECT."*
- ⊘ **The CPU also wrote GR completion semaphores.** `nvkvm_chan_execute` is called for **every**
  channel with no client filter (`:4107`), and its parser honours `SET_REPORT_SEMAPHORE_A/B/C/D`
  (`:6544-6547`) — the **compute-class** completion — writing it via `nvkvm_chan_sem_wr32`. Suppression
  exists only for **user-CE** channels and only under `m2hostsem` (`:5554`:
  `hostonly = m2exec && m2hostsem && is_user_ce`), which was **`0` in the green run**. ⇒ Two writers on
  the GR completion, by construction.
  ★ **But measure before concluding:** `mode2_2nd_context_hang.md:1889-1891` records **zero** emulator
  semaphore writes to the compute pool addresses (`0x2044xxxxx`) for a whole run — the pool *was*
  written by the host GPU. The likely reason is `:5839-5846`: a **user compute client's channel never
  takes the blind VAS probe**, so when `chan_pdb` does not resolve the parse bails before reaching any
  method. ⇒ **The double-writer path is present and default-live; on the measured workload it fired
  zero times on that address.** Both halves are true and neither should be quoted alone.
- ⊘ **USERD was trapped too.** Per §5b/C1 there is no KVM memslot in the Mode-2 data plane at all;
  `nvkvm_mmap_host.c`'s memslot machinery is never called from the device.

### 2.3 ⇒ The correct one-line characterisation of the C

**Content passthrough, doorbell and completion emulated.** The C forwarded the guest's *memory* and its
*command bytes* verbatim to real hardware, and trapped/mediated everything that names *when* — the
doorbell and the completion. That is genuinely the two-axis split in embryo, and it is a real precedent
for the userspace GR arm — but it is **not** an existence proof for an untrapped USERD page or
host-owned completions. Per §5b: *"Genuine passthrough … is new work, not a port. It must simply not be
costed as transcription."*

### 2.4 The flag axis — checked, because this campaign has been burned by it

**[measured]** `cap3_matmul_forwarding.rec.zst`'s self-describing header, written by the device at
realize time:

```
props: trace=1 m2fwd=1 m2exec=1 m2hostsem=0 m2cefwd=0 m2cexec=0 m2opaque=0 m2trace=0 m2romregs=0
extra: Capture 3: cuCtxCreate -> 2048^2 matmul
```

corroborated by `traces/mode2_c_reference/README.md:29`. **Execution was ON.** `m2fwd` and `m2exec`
default `true` (`nvkvm_gpu_emul.c:9928-9929`) and **no script anywhere turns them off** except the
deliberate hermetic capture (`scripts/mode2_diag/rec_capture.sh:58`). The "green run had execution off"
failure mode **does not apply here**.

Three traps found while checking, all worth carrying:

- ⚠ **A stale comment inverts the default.** `:3884` reads *"gated m2exec, **default off**"*; `:9929`
  is `DEFINE_PROP_BOOL("m2exec", …, true)`. Anyone auditing by grepping for "default off" would wrongly
  quarantine a live path. (Already recorded at `mode2_fb_crossing_question.md:206-216`.)
- ⚠ **`:+` footgun.** `run_mode2_vm.sh:113` uses `${NVKVM_M2HOSTSEM:+,m2hostsem=on}` — so
  `NVKVM_M2HOSTSEM=0` **enables** it. Any non-empty value does.
- ⚠ **`m2fwd` has a silent runtime kill path** — set `false` at `:6690` and `:6701` if the host isolate
  can't spawn or `/dev/nvidia*` can't be opened. A run started with `m2fwd=on` can end with forwarding
  off, and the **only** witness is a `qemu_log` line. `m2exec` has no such path.
- ⊘ **Two different green vectors exist for "the matmul".** `cap3` recorded `m2cefwd=0`; the
  2026-07-29 ladder boots (`BENCH_REBUILD_NOTES.md:486,542,572`; `bench_boot.sh:56`) used
  `NVKVM_M2CEFWD=1`. **Both produced `bad=0 maxerr=0`.** `how_the_c_passed_the_gr_wall.md:121-124`
  treats `m2cefwd=0` as *the* green vector and is therefore over-narrow.

⚠ **And on "LLM parity", which is where the brief's premise came from:** the ~20 tok/s 7B claim is
**Mode-1** (`tests/integration/run_llm_7b.sh:1-9` — it requires `nvkvm-guest.ko`). The Mode-2 LLM claim
is the **49.9 vs 47.5 tok/s** bare-metal number (`docs/MILESTONES.md:12-13`), and its evidence is
**documentary only** — commit `6942048` is docs-only, 1 file, 37 insertions, and there is **no committed
log or trace** of that run, unlike the matmul. Mode-2 LLM harnesses do exist and were demonstrably run
(`scripts/mode2_diag/llm_run_guest.sh`, driven by ~16 host scripts; the `m568: 91932/91960` figure at
`nvkvm_gpu_emul.c:576-578` cannot be obtained without running it). ⇒ Treat the two claims as
independent with very different evidence strength, and never let the Mode-1 number stand in for Mode-2.

---

## 3. What follows for the design

1. **The two-axis split holds**, and §1.4 is now its proof rather than its assumption: the emulated
   (kernel) axis is `{CE methods} ∪ {five NV902D_ methods}`, with **no MME anywhere in kernel code**.
   Record this as a *checkable invariant*, not a belief — re-run both greps in §1.4 on every `ogkm`
   version bump, because the answer is a property of a version, not of the architecture.
2. **The emulated axis must nonetheless model a GR-engine channel**: `FERMI_TWOD_A` `SET_OBJECT` +
   `NV902D_NOTIFY`, and a `GR0` channel carrying a `3D`/`COMPUTE` object that never receives work.
3. **Do not use the GA106 bench to conclude "no kernel GR channel"** — Bug 4208224 is TU102/104/106 only
   and is invisible there, while Turing is inside the support floor.
4. **Golden-image init is an ALLOCATION event, not a submission event.** Getting it right means the GR
   object allocation must produce a real golden context on the host; it does **not** mean decoding a
   pushbuffer. This is a materially easier problem than the brief assumed.
5. ★ **Both allocations are already handled at `nvkvm-rs@425c450` — see §1.7 before opening any of this
   as work.** Golden image: served end to end since 2026-08-08. Watchdog: refused by name on three
   gates, measured non-fatal. The only genuinely open item §1.7 found is on the **method** plane, and it
   is independent of the watchdog: `PushMethod::Opaque` carries no class, so an unmodelled *engine
   class* on a subchannel is indistinguishable from an unmodelled *method*.

## See also

- `mode2_channel_ownership_split.md` — the owner ruling this page serves; **§5b is the Q2 answer** and
  **§5c is why content passthrough is correct by construction**.
- `mode2_doorbell_chid.md` — the June ruling; `:395` is the measured *"doorbell pass-through is
  INCORRECT"*.
- `how_the_c_passed_the_gr_wall.md` — the M5.38 single-writer fix; ⚠ its §5 green-vector claim is
  over-narrow (§2.4 above).
- `is_passthrough_the_only_correct_route.md`, `mode2_guest_ram_crossing.md` — the passthrough cost side.

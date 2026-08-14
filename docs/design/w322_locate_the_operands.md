# w322 — LOCATE THE OPERANDS

**STATUS: LIVE — branched from master `53d6375c`. §1–§5 were committed BEFORE the first boot
of this rung; results are appended below §6 and the prediction is graded against what is
written here.**
Artifacts: `traces/w322_operands/`.

> ⊘ **`git diff master -- crates/` is EMPTY on this branch.** Every edit is under
> `scripts/bench/`, and every new behaviour in the workload is behind an environment variable
> that defaults to the previous behaviour. ⇒ **this is a measurement-only rung**, and the
> correctness ladder below exists to show the *measurement* is sound, not to clear a change it
> cannot cause.

---

## 1. The question, and what w320 left standing

w320 established that `cuCtxSynchronize` **is the kernel running** — submit is flat at
3.48 → 4.37 ms across a 4096× range of arithmetic while sync moves 936× — and that against
native on the same GA106 the remaining overhead is **21.7–80.9× on the compute**. It then
reproduced a slowdown of the same order *by construction*, natively, with no guest anywhere,
by moving the operands into `cuMemHostAlloc(DEVICEMAP)` pinned host memory: **10.9–66.3×**.

⊘ **And its control OVERSHOT.** At N ≥ 1024 the native host-memory arm was **slower than our
guest** (129.097 ms vs 59.786; 1480.366 vs 908.966). w320 read that, correctly, as
*sufficiency is not identity*: placement costs this much, but that is not evidence our buffers
are placed this way.

So: **where are the guest's `cuMemAlloc` buffers physically backed, and at what bandwidth does
the host GR engine reach them?**

---

## 2. ★★★ WHAT THE SOURCE ALREADY SETTLES — read before spending a boot

Two read-only traces, taken before any measurement, answer the *location* half from the code
and pin the vocabulary. Both are cited to file:line so they can be checked rather than
believed.

### 2.1 The chain, hop by hop

| hop | what it is | where |
|---|---|---|
| guest `cuMemAlloc` | the guest's own CPU-RM does its heap management; **we are not present at the allocation.** No `NV_ESC_RM_ALLOC` / `NVOS32_ATTR_LOCATION_*` decoder exists anywhere in this tree | — |
| the guest's GMMU | the buffer is mapped with **2 MiB `Aperture::Vidmem` PTEs**. This is the *only* authority on where the guest thinks it put them, and it is read by the page-table walk, not asked of RM | `crates/kayfabe-mmu/src/walker.rs:1041`, `crates/kayfabe-mmu/src/lib.rs:623` |
| our emulated FB | `SparseFb`, a `HashMap<pageframe, Box<[u8;4096]>>` in the **VMM process heap** — neither guest RAM nor host VRAM. Advertised size 12 GiB | `crates/kayfabe-device/src/fbwin.rs:802-835`, `crates/kayfabe-device/src/ga10x.rs:188` |
| the join | the publication pass selects rows that are `Aperture::Vidmem` and 64 KiB-granular, and replaces the local pages with the isolate's mapping | `crates/kayfabe-rt/src/device.rs:3618-3665`, `crates/kayfabe-device/src/fbwin.rs:1070` |
| **the host object** | `memfd_create(…, MFD_ALLOW_SEALING)` — **no `MFD_HUGETLB`** — mapped `CachePolicy::WriteBack` at `HostPageSize::query()`, then described to host RM | `crates/kayfabe-linux-raw/src/host_fd_unsafe.rs:125`, `crates/kayfabe-isolate-host/src/rm.rs:5104-5117` |
| **the RM class** | `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` (0x0071) with `LOCATION_PCI \| PHYSICALITY_NONCONTIGUOUS \| COHERENCY_CACHED \| MAPPING_NO_MAP` = **`0x40001010`** | `crates/kayfabe-isolate-host/src/rm.rs:2411-2415` |
| the mapping | `NV_ESC_RM_MAP_MEMORY_DMA` / `NVOS46`, issued **twice** (guest `Vas` + isolate `ExecutorVas`) at the same VA | `crates/kayfabe-isolate-host/src/rm.rs:5133`, `:4267-4294`, `:2110-2145` |

**Address vocabulary at each hop** — the tree's rule is that no real PHYS exists, only GPGA or
GPA, and this chain honours it: guest **VA** → the guest's own GMMU leaf, an **fb-phys /
GPGA** inside the emulated framebuffer → the join, a **host process VA** over a `memfd` →
an RM memdesc over those host pages, whose physical addresses **RM owns and we never see** →
`NVOS46` FIXED at the **same numeric GPU VA** in two host VA spaces. There is no host PHYS
anywhere in our code.

### 2.2 ★★★★★ TWO THINGS THIS RETIRES BEFORE THE FIRST BOOT

**(i) The w290 prior does not describe this workload.** *"99.4 % of the table is guest RAM"*
(`guest_ram=16328`) was measured on **cup2**, a CE round trip. On the CUDA process's own GR
VAS during a cup8 matmul the census reads, in already-committed logs:

```
[proc=2 pdb=0x201000 … total=18309 already_host=18271 already_pinned=0 guest_ram=0 …]   (w318on)
[proc=2 pdb=0x201000 … total=18326 already_host=18295 already_pinned=0 guest_ram=0 …]   (w320sizes / w320q1)
```

`guest_ram=0`. ⇒ **the operands are not guest RAM**, and quoting w290 at this workload would
have been the "a ruling's date and architecture are part of the citation" error.

**(ii) There is no VRAM chain with a caller.** `NV01_MEMORY_LOCAL_USER` exists
(`alloc_device_local`, `rm.rs:2337-2351`) but serves only rings, USERD and semaphores; the
vidmem *leaf* chain `FbLeafBacking::Vidmem` is explicitly caller-less and superseded
(`crates/kayfabe-fwd/src/lib.rs:2255-2262`). The divergence is named in our own source
(`rm.rs:5048-5054`): *"Sysmem, not vidmem, and it is a NAMED divergence… The engine reaches
this leaf over PCIe instead of out of local framebuffer."*

### 2.3 ⊘ WHAT THE SOURCE CANNOT SAY, AND WHY IT STILL NEEDS MEASURING

- **What bandwidth the engine actually achieves.** Nothing in the code is a number.
- **Whether the GMMU PTEs are GPU-L2 cacheable** (`vol`), and **what leaf page size RM chose**.
  We set neither: the only `NVOS46_*` constant that exists in the whole workspace is
  `DMA_OFFSET_FIXED_TRUE` (`crates/kayfabe-abi/src/bringup.rs:549`) — no `CACHE_SNOOP`, no
  `PAGE_SIZE`, no `kind_override` — and the host VA space is allocated with
  `NvVaspaceAllocationParameters::default()` (`rm.rs:4162-4168`). Both are **RM defaults, and
  the code cannot tell you what RM defaulted to.**
- ★★★ **And here is the fact that most constrains the "we beat sysmem" gap**: our flag word
  `0x40001010` was *captured from a real host CUDA `cuMemHostAlloc` on 580.159.04*
  (`crates/kayfabe-abi/src/bringup.rs:530-534`). **Our descriptor's location and coherency bits
  are byte-identical to the ones `cuMemHostAlloc` itself issues.** ⇒ allocation-side
  cacheability is a **dead end** as an explanation of the difference; whatever differs is
  downstream of RM's memdesc→PTE derivation, or is not a placement difference at all.

---

## 3. THE INSTRUMENTS

### 3.1 ★★★★★ The aperture spectrometer — a second kernel, because `mm` cannot answer this

`mm` is the wrong instrument for a bandwidth number, and the reason is arithmetic:

> native VRAM at N=2048 does 2·N³ = 17.2 GFLOP in 22.3 ms = 770 GFLOP/s. If all 2·N³ operand
> loads reached DRAM that would be **68.7 GB in 22.3 ms = 3.08 TB/s**, i.e. **8.5× a GA106's
> ~360 GB/s**. ⇒ `mm`'s runtime is a **cache-hierarchy** number. Dividing its FLOPs by its time
> and calling the result "bandwidth" would be an invented quantity.

`bw` (`scripts/bench/cup8bench.c`, `BW_PTX`) is a single coalesced streaming pass with **no
reuse**: grid-stride, 262 144 threads, one accumulate per element. Over a buffer far larger
than L2 the hit rate goes to zero by construction, so bytes ÷ time **is** the aperture's
bandwidth. Swept over buffer size it is a spectrometer, not a point:

- **small buffer (fits L2)** → L2 bandwidth, the *same* whatever the backing store is;
- **large buffer (≫ L2)** → the backing store. VRAM ~360 GB/s vs PCIe gen3 ×16 ~12.6 GB/s.

★★★ Those endpoints are ~28× apart, so the large-buffer plateau **discriminates the aperture
directly** — and it is read against **the same kernel's own native VRAM and native sysmem
plateaus on the same GPU**, so the verdict is an interpolation between two *measured*
endpoints rather than a comparison against a datasheet.

⚠ **The brief names the trap this is designed around**: 28× brackets the measured 21.7–80.9×
spread, and a magnitude that fits belongs to the instrument until proven otherwise. **The
plateau is not the ratio.** It is a different quantity, measured by a different kernel, with
its own controls below.

Controls built into it:
- bytes read per launch are held ~constant across the sweep (the pass repeats `R` times), so
  neither end is dominated by our ~4 ms submit floor;
- bandwidth is computed from **`sync`**, not from the whole launch — w320 is why — and the
  `incl_submit` figure is printed beside it so that choice is checkable;
- every element is 1.0f and the element count is rounded to a multiple of the thread count, so
  the expected output is **one constant** (256) over all 262 144 outputs, exact in fp32; the
  output is **poisoned before every launch**, so `BENCH_NOLAUNCH` inverts to `bad > 0`;
- every failure path prints a **named refusal** and a row marked `UNMEASURED`. No mode, size or
  neighbour is ever substituted for a row that did not run.

### 3.2 ★★★ The native ruler — and the pessimality check the brief asks for FIRST

`scripts/bench/w322_native.sh` runs five placements natively, no guest, no QEMU:
`vram`, `hostalloc` (w320's control, unchanged), `hostalloc_wc`, `hostreg` (2 MiB-aligned anon
+ `MADV_HUGEPAGE` + `cuMemHostRegister`), `managed_cpu` (`cuMemAllocManaged` + preferred
location CPU + accessed-by GPU).

**The reading**: if `hostalloc`, `hostreg` and `managed_cpu` land within a small factor of one
another, "pinned sysmem over PCIe" is a **narrow band** and w320's control was representative.
If one is much faster, **the control was pessimal** and w320's overshoot loses its force.
`hostalloc_wc` is a **directional known-positive for the instrument**: write-combining must
come out *worse* for a read-only kernel, and if it does not, the sweep is not resolving
placement at all.

### 3.3 ★★★ The host framebuffer counter — an aperture check that owes nothing to our own bookkeeping

The sweep allocates 1 → 256 MiB in sequence, one buffer live at a time. Three hypotheses,
three **different** signatures on counters we do not maintain:

| if the bytes are | `nvidia-smi memory.used` | QEMU/isolate RSS |
|---|---|---|
| host VRAM | steps by ~256 MiB and back | flat |
| host sysmem (our arena) | flat | grows by ~256 MiB |
| guest RAM pinned through | flat | flat (2 GiB memfd is preallocated) |

⚠ It samples at **1 Hz**, so a row completing inside a second can be missed entirely — and a
missed step reads exactly like a step that never happened. That is why the largest row is also
the slowest, why the raw samples are kept rather than only their maximum, and why an absent
sampler file is reported as **UNMEASURED** rather than as a delta of zero.

### 3.4 ⊘ The instrument I am NOT building, and what would be needed if the answer needs it

`GET_PDE_INFO` (`NV0080_CTRL_CMD_DMA_GET_PDE_INFO`, 0x801809) is **already built and callable
unprivileged** — `crates/kayfabe-isolate-host/src/rm.rs:5784-5860`, driven by
`kayfabe-rm-ladder --r33` arm 6 — and returns `pageSize`, `pteCacheAttrib` and aperture for the
PDE covering a VA, taking `hVASpace` as a parameter so it can be asked about *our* space. It is
the instrument for the page-size hypothesis in §4.3. ⊘ The leaf-PTE oracle `GET_PTE_INFO`
(0x801801) is **unusable on a production driver** — measured `NV_ERR_TEST_ONLY_CODE_NOT_ENABLED`
(126) for every address (`crates/kayfabe-abi/src/submit.rs:4253-4272`).
It is not in this rung's critical path: the bandwidth plateau answers *where*, and the page
size only refines *why the gap to `cuMemHostAlloc` has the sign it has*.

---

## 4. ★★★★★ PRE-REGISTERED, BEFORE THE FIRST BOOT

**I predict (A): the operands are in host sysmem, and the bandwidth accounts for the gap.**
§2 makes this close to a foregone conclusion on the *location* half — the code has one chain
and it is `LOCATION_PCI` — so the honest statement of what is still open is narrower and is
where the rung can still be surprised:

1. **The guest's large-buffer plateau lands in the PCIe band**, i.e. within a factor of ~2 of
   the native `hostalloc` plateau and **far** (≥10×) below the native `vram` plateau.
2. **`nvidia-smi memory.used` does NOT step by the sweep's largest buffer size.**
3. **The sysmem family is narrow**: `hostalloc`, `hostreg`, `managed_cpu` within ~2× of each
   other at the largest size. ⇒ w320's control was *not* badly pessimal, and the 1.6–2.2×
   by which our guest beat it is a **within-sysmem** difference, not evidence of a different
   aperture.
4. ⊘ **The one I most expect to be wrong is 3.** If `managed_cpu` or `hostreg` is much faster
   than `hostalloc`, then page size dominates inside the sysmem family, w320's control was
   pessimal, and the "we beat sysmem" observation dissolves rather than needing an explanation.

**Grading the letters** (the brief's, restated so the grading is not chosen afterwards):

- **(A)** sysmem, and the bandwidth accounts for the gap ⇒ **the target**. Then state what it
  would take to place operands in VRAM and what that costs, ⚠ including how the guest's view
  survives when VRAM is not guest-addressable by construction.
- **(B)** VRAM, and the gap is elsewhere ⇒ **the more important result**; then attribute the
  22–81× to something else and name it.
- **(C)** mixed ⇒ report the split and which side dominates the time.
- **(D)** the aperture cannot be determined from here ⇒ name the missing instrument. ⊘ Do not
  guess it from a name or a handle.

⚠ **Additional refusals, pre-committed:**
- A bandwidth figure is quoted **only** from a row whose `bad = 0`. A fast row that computed
  the wrong answer is not a fast row.
- The guest ÷ native comparison is taken **at the same buffer size and the same target byte
  count**, or not at all.
- No two-term model is fitted to anything here. w320 §5.3 returned `b = 20.979` — within 11 %
  of the hypothesised value — with a 40 ms RMS residual on 6–500 ms data and systematically
  curved residuals. **≥3 points, every residual printed, re-fit without the largest**, or no
  fit.

---

## 5. What could make this rung wrong

- ⚠ **The bench box is itself a KVM guest**, so our guest runs at L2. That affects MMIO exit
  cost, not PCIe DMA bandwidth — but the native arms run on the same L1 box, so the *ratio* is
  taken between two things sharing that condition.
- ⚠ **`n = 1 boot` per guest arm.** The discriminating quantity is a ~28× separation between
  two plateaus; w315 measured boot-to-boot scatter of ~8 ms on a ~100 ms quantity. A scatter
  that small cannot move a 28× verdict, but every individual figure carries one boot and is
  quoted that way.
- ⊘ **The spectrometer measures the aperture of a buffer allocated by `bw`'s own path, which is
  the same `cuMemAlloc` the matmul uses — but it is not literally the matmul's buffer.** The
  identity is by construction (same call, same size class, same publication path), not by
  observation of the matmul's own pointers. If the two ever diverged, this would report the
  spectrometer's buffer.

---

## 6. RESULTS

*(nothing above this line was written with a result in hand)*

### 6.1 ★★★★★ THE ONE-LINE ANSWER — pre-registered outcome **(A)**

**The operands are HOST SYSMEM — `memfd` pages inside the unprivileged isolate, reached by the
host GR engine over PCIe gen3 ×16 — and the aperture accounts for the DOMINANT factor in the
22–81×, but not all of it.** The measured endpoints on this GA106, same kernel, same binary:

| | read bandwidth (n=3, 256 MiB, single pass) | scatter |
|---|---|---|
| **host VRAM** (`cuMemAlloc`, native) | **313.5 GB/s** | ±0.04 % |
| **host sysmem, 2 MiB pages** (`hostreg` / `managed_cpu`, native) | **12.33 GB/s** | ±0.2 % |
| PCIe gen3 ×16, this link (`pcie.link.gen.current=3`, `width=16`) | ~12.6 GB/s theoretical | — |

⇒ the sysmem arms **saturate the link**, and the aperture ratio is **25.4×**.

### 6.2 ⊘⊘⊘ w320's CONTROL WAS PESSIMAL *AND* UNSTABLE — and the anomaly that framed this brief DISSOLVES

The brief's central gap was *"our guest is FASTER than native-with-host-memory at N ≥ 1024, so
our buffers are not plain pinned sysmem."* It is not our buffers that were unusual. **It was the
control.**

`cuMemHostAlloc(DEVICEMAP)` — w320's only non-VRAM placement — measured, on the same GPU, same
kernel, same binary, 256 MiB, single pass, **five times**:

```
3.697   9.456   11.886   10.887   9.613  GB/s      range 3.70 – 11.89  =  3.2x
```

against, in the same runs:

```
vram         313.567  313.599  313.376  313.475  313.645   (0.04 %)
hostreg       12.318   12.325   12.350   12.321   12.315   (0.2 %)
managed_cpu   12.353   12.308   12.320   12.312   12.319   (0.2 %)
```

★★★ **Three placements reproduce to 0.2 % and one swings 3.2 ×.** `cuMemHostAlloc` is the odd
one out, it sits **10–70 % below the link it is running on**, and it **degrades with buffer
size** (12.07 → 9.46 GB/s from 4 to 256 MiB) while the 2 MiB-page arms stay flat. That is the
signature of GPU TLB pressure over small pages, and it is a property of *that allocator*, not
of sysmem.

**Re-run of w320 §5.5 against an honest control** — `mm`, native, same hour, `bad=0 maxerr=0`
in every row:

| N | native VRAM | native **`hostreg`** | native `hostalloc` (w320's) | our guest (w320) | guest ÷ hostreg |
|---|---|---|---|---|---|
| 128 | 0.012 | **0.130** | 0.133 | 0.971 | **7.5× slower** |
| 512 | 0.359 | **6.214** | 6.070 | 22.734 | **3.7× slower** |
| 1024 | 2.844 | **43.617** | 116.340 | 59.786 | **1.37× slower** |
| 2048 | 22.873 | **338.524** | 1527.352 | 908.966 | **2.69× slower** |

⊘ **The overshoot is gone.** w320 read *"guest is 2.2× FASTER at N=1024 and 1.6× FASTER at
N=2048"*; against a control that saturates PCIe the guest is **slower at every size**. Its
`hostalloc` column is the one that moved — 116.3 → 43.6 and 1527.4 → 338.5, i.e. **2.7× and
4.5×** — and w320's inference rested entirely on that column.

★★ **Nothing in w320 is refuted except the inference the brief already flagged as its own.**
The submit/sync split, the 936× duration scaling, `nvcsw = 0`, and the guest ÷ native-VRAM
ratios are untouched: this rung did not re-measure them and does not disagree with them.

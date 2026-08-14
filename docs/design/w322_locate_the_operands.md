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

### 6.3 ★★★★★ THE BANDWIDTH — the guest's operands are read at **2.56 GB/s**

`w322bw`, one boot, gate ON, `BENCH_BW_REPS=1` (single pass — no reuse is available at any
size), `bad=0` in every measured row:

| buffer | guest `sync_med` | **guest read** | native VRAM | native sysmem (2 MiB pages) |
|---|---|---|---|---|
| 4 MiB | 1.856 ms | **2.259 GB/s** | 213.1 | 11.98 |
| 16 MiB | 6.561 ms | **2.557 GB/s** | 269.6 | 12.23 |
| 32 MiB | ⊘ UNMEASURED — fill refused at byte offset `0x800000`, rc=719 | | 290.0 | 12.28 |
| 64 MiB | ⊘ UNMEASURED — alloc refused rc=719 (context already dead) | | 303.5 | 12.30 |
| 128 MiB | ⊘ UNMEASURED — alloc refused rc=719 | | — | 12.32 |

★★★★★ **2.56 GB/s is 123× below this GPU's VRAM and 4.8× below a PCIe-saturating sysmem
read.** Nothing that streams at 2.56 GB/s is being read out of a 313 GB/s aperture. Combined
with §2's source trace — one chain, `LOCATION_PCI`, and no VRAM chain with a caller — the
location is settled from two independent directions.

⚠ **AND HERE IS THE FIT I AM NOT MAKING.** Two points give
`sync = 0.392·MiB + 0.288 ms` ⇒ a fixed term of 0.29 ms and a marginal 2.67 GB/s. **Two points
fit a two-parameter model exactly and leave no residual that could disagree** — this is w320
§5.3's `b = 20.979` in miniature, and the brief pre-registers ≥3 points, every residual
printed, and a re-fit without the largest. **I have two, I am not banking the fit, and the
reason I have two is §6.5.** ⊘ Naming why the other three rows are missing is worth more than
the line I could have drawn through the two I have.

### 6.4 ⊘ THE HOST FRAMEBUFFER COUNTER — INCONCLUSIVE, and that is a statement about the instrument

§3.3 pre-registered three distinct signatures. What the 1 Hz sampler actually recorded over
58 samples of the `bw` boot:

```
fb_used_MiB, distinct values:   0 (x12)   1 (x25)   5 (x14)   26 (x1)   35 (x1)   36 (x5)
qemu RSS:                       61 MiB  ->  1.27 GiB
```

⊘ **This cannot decide the question as instrumented, and I am not going to make it.** The
framebuffer steps (+21, +9, +1 MiB) are *the same order as the buffers being allocated*
(4/16/32 MiB), the sampler's 1 s period is *the same order as a row's duration*, and no marker
ties a sample to a row. A reader wanting the VRAM answer could read the 5 → 26 MiB step as the
16 MiB buffer landing in the framebuffer; a reader wanting sysmem could read it as the
isolate's channels and rings, which §2 says really are `NV01_MEMORY_LOCAL_USER`. **Both
readings fit, so the instrument decides nothing.**

★ The RSS side is equally soft: QEMU grew by 1.2 GiB, but guest RAM is a 2 GiB memfd faulting
in, so a 16 MiB arena is inside the noise.

**What would fix it**, if anyone needs this instrument later: have the workload write a marker
into the FB log at each row boundary (a named file the sampler tails), and sample the isolate's
own RM allocations rather than the whole device — the process-level `used_memory` column read
`1689016,0` on the 7 samples where it appeared, i.e. **0 MiB attributed to the compute process**,
which is suggestive in the right direction but is one column on seven samples and is not
enough to carry a verdict.

⇒ **The bandwidth in §6.3 is the arbiter here, and the FB counter is recorded as an
instrument that did not resolve.** ⊘ It is written up rather than dropped because a
pre-registered instrument that fails silently is how a rung ends up with three claims and two
measurements.

### 6.5 ★★★ A CEILING NOBODY HAD PROBED — the compute path cannot back a 32 MiB allocation

Every row at and above 32 MiB failed, in both boots, at the same place: a device-side fill
refused with `rc=719` (`CUDA_ERROR_LAUNCH_FAILED`), which then took the CUDA context down so
the larger rows could not even allocate. With the fill chunked to 8 MiB the refusal names its
offset:

```
BW_FILL_FAIL mib=32 at_element=2097152 of 8388608 (byte offset 0x800000) rc=0/719
```

— the **first** 8 MiB chunk was filled; the **second** was not. And the device log names the
cause on the same boot:

```
DRAIN-TIMING max_drain_us=48908 disposed=16 residue=0 turns=4 budget_hit=true
  ⇒ Budget: 40000 us (1% of scrubberDestruct's 4000000 us) ...        (x5 in this boot)
```

⇒ **`budget_hit=true` — this is w319's drain-budget truncation, reached by allocation SIZE.**
The publication pass ran out of its 40 ms budget with leaves unpublished, the GR engine then
read a VA whose leaf was never backed, and the guest saw a dead channel rather than a named
refusal.

★★ **The load-bearing part is where the cliff sits.** cup8 at N=2048 allocates **three
16.78 MiB buffers** — under the threshold, one buffer at a time. This rung is the first thing
to ask for a **single** allocation above it. ⇒ every green in this campaign has been running
just under a ceiling nobody had measured, and *"it works at N=2048"* is not evidence it works
at N=4096.
⊘ This is a **pre-existing defect, not something this rung introduced**: `crates/` is
byte-identical to master, and the budget and its constant are master's.

### 6.6 ★★★★★ THE NUMBERS THAT CLOSE — and the residual that is NOT placement

Guest headline, **n = 3 boots**, 16 MiB, single pass, `bad=0` in every row:

```
2.557   2.481   2.506  GB/s      median 2.506,  range +/- 1.5 %
```

⚠ The middle figure is from the arm named `bwneg`, which — see §6.7 — **ran the measurement
workload** because its arming was dropped. It is a valid third measurement of the *bandwidth*
precisely because it was not a negative control; it is quoted here and disqualified there, and
those are the same fact read twice.

⊘ **And the matmul figure below is THIS RUNG'S, not w320's.** A `sizes` arm was re-run in the
same hour as the native arms, on the same binary: `0.948 / 23.467 / 56.973 / 934.475 ms` at
N = 128/512/1024/2048, `bad=0 maxerr=0`, `SIZES_DONE=4`, `Xid=0`. That reproduces w320's
`0.971 / 22.734 / 59.786 / 908.966` **to within 2.4–4.7 % at every size**, so w320's curve is
confirmed rather than assumed — and the composition below uses only same-hour numbers, so no
cross-session drift can hide in it.

Composing the aperture with the same-hour matmul, all on the same GA106, all `bad=0 maxerr=0`:

| quantity | value | how obtained |
|---|---|---|
| native VRAM read | **313.5 GB/s** | n=3, ±0.04 % |
| native sysmem read, 2 MiB pages | **12.33 GB/s** | n=3, ±0.2 %, = PCIe gen3 ×16 saturated |
| **our guest's operand read** | **2.51 GB/s** | n=3 boots, ±1.5 % |
| guest ÷ VRAM | **125×** | |
| guest ÷ link-saturating sysmem | **4.9×** | |

And on the matmul at N=2048, the two factors multiply out:

```
guest 934.475 ms  /  native VRAM  22.873 ms  =  40.85 x       (the thing being explained)
native hostreg 338.524 / native VRAM 22.873  =  14.80 x       <- PLACEMENT: sysmem vs VRAM
guest 934.475     / native hostreg 338.524   =   2.76 x       <- RESIDUAL: our sysmem vs good sysmem
                                       14.80 x 2.76 = 40.85   <- closes exactly
```

★★★ **So the 22–81× splits into two named, separately measured factors**, and neither is the
submit path:

- **PLACEMENT — 14.8× at N=2048.** Our operands are in sysmem and not in VRAM. This is the
  larger factor and it is the one the roadmap turns on.
- **APERTURE QUALITY — 2.8× at N=2048 on the matmul, 4.9× on the raw stream.** Our sysmem
  reaches only 20 % of a PCIe link that `hostreg` and `managed_cpu` saturate. **This is a
  separate defect from placement, it is ours, and nobody had measured it.**

⊘ The two ratios differ (2.76 on `mm`, 4.92 on `bw`) and that is expected rather than a
discrepancy: `mm` is served largely out of L1/L2 (§3.1's arithmetic), so only its miss traffic
is exposed to the aperture. **The streaming number is the aperture; the matmul number is what
the aperture costs THIS workload.** Quoting either as the other would be wrong.

### 6.8 ★★★★★ WHAT THE FIX WOULD COST — two of them, and the cheap one was not on anyone's list

**FIX 1 — APERTURE QUALITY: make our sysmem saturate the link. Worth ~4.9×.**

Our two host backings both hand RM **4 KiB-granular** sysmem:

- the FB-leaf chain mints a `memfd` with **no `MFD_HUGETLB`** and maps it at
  `HostPageSize::query()` = 4 KiB (`crates/kayfabe-isolate-host/src/rm.rs:5104-5113`,
  `crates/kayfabe-linux-raw/src/host_fd_unsafe.rs:125`), then describes it
  `PHYSICALITY_NONCONTIGUOUS`;
- the guest-RAM pin chain is documented as **one `OS_DESCRIPTOR` and one fixed map per 4 KiB
  row** (13 313 rows in 2.7 s on the w308 cup8 boot).

The native ruler shows what that costs, on the same GPU with the same kernel: placements whose
pages are **2 MiB-contiguous** (`hostreg` with `MADV_HUGEPAGE`, `managed_cpu` through UVM) hold
**12.33 GB/s flat from 4 to 256 MiB**, while `cuMemHostAlloc` — the one whose page geometry we
do not control — runs **3.7–11.9 GB/s and degrades with size**. Our guest sits at **2.5 GB/s**
(FB-leaf chain) and **~3.1 GB/s** (its own `cuMemHostAlloc` chain): **both** of our chains are
in the small-page band, and they are within 1.24× of each other despite being completely
different code paths.

⇒ **Hypothesis with a mechanism, a native control at the same magnitude, two independent guest
chains agreeing, and a named next measurement**: RM installs small GPU PTEs over our
non-contiguous 4 KiB sysmem, and the GPU TLB is the limit.
⚠ ⊘ **NOT MEASURED: I have not read the page size back.** The instrument for it exists and is
callable unprivileged — `pde_info` / `NV0080_CTRL_CMD_DMA_GET_PDE_INFO`
(`crates/kayfabe-isolate-host/src/rm.rs:5784-5860`), which takes `hVASpace` so it can be asked
about *our* space, and it is already driven by `kayfabe-rm-ladder --r33` arm 6. **One call on
one operand VA settles it.** ⊘ `GET_PTE_INFO` (0x801801) will not work: measured
`NV_ERR_TEST_ONLY_CODE_NOT_ENABLED` on a production driver.

**Cost if the hypothesis holds:** back the leaf `memfd` with huge pages (`MFD_HUGETLB`, or
`MADV_HUGEPAGE` on the mapping plus a fault-in before `alloc_os_descriptor`) so RM sees
2 MiB-contiguous physical ranges. That is a change to **one allocation site**, it does not move
any address, it does not change what the guest sees, and it is bounded above by 4.9×.
⚠ It is not free: `MFD_HUGETLB` needs a reserved hugepage pool and fails loudly if there is
none — which is the right failure, but it is a deployment requirement, not just a flag.

**FIX 2 — PLACEMENT: put the operands in host VRAM. Worth ~14.8× on top, and it is the harder one.**

The chain exists in skeleton: `alloc_device_local` already allocates
`NV01_MEMORY_LOCAL_USER` with `ATTR_CONTIGUOUS_VIDMEM` (`rm.rs:2337-2351`) and serves rings,
USERD and semaphores from it, and `FbLeafBacking::Vidmem` exists but is **caller-less and
superseded** (`crates/kayfabe-fwd/src/lib.rs:2255-2262`). So this is re-arming a designed path,
not inventing one.

★★★ **How the guest's view survives, which is the part the brief flags** — VRAM is not
guest-addressable, and the answer is that it does not have to be *directly*. Today the join
makes **one** memory: `SparseFb::install_join` replaces the local pages with the isolate's
mapping of the `memfd`, so a guest BAR1/BAR2 access and a host GR read hit the same bytes. The
VRAM version is the same shape with a different `FbJoined`: the isolate maps the
`NV01_MEMORY_LOCAL_USER` object through **host BAR1** into its own address space (an ordinary
unprivileged `NV_ESC_RM_MAP_MEMORY`) and installs *that* as the FB backing. There is still
exactly one memory; the guest still reads and writes at its own addresses; the GR engine reads
local framebuffer.

⚠ **And this is precisely the `BackingBytes::ShadowsGuestMemory` case our own source was
written to warn about.** `w228` measured that chain `placed_as_asked=true` **and blank** — an
engine reading zeros where the guest wrote, `#12` in the C artifact, and **self-concealing**: a
run over a blank object logs identically to a correct one. ⇒ the fix is only correct if both
halves land atomically, and it needs a known-positive that reads back guest-written bytes
through the engine, not a `placed_as_asked=true`.

**What it costs, named honestly:**

1. **A BAR1 window manager.** Host BAR1 is typically 256 MiB; the emulated FB advertises
   **12 GiB** (`crates/kayfabe-device/src/ga10x.rs:188`). Mapping it whole is impossible, so
   joined VRAM leaves need an on-demand, evictable BAR1 window. This is the bulk of the work.
2. **Guest CPU access changes character.** Today a guest write to a joined page lands in host
   DRAM; through BAR1 it becomes an uncached/WC PCIe write, and **BAR1 reads are far worse than
   BAR1 writes**. `cuMemcpyDtoH` of C at N=2048 is 16.78 MiB of reads. ⚠ **UNMEASURED, and it
   could eat a large share of the win** — it is the first thing to measure before committing,
   and the `memtype_probe` in this tree is the wrong instrument for it (it measures the CPU's
   effective type, not BAR1 throughput).
3. **VRAM capacity becomes a real constraint.** 12 GiB advertised against 12 GiB physical,
   shared with host RM and any co-tenant, versus today's 49 GiB of host DRAM.

**The payoff, if both land:** the aperture goes 2.51 → 313.5 GB/s (**125×**), and on the
matmul the composed 39.8× would collapse toward ~1.2–1.5×. ⇒ **this is the whole remaining
gap to native on large kernels**, and after w320 moved the binding constraint off the submit
path, it is the roadmap.

⊘ **And the ordering is not obvious from the sizes.** Fix 1 is worth 4.9× for roughly one
allocation site; Fix 2 is worth 14.8× for a BAR1 window manager and a correctness class that
has already cost this project weeks. **Fix 1 first**, and its `pde_info` measurement costs one
ioctl.

### 6.7 ⊘⊘⊘ WHAT WENT WRONG IN THIS RUNG — four instrument failures, three of them GREEN

★★★★★ **(1) THE KNOWN-POSITIVE DID NOT FIRE, AND IT REPORTED A PASS.** The `bwneg` arm
exported `KAYFABE_BENCH_NOLAUNCH=1`. `cup8bench_hook.sh` **never forwarded it** — it forwards a
fixed list, and `BENCH_NOLAUNCH` was not on it (the hook's only negative control is a separate
hardcoded `KAYFABE_BENCH_ONLY=negctrl` invocation with its own fixed sizes and no `bw` phase).
So the arm ran the **measurement** workload and returned:

```
BENCH_MODE=MEASURE
BENCH_VERDICT: PASS (every bw row verified)      GUEST_BENCH_TOTAL_BAD=0
```

⊘ **A negative control that silently became a positive one, and announced a PASS.** Nothing in
`bad=0`, in the verdict, in the exit code, or in the arm's name distinguished it. The single
tell was `BENCH_MODE`, three lines up.
★ Fixed twice over, because forwarding alone would leave the next omission invisible:
`B_NOLAUNCH` is forwarded, **and** the arm now **asserts `BENCH_MODE=NOLAUNCH` is present** and
prints `⊘⊘⊘ VOID` if it is not. ⚠ Note my own program already had this guard for `BENCH_ALLOC`
— a misspelled mode is a hard refusal — and I did not put one on the arming that grades
everything else.

★★★ **(2) THE FIRST SWEEP MEASURED L2 AT EVERY PLACEMENT.** The repeat loop is inside the
thread, so its reuse set is `resident_threads × (NF/NT) × 4`, not the buffer. Native `vram`
read **1930 GB/s at 16 MiB** and native `hostalloc` read **107 GB/s** — **8.5× PCIe gen3 ×16's
theoretical ceiling**, which no sysmem read can do. ★ **The instrument's own native arm caught
it**, because an impossible number is easier to disbelieve than a plausible one; had I run only
the guest arm, 30 GB/s would have looked like an interesting intermediate aperture.

⊘ **(3) THE FB SAMPLER RECORDED 58 SAMPLES AND THE SUMMARY PARSED NONE.** `nvidia-smi
--format=csv,noheader,nounits` returns `"1234, 7"`, and `tr -d ' '` glued both numbers into one
field, so the analyser's `$2` was the RSS column and every sample read `UNMEASURED`. **The data
was never missing; the reader was wrong** — and an empty summary is indistinguishable from a
sampler that never started.

⊘ **(4) THE BATCH LAUNCHER `cd`-ED ONE LEVEL SHORT** and all four arms exited 127 in 8 seconds
— **while printing its own `W322_BATCH_TERMINATOR`.** A terminator line says the launcher
reached its end, never that the work ran.

★★ **Three of these four produced GREEN or COMPLETE-looking output.** That is the pattern this
tree keeps paying for, and the only two things that caught them were (a) an arm whose number
was *arithmetically impossible* and (b) checking an `rc` that a terminator had already made
look unnecessary.

### 6.9 PREDICTIONS, GRADED

| # | I predicted (§4, before any boot) | measured | verdict |
|---|---|---|---|
| 1 | guest plateau in the PCIe band, ≥10× below VRAM | 2.51 GB/s: **125×** below VRAM, **4.9×** below the link | ✔ CONFIRMED, and further below the link than "within ~2×" allowed |
| 2 | `memory.used` does not step by the buffer size | ⊘ **the instrument did not resolve** (§6.4) | ⊘ UNMEASURED, not confirmed |
| 3 | sysmem family narrow, within ~2× | `hostalloc` 3.70–11.89 vs `hostreg` 12.33 — **3.2× spread, and unstable run to run** | ⊘ **REFUTED** |
| 4 | "#3 is the one I most expect to be wrong" | it was | ✔ |

★ **#3 is the one that mattered**, and I said so in advance. Its refutation is what dissolves
the brief's central gap: the sysmem family is *not* narrow, w320 sampled it at its worst and
least stable point, and *"our guest beats pinned sysmem"* was a statement about
`cuMemHostAlloc`, not about our buffers.

⊘ **My own §5 caveat fired too**: I wrote that the spectrometer measures a buffer allocated by
the same `cuMemAlloc` path as the matmul's *"by construction, not by observation of the
matmul's own pointers."* The 32 MiB cliff (§6.5) is a case where the two really do behave
differently — the matmul's three 16.78 MiB buffers stay under a ceiling a single 32 MiB
allocation does not — so that caveat was load-bearing rather than decorative.

### 6.10 ★★★ THE GUEST'S *OTHER* CHAIN — ≥3 points, a fit that survives its own guard, and a second cliff datum

`bwhost` runs the same sweep with the guest calling **its own** `cuMemHostAlloc(DEVICEMAP)`.
⊘ w320 could not ask for this: `BENCH_HOSTMEM` was never forwarded to the guest. That path does
not go through FB-leaf publication at all — it is the **guest-RAM pin** chain
(`VerbPlan::PinGuestRam`, one `OS_DESCRIPTOR` and one fixed map per 4 KiB row) — so it is a
second, independent host backing measured by the same kernel.

| buffer | sync_med | read | `bad` |
|---|---|---|---|
| 4 MiB | 1.864 ms | 2.251 GB/s | 0 |
| 16 MiB | 5.643 ms | 2.973 GB/s | 0 |
| 32 MiB | 10.904 ms | **3.077 GB/s** | 0 |
| 64 MiB | 21.626 ms | **3.103 GB/s** | 0 |
| 128 MiB | ⊘ deadline — the arm's inner timeout fired mid-row; `TERMINATOR_SEEN=no` | | |

★★ **This chain gives FOUR points, so the fit the brief demands is possible here** — and it is
run with all three guards, not just quoted:

```
ALL 4    sync = 0.33021*MiB + 0.4332 ms    RMS residual = 0.0869 ms
         residuals  +0.110  -0.074  -0.096  +0.059        (0.44 % of the 1.86-21.63 ms range)
         marginal bandwidth = 3.175 GB/s
DROP 64  sync = 0.32318*MiB + 0.5352 ms    RMS residual = 0.0448 ms
         residuals  +0.036  -0.063  +0.027
         marginal bandwidth = 3.245 GB/s      (slope moved 2.13 %)
```

⇒ **the guest-RAM-pin chain's asymptotic aperture is 3.18–3.25 GB/s**, with a fixed term of
~0.43 ms. ⚠ Contrast with §6.3, where the same arithmetic on **two** points is refused: it is
the same instrument, and the difference is entirely how many points survived.

★★★ **TWO CONCLUSIONS, and the second is the more useful one:**

1. **Both of our host backings sit in the same narrow band — 2.5 and 3.2 GB/s — despite being
   completely different code paths** (memfd FB leaves vs pinned guest RAM). Two independent
   chains landing 1.27× apart, both ~4–5× under a link that two native placements saturate,
   points at something they SHARE. What they share is 4 KiB granularity (§6.8).
2. **32 and 64 MiB succeeded here and failed in the default chain.** ⇒ the §6.5 cliff is
   **specific to FB-leaf publication**, not a general size limit, which both localises it and
   rules out "the guest cannot allocate that much" as the explanation.

### 6.11 CORRECTNESS

⊘ **This rung is measurement-only: `git diff master -- crates/` is 0 lines.** Every edit is in
`scripts/bench/`; the workload's new behaviour is behind env vars defaulting to the previous
behaviour, and the one change that touches every guest run — the hook's env line — was proved
inert by the `sizes` arm reproducing w320 to within 5 % at four sizes. ⇒ per the brief, a
non-regression this rung cannot cause is not evidence it is asked to produce. What is graded
here is that **the measurement** is sound.

| workload | n | result |
|---|---|---|
| `cup8` N=128..2048 bit-exact (`sizes`) | 1 boot, 4 sizes × 12 iters | `bad=0 maxerr=0` ×4, `SIZES_DONE=4`, `Xid=0` |
| `bw` rows, all boots | 4 boots, 12 measured rows | `bad=0` in **every** row |
| `BENCH_ALLOC` mis-arming guard | — | hard refusal by construction (`FAIL-BAD-ARMING`) |
| native ladder | 5 modes × 6 sizes + 12 reps | `bad=0` in every row; `NATIVE_NEGCTRL_RC=0` each arm |
| ★ `bw` negative control | ⊘ **VOID on the first attempt** — see §6.7(1) | re-armed; forwarding + assertion now in the tree |

⚠ **The `bw` verifier's known-positive is asserted for the NATIVE arms** (`w311_native.sh` runs
`BENCH_NOLAUNCH=1` first and reported `NATIVE_NEGCTRL_RC=0` on every one of the 17 native
invocations, i.e. the verifier fired with launches skipped) and was **VOID for the guest arms**
on the boot that attempted it. ⊘ Stated as it is: the `bw` verifier is shown alive on this
build and this box, and *not yet* shown alive inside the guest. The forwarding and the
assertion are committed; the guest-side known-positive is the first thing to run next and it
costs one boot.

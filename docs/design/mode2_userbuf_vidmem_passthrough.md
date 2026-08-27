# Mode-2 user-buffer data plane: real host-vidmem passthrough + CE forward (the cup5/LLM hang fix)

Status: **diagnosis complete & corrected, implementation pending** (2026-06-14, m562). Branch `consolidation`.

Root-cause analysis of the Mode-2 bulk-copy / LLM-model-load slowness and the agreed fix (real
host-vidmem backing for user allocations + copy-engine forward). Reviewable companion to the live
agent-memory notes (`memory/mode2_execfwd_layer2.md` tail).

> **2026-06-14 correction.** An earlier version of this doc blamed a *UC userspace mapping / guest
> libcuda CPU `memcpy`* and asserted the user vidmem `RM_ALLOC` "is forwarded to a real host vidmem
> object." A boot on the **open-580 RM core built from source** (patchable, with placement printks
> compiled into `src/nvidia`) **falsified both claims** — see Diagnosis below. The fix direction
> (real vidmem backing + CE forward) is unchanged and now better-justified.

## Symptom

`cuMemcpyHtoD` of a `cuMemAlloc` buffer is glacial: **~95–107 MB/s**, host GPU util **0%** (host
oracle on the same RTX 3060 does the identical 64 MB copy at **~7 GB/s** via CE). cup2 (4 KB) passes
instantly; the LLM moves ~469 MB of weights and the model load crawls/hangs. Repro: `tests/mode2/cup5.c`
(bulk HtoD/DtoH + byte-verify + timing); discriminator `tests/mode2/cup6.c` (3-copy + self-introspection).

## Diagnosis — ground truth from the open-580-from-source boot (m562)

We built `open-gpu-kernel-modules@580.159.04` **from source** as the guest driver (no `nv-kernel.o_binary`
blob → patchable RM), with two diagnostic printks compiled in, and measured the host side *during* the
copy. Hard facts:

1. **Placement is correct and matches real hardware.** `vidmemConstruct_IMPL` printk: the 64 MB
   `cuMemAlloc` is **FBMEM via PMA** — `pmaInit=1 bIsPmaAlloc=1`. `memmgrInitBaseFbRegions_FWCLIENT`
   printk: our faked GSP advertises a sane FB region table (`numFBRegions=5`, `usable=0x2ecad0000`
   ≈ 11.7 GiB). So the guest RM places the buffer in real vidmem exactly as on bare metal.

2. **libcuda picks the CE/DMA path, like host.** cup6's `/proc/self/maps`×pagemap introspection: the
   device pointer `dp` is **PROT_NONE (`---p`), no CPU mapping at all**. There is therefore **no
   guest-userspace `memcpy`** of the payload — libcuda issues a copy-engine `LAUNCH_DMA`, identical to
   the host oracle. (This retires the earlier "UC userspace memcpy" model, which had mis-attributed an
   unrelated PROT_NONE VA reservation at `0x10000000000`.)

3. **The data plane is fake — that is the whole bug.** Host `nvidia-smi` sampled *throughout* the
   copy: **util stayed 0% and `memory.used` never moved off baseline** (peaked 19 MiB, never the
   64 MB the guest "allocated"). Therefore:
   - the guest's PMA-FBMEM buffer is **fake-backed** (emulated FB / `g_malloc0` `fb_page` /
     guest-RAM), **not a real host vidmem object** — falsifying the prior "is forwarded to real host
     vidmem" claim; and
   - the copy **does not run on the host GPU** — it is **CPU work**. The `NVKVM-DPLANE` probe (below)
     confirmed the mover is **QEMU's emulated `LAUNCH_DMA` loop** at `nvkvm_gpu_emul.c:3755-3844`,
     which resolves and copies **4 bytes at a time** (`ce_bytes_total` scales with the payload; every
     byte funnels through `nvkvm_fb_write`) → the ~100 MB/s ceiling.

> **NVKVM-DPLANE diagnostic boot (2026-06-14) — refines the above.** A gated probe (CE per-call dest
> verdict + byte attribution) showed: (a) the **emulated CE loop is the data mover** — confirmed, not a
> kernel memcpy; (b) the dest is **mostly real-backed already** — `overlay_real_write_bytes ≈ 75 MB`
> (bulk 16/14/2 MB chunks `verdict=gpga`, real host vidmem) vs only `fbpage_write_bytes ≈ 8.6 MB` (small
> scattered 64 KB chunks `verdict=fbpage`, fake) plus 785 virtual-addressed copies. Host `memory.used`
> stays ~19 MB because those real regions are small and **overwritten** (staging/scratch reuse), not a
> fresh 64 MB object. ⇒ the dominant fix is **(B) forward the CE to the host GPU**; backing (A) is
> largely present and shrinks to closing the `fbpage` gaps + host-VAS-mapping the full user dest.

### What is NOT the problem (closed branches — do not re-investigate)

- **Placement / PMA / FB-region table** — correct (item 1). The faked-GSP `fbRegionInfoParams` is fine.
- **Cacheability / UC vs WB / `nv_encode_caching`** — moot: `dp` has no CPU mapping, so its cache
  attribute is irrelevant to the copy. The WB-everywhere principle ([[mode2_wb_cacheability_principle]])
  still holds generally, but it is **not** the cup5 lever.
- **libcuda's CE-vs-CPU heuristic** — correct: it chose CE, same as host.

## Root cause — the FBMEM the guest gets is a fake page, and the CE runs on the CPU

The guest RM correctly allocates FBMEM and emits a CE copy, but in Mode-2 that FBMEM is backed by
emulated/guest-RAM pages instead of a real host GPU vidmem object, and the CE descriptor is executed
by QEMU on the CPU instead of being submitted to the host GPU. Real double-mmap passthrough **exists
and is proven**, but is wired only for **privileged** buffers (USERD / GPFIFO / GR-context), reactively
via `m2exec`. User data allocations were left fake as bring-up scaffolding (see
`mode2_memory_model.md:36-40`, `mode2_dataplane_architecture.md:244-265`, which names this exact gap).

This matches the architectural principle: **guest userspace should see almost no fake pages** —
anything guest userspace can obtain as a page is also obtainable by the QEMU/isolate via host
ioctls/mmap, so it should be direct passthrough (guest RAM, or real host-GPU memory mmap'd through).
Fake pages belong to the guest *kernel* side (PDTs etc.). This user buffer is wrongly fake.

## Reusable machinery (already proven — extend to user allocations)

| Function | File:line | Role |
|---|---|---|
| `nvkvm_m2_host_alloc_map_vidmem()` | `nvkvm_gpu_emul.c:4577` | alloc real host vidmem (CONTIGUOUS\|VIDMEM) + host `RM_MAP_MEMORY` → `{qva,mapfd,size}` |
| `nvkvm_m2_back_and_map()` | `:5056` | double-mmap a host vidmem obj into `m2_fbback[]` at guest-FB phys (guest CPU + host GPU share bytes) + FIXED-map into host GR VAS at guest VA. Called today only with labels `ctx*`/`gpfifo`/`pushbuf`/`userd` |
| `nvkvm_m2_back_and_map_sys()` | `:5260` | OS_DESCRIPTOR sysmem variant |
| `nvkvm_m2_map_dma()` | `:4834` | `RM_MAP_MEMORY_DMA` FIXED (unprivileged) into a VASpace |
| `nvkvm_m2_gpga_obj()` | `:5297` | register GPGA→host-obj (`m2_objs[]`+`m2_gpga[]`) so BAR1/PRAMIN/overlay resolve via `nvkvm_fb_host_overlay()` |
| host GR GPFIFO forward (doorbell) | `nvkvm_m2_doorbell_setup():5119`, `nvkvm_m2_exec_doorbell():2580-2637` | AMPERE_USERMODE_A doorbell + work-submit token; rings real host GR channel. **CE-forward extends this** |
| emulated CE (to be bypassed for forwarded chans) | `nvkvm_chan_execute()` `LAUNCH_DMA` `:3755-3844` | current 4-byte CPU copy + completion-sema; user-CE sema already gated host-only |
| isolate device-mmap into GPA window | `nvkvm_isolate_handlers.c:1808-1839` | the real passthrough mmap; user vidmem mmap currently falls to anon/guest-RAM (~1858-1869) |

## Fix plan — option 3 (real vidmem backing + CE forward)

**(A) Back user FBMEM with a real host vidmem object.**
1. **Find the snoop hook** — the precise point where a guest user `cuMemAlloc` FBMEM allocation becomes
   observable to the QEMU device (guest NVOS32 / `NV01_MEMORY_LOCAL_USER` `RM_ALLOC`, and the guest-FB
   physical range PMA assigns it). User allocations are **not** snooped today; this is step 0.
2. For each such allocation, allocate the real host vidmem object (`host_alloc_map_vidmem`) and back the
   guest-FB-phys range with it via `back_and_map`/`gpga_obj` (double-mmap: CPU view at guest-FB phys,
   GPU view at the guest VA in the host GR VAS) instead of a fake `fb_page`.
3. **Verify (A):** re-run cup6 → host `memory.used` jumps ~64 MB (buffer now lives on the host GPU),
   byte-exact preserved.

**(B) Forward the CE copy to the host GPU.**
4. With src (guest sysmem staging, already host-DMA-able via the memfd window / OS_DESCRIPTOR) and dst
   (now real host vidmem) both real host objects, submit the `LAUNCH_DMA` to a host CE channel (new
   `m2_ce_channel`, or the GR channel's CE subchannel) and ring the doorbell — extend `exec_doorbell`
   (`:2580-2631`); bypass the emulated 4-byte loop for forwarded channels. User-CE completion sema is
   already gated host-only.
5. **Verify (B):** cup5/cup6 → **host util>0**, **GB/s**, byte-exact; then llama (`m557`).

This honours the directive to prove real host compute (util>0, HW-written results), not faster faking —
optimising the emulated CPU CE loop would be "faster faking" and is explicitly rejected.

**Diagnostic boot before coding (A):** one instrumented boot to pin (i) the exact handler/site where a
user `cuMemAlloc` FBMEM alloc + its guest-FB-phys is observable in QEMU, and (ii) confirm which CPU path
actually moves the 64 MB today (QEMU emulated-CE vs kernel memcpy into `fb_page`) — that fixes where the
backing is wired.

## Diagnostic artifacts

- Tests: `tests/mode2/cup5.c` (bulk repro), `tests/mode2/cup6.c` (3-copy + maps×pagemap introspection).
- Harnesses: `m558_bulk_dataplane_host.sh`, `m559_cpu_attrib_host.sh`, `m560_copy_discriminator_host.sh`,
  `m561_cache_probe_host.sh`, **`m562_placement_probe_host.sh`** (open-580 from-source + placement printks);
  `rtp_run_guest.sh` `cup6` subcommand.
- Open-580 from-source guest driver: host tree `research_clones/open-580` (full source); lean bootable
  tree `research_clones/open-580-guest` (kernel-open + materialized `nv-kernel.o_binary` + 0xFFF500 uvm
  patch); boot via `OGKM=research_clones/open-580-guest`. Placement printks in
  `open-580/src/nvidia/src/kernel/{gpu/mem_mgr/mem_mgr_gsp_client.c (NVKVM-FBREG), mem_mgr/video_mem.c (NVKVM-PLACE)}`.
- QEMU probes: `CE-INSTR` (~:2534), tagged `M5: CE COPY` (:3762), `BAR1-TRAP-INSTR` (`nvkvm_baraperture_write`, :2876).

---

## Consolidation ledger (2026-06-14) — STOP, read before writing more reactive code

Written after a long back-and-forth (a dozen hypotheses) so the next person — possibly the
maintainer debugging this by hand — starts from settled ground, not a re-guess. **The headline:
the "reactive re-walk" hardening one would naively add ALREADY EXISTS at maximal coverage. Adding
more reactive special-cases is the slop trap. The real open question is an *observation*, not a
patch.**

### A. ESTABLISHED (high confidence, with evidence)

1. **Placement is correct.** The 64 MB user `cuMemAlloc` lands FBMEM-via-PMA in the guest RM,
   identical to bare metal (open-580-from-source printks: `pmaInit=1 bIsPmaAlloc=1`,
   `numFBRegions=5`, ~11.7 GiB). Not a placement bug.
2. **libcuda picks CE/DMA, like host.** `dp` is `PROT_NONE` — no guest-userspace `memcpy`. Same
   copy path as the host oracle (cup6 maps×pagemap).
3. **The data plane is fake-backed where it slips.** Host `nvidia-smi` during the copy: util 0%,
   `memory.used` flat (~19 MiB peak, never +64 MB). The guest's "vidmem" bytes the host GPU can't
   see land in emulated FB / `g_malloc0` `fb_page` / the guest-RAM memslot.
4. **The bulk 64 MB does NOT flow through the emulated CE.** Max CE COPY = 4 KB, CE total ~1.36 MB
   (the CE carries only the MEMSET buffer-zeroing ~79 MB + tiny copies). The 64 MB DATA moves via
   the **CPU-write / GPA-window path**, not a `LAUNCH_DMA`. ⇒ "forward the CE to the host" would
   speed MEMSET, not the weight HtoD.
5. **`gpga_obj` is the real-backing primitive and it works.** `nvkvm_m2_gpga_obj()` allocs a real
   host vidmem object, copies pre-existing `fb_page` bytes in, FIXED-maps it into the host GR VAS
   at the guest VA, and registers the gpga→obj range so `nvkvm_fb_host_overlay()` resolves all
   CPU/BAR1/PRAMIN access to it. cup4 NxN fp32 matmul is byte-exact **because** its buffers happen
   to be caught by the walk and `gpga_obj`-backed.

### B. The reactive machinery THAT ALREADY EXISTS (do not re-implement)

- **Per-doorbell compute-VAS re-walk** — `nvkvm_m2_exec_doorbell()` (`:5851`), M5.10 + M5.48c:
  re-runs `enum_gr_sysmem(grc)` on **every doorbell with new work** (GP_PUT advanced), idempotent
  via `va_seen`, capped 1000 sweeps. This is "back newly-mapped user buffers before the host GPU
  touches them" — already maximal for the GR client.
- **Walk-driven vidmem backing** — `nvkvm_m2_leaf_flush()` vidmem branch (`:5622`), per-2-MiB-chunk
  dedup, calls `gpga_obj` for each new vidmem leaf.
- **Copy-time dst backing (Phase A / M5.60, gated `m2cefwd`)** — at `LAUNCH_DMA` (`:3873`): PHYS-FB
  dst → `gpga_obj` directly (covers MMU-bypassing physical copies that have no PTE); VIRT dst →
  re-walk. Regression-clean, OFF by default.

### C. Hypotheses TRIED and KILLED (the back-and-forth, so we don't relitigate)

| # | Hypothesis | Verdict | Why killed |
|---|---|---|---|
| 1 | Enlarge BAR1 / **resizable BAR** so the CPU can map big vidmem through | **NO-OP for THIS gate; REAL for prod (deferred, NOT dead)** | No-op *now*: emulated BAR1 is 256 MB *MMIO* and today the guest CPU never reaches user vidmem through it (vidmem mmaps are 8 GiB+ guest-RAM/GPA-window VAs, `dp` PROT_NONE; `mode2_dataplane_architecture.md:242-260`). BUT for **prod** the 256 MB hardcode (`bar1_size`) is a genuine ceiling — once >256 MB of gpga objects are CPU-mapped through BAR1 it must grow / become resizable. Keep as a prod-scale task. |
| 2 | **Eager-snoop the `cuMemAlloc`** at alloc time → back full range | **NOT FEASIBLE as imagined** | User vidmem is NVOS32/VidHeapControl, allocated guest-RM-locally via PMA with **no GSP-RPC** → no QEMU-visible alloc event, no observable guest-FB-phys until map (PT walk) or copy (LAUNCH_DMA dst). That's *why* backing is reactive. |
| 3 | UC / non-WB userspace mapping forces a slow CPU `memcpy` | **DEAD** | `dp` has no CPU mapping at all (PROT_NONE); cacheability is irrelevant to this copy. |
| 4 | libcuda chose CPU instead of CE (a heuristic gap) | **DEAD** | It chose CE, same as host. |
| 5 | The kernel `memmgrGetMemTransferType` picks memcpy on *coherency* | **DEAD** | It branches on **aperture** (SYSMEM↔SYSMEM → CPU; else CE), not coherency (ogkm `mem_utils.c:73-81`). And the bulk isn't the kernel path anyway. |
| 6 | Simulation/emulation strap makes RM take a slow path | **DEAD** | `SIM=0 EMU=0 FMODEL=0 RTLSIM=0` (NVKVM-EAS printk). |
| 7 | FBMEM aperture mis-reported as SYSMEM to libcuda | **DEAD** | FBMEM resolves to VIDMEM correctly (`kbusGetEffectiveAddressSpace`). |
| 8 | "Forward the CE to the host GPU" is the fix | **MOSTLY MOOT** | The 64 MB isn't a CE copy (fact A4). CE-forward only helps MEMSET. |
| 9 | Add a per-doorbell re-walk to catch late buffers | **ALREADY DONE** | M5.48c (section B). Re-adding = slop. |

### D. The ONE remaining unknown — it is an OBSERVATION, not a patch

The m563/Phase-A boot showed the bulk dst is **mostly already real-backed** (`verdict=gpga`
≈ 75 MB) yet host `memory.used` stays ~19 MB. Two mutually-exclusive readings, and **we have not
disambiguated them**:

- **(D1) backed-but-churned** — the dst IS real host vidmem, but `gpga_obj` allocates *small*
  objects that get *reused/overwritten* (staging/scratch), so residency never accretes to 64 MB.
  If so, **correctness may already hold** (every byte the host GPU reads is real at read time) and
  the flat `memory.used` is an accounting artifact, not garbage. → then there is no correctness
  bug here, only the perf one (deferred).
- **(D2) truly-unbacked tail** — some fraction (the ~8.6 MB `verdict=fbpage` + 785 virtual copies,
  or a physical-addressed buffer in a non-`grc` VAS the sweep never visits) is fake, and a real
  compute READ of it returns garbage. → then *that specific slice* is the correctness gap, and the
  fix is to make exactly that slice resolve to `gpga`.

**The decisive experiment (one boot, no new code):** run a *compute* workload that READS a large
user buffer the guest wrote (not matmul, whose buffers are walk-caught) and byte-verify the GPU's
output. Byte-exact ⇒ (D1), correctness already holds, move to perf. Wrong bytes ⇒ (D2); then dump
the verdict tally for *that* buffer's leaves to see which classify `fbpage`/untranslated and back
*only* those. Either way the next move is **measure, then target** — not another blanket sweep.

**The stronger invariant (do not lose sight of it even if D1).** Per `mode2_memory_model.md`, guest
*userspace* should see **~no fake pages at all** — anything guest userspace can obtain as a page is
also obtainable by QEMU/isolate via host ioctls/mmap, so it must be real passthrough. A workload
passing (D1) only proves *that* workload's reads happened to hit real-backed bytes; it does NOT
prove the invariant holds. So the experiment should ALSO audit: after the run, does **any** leaf in
a user (non-kernel) VAS still classify `fbpage` (fake)? Any such leaf is a latent correctness/
security bug — a different workload could read it as garbage. D2 is just the case where the tested
workload already trips it. The end-state is "zero user-visible fake pages," not "this test passes."

### E. Why no code was written this round

The authorized "reactive hardening" already exists (B). Until (D1) vs (D2) is resolved by the
experiment in (D), any new backing code is a guess layered on a working milestone — the slop risk
the maintainer flagged. Working milestone (`90271e8`, default path, cup4 byte-exact) is **untouched**;
the only diff is the gated `m2cefwd` Phase-A (+102 lines, OFF by default).

---

## VERDICT (2026-06-14, m564/cup7): D2 — ROOT CAUSE = HOST GPU BAR1 (256 MiB) EXHAUSTION

The decisive experiment ran. `tests/mode2/cup7.c` (host GR kernel `out[i]=in[i]+1` over a 64 MiB
user `cuMemAlloc`, byte-verified) on the **default path** (working milestone, no `m2cefwd`):

- cup7 **hung** (rc=124) — not garbage-read, worse: a hard host fault mid-copy.
- **`nvkvm_m2_gpga_obj` failed 232×** after **148 successful objects**; cap NOT hit (148 ≪ 1024).
- Inner failure (M5.3 log): **`RM_MAP_MEMORY ... st=0x51`** (insufficient resources) — the *CPU
  mapping* step, not the vidmem `RM_ALLOC`.
- Sum of successfully CPU-mapped host vidmem at the failure point = **248.2 MiB**.
- Host GPU **BAR1 = 256 MiB** (`nvidia-smi`: Total 256 / Used 1 / Free 255 at idle), **not
  runtime-resizable** (no `/sys/.../resource1_resize` → BIOS never exposed ReBAR).
- Host **Xid 31 — CE2 MMU FAULT_PDE ACCESS_TYPE_VIRT_WRITE @ `0x77c7_1d800000`** = *exactly* the
  first leaf `gpga_obj` failed to back (`cva=0x77c71d800000`). The host CE faulted on the unbacked
  leaf → channel wedge → cup7 hang.

**Mechanism:** `nvkvm_m2_host_alloc_map_vidmem()` (`:4765`) CPU-maps **every** backing object via
`RM_MAP_MEMORY` + `mmap`, which consumes the **host GPU's 256 MiB BAR1**. We are profligate — all
148 backings (GR-ctx, sysmem mirrors, pushbuffers, vidmem leaves) burn BAR1 — so a large user
buffer's tail pushes cumulative mapped vidmem past 256 MiB and the maps fail. The backing logic is
otherwise CORRECT (the first 248 MiB mapped fine, `gpu_mapped=1 st=0x0`); the only wall is the
host BAR1 budget. `st=0x51` = the host RM refusing a BAR1 mapping it has no aperture for.

### Fix options (this CONVERGES the "perf" path with the correctness gate)

1. **Host resizable BAR (your instinct — vindicated, but HW-blocked here).** Full 12 GiB host BAR1
   would let all objects CPU-map. NOT available on this vast.ai box (no BIOS ReBAR / no runtime
   resize). Could try a vast.ai instance with ReBAR enabled. This is the clean PROD answer (task #5).
2. **Shrink the host-BAR1 footprint — stop CPU-mapping objects the GUEST CPU never touches.** Many
   of the 148 are GPU-only (golden GR-ctx, pushbuffers the host fetches via its own VAS) and don't
   need a `qva`/`m2_fbback` CPU view at all — back them with `map_dma` into the GR VAS WITHOUT the
   `RM_MAP_MEMORY`+`mmap`. Frees BAR1 for the buffers that genuinely need a guest-CPU view. Buildable,
   milestone-safe (gated), buys headroom up to ~256 MiB total — fixes moderate buffers, NOT multi-GB.
3. **Forward the CE to the host GPU (the "deferred perf" path — turns out to be load-bearing for
   correctness).** If the bulk HtoD runs as a real host CE copy, the source is SYSMEM staging
   (host-DMA-able via the guest-RAM memfd, **no BAR1**) and the vidmem dst needs only a GPU-side
   `map_dma` — **never CPU-mapped, so zero host BAR1**. This is the ONLY option that scales to GB
   LLM weights on a 256 MiB-BAR1 host. ⇒ un-defer #2 (CE-forward); it is not merely perf.

**Why this is not slop:** the backing code is correct; the bug is a *resource budget* (host BAR1),
proven by byte-count (248/256 MiB) + error code (st=0x51) + fault address match. The fix is to stop
spending BAR1 we don't need (opt 2) and/or move bulk data off BAR1 entirely (opt 3), not to bolt on
another reactive sweep.

### Confirmation (m564b, cup7 @ 8 MiB): D1-within-budget — backing logic is CORRECT

Re-ran cup7 at **8 MiB** (under the 256 MiB BAR1 budget). Result: **PASS=D1**, `bad=0`, **gpga_obj
FAILED = 0**, host BAR1 peak 138 MiB. The host GR engine read the user buffer byte-exact. This
isolates the variable cleanly: same code, same path — small buffer passes, large buffer fails only
because cumulative CPU-mapped vidmem crosses 256 MiB. ⇒ the backing is correct; the bug is purely
the host-BAR1 budget. (Boot/ctx vidmem overhead ≈ 80 objs ≈ 130 MiB, so today only ~126 MiB of BAR1
is left for user buffers — footprint-shrink (#6) widens that; multi-GB still needs CE-forward (#3).)
(Harness note: m564 greps host `dmesg` which is NOT cleared across QEMU restarts → a stale Xid from a
prior boot can appear; trust the per-run `gpga FAILED` count + VERDICT, not raw dmesg Xid.)

---

## CE-FORWARD BUILD PLAN (2026-06-14) — the off-BAR1 fix, phased + gated

Chosen fix (user, 2026-06-14): forward the bulk user-CE `LAUNCH_DMA` to the **host** GPU so the
vidmem dst is **never CPU-mapped** (zero host BAR1) and the src rides the guest-RAM memfd (zero
BAR1). This is the only path that scales past the host's 256 MiB BAR1 wall (proven D2 root cause).

### What ALREADY exists (reuse, do not rebuild)

- **Host channel forwarding infra** — `shadow_fwd` (`:4252`) creates host channels mirroring the
  guest's; `exec_doorbell` (`:2580+`) rings each channel's own `host_token` (M5.22), schedules its
  TSG (M5.25 `GPFIFO_SCHEDULE`), aligns the host GP_GET cursor. The user-CE channel (0xc56f) is
  already shadow-forwarded and its doorbell already rings — today a **no-op** because its work
  (pushbuffer + src/dst mappings) isn't bridged, so the emulated 4-byte loop does the copy instead.
- **GR pushbuffer bridge** — `exec_doorbell` M5.9 (`:5931+`) maps each GR GPFIFO entry's pushbuffer
  into the host VAS (`back_and_map`) so the host GR fetches real work. **CE needs the same.**
- **Sysmem src mapping (no BAR1)** — `back_and_map_sys` (`:5448`) OS_DESCRIPTORs guest sysmem into a
  host VAS. The HtoD src (guest staging) is guest-RAM, host-DMA-able via the shared memfd.
- **Real vidmem dst object** — `gpga_obj` (`:5485`) / `host_alloc_map_vidmem` (`:4765`). **The one
  change:** a GPU-only variant that SKIPS `RM_MAP_MEMORY`+`mmap` (the BAR1 consumer) — the CE dst is
  PROT_NONE to the guest CPU, so it needs only a GPU-side `map_dma` into the CE channel's VAS.
- **Completion sema** — already host-gated for user-CE clients (`nvkvm_chan_sem_wr32`, M5.49b).

### The exact gaps (this is the whole build)

1. **Off-BAR1 dst backing.** New `host_alloc_map_vidmem` flag `gpu_only` → skip the CPU map; in
   `gpga_obj`, when backing a CE-forward dst, use it (no `cpu_qva`, no `m2_fbback`, no host BAR1).
   The dst still gets a real host vidmem object + `map_dma` FIXED into the CE channel's host VAS.
2. **Bridge the CE pushbuffer + GPFIFO** to the host CE channel (mirror M5.9 for the 0xc56f CE chan):
   map each pushbuffer the guest's CE GPFIFO entry points at into the host VAS so the host CE fetches
   the real `LAUNCH_DMA`.
3. **Map src + dst into the CE channel's host VAS** at the guest VAs (src via `back_and_map_sys`,
   dst via the gpu_only vidmem map_dma) so the host CE's MMU resolves both operands.
4. **Stop emulating** the 4-byte loop for the forwarded user-CE channel (gate the `:3953` COPY loop
   off when the channel is host-forwarded) — let the host doorbell ring (already wired) do the copy.

### Guardrails to PRESERVE (breaking these = regression)

- M5.39: never ring/forward the guest-KERNEL CE scrubber (client `0xc1d00001`). CE-forward applies
  ONLY to user-CE clients (`nvkvm_m2_is_user_ce`).
- The MEMSET/SCRUB paths (`mscrub`/`remap`) stay emulated (they zero fake-FB / wipe USERD; not user data).
- Default path (no `m2cefwd`) must stay byte-identical — everything gated.

### Phased rollout (each phase = one bench boot, observable signal)

- **P1** off-BAR1 dst object: log `gpu_only vidmem obj st=0` + confirm `gpga_obj` no longer fails at
  148 (BAR1 usage flat). Metric: cup7@64MiB gets ALL leaves backed (gpga FAILED=0), even if the copy
  is still emulated.
- **P2** bridge CE pushbuffer + map src/dst into the CE VAS: log host CE GP_GET advancing + no Xid.
- **P3** disable emulated loop for the forwarded chan; verify the host CE actually moved bytes:
  **host `memory.used` rises ~64 MiB, util>0**, cup7@64MiB byte-exact (hang→PASS).
- **P4** scale: cup7 at 256 MiB+ (past BAR1) byte-exact; then llama (`m557`).

Success = cup7@64MiB flips hang→PASS=D1 WITH host mem.used+util reflecting a real host-GPU copy
(un-forgeable). Gate: extend `m2cefwd` (OFF by default); working milestone 90271e8 stays the default.

### P1 ORDERING FINDING (2026-06-15) — the window arrives too late for single-shot copies

Implementing P1 surfaced a real ordering constraint that the build plan above missed. The intended
design — observe the user-CE dst at `LAUNCH_DMA` (M5.60 in `nvkvm_chan_execute`), register a dst VA
window, then have the re-walk back those leaves GPU-only — **cannot reduce host BAR1 for a single
copy**, because the doorbell handler runs the backing walk BEFORE it decodes the copy:

- `nvkvm_chan_io_write` doorbell path: `nvkvm_m2_exec_doorbell(s)` (`:2491`, contains the M5.10 /
  M5.48c GR-VAS re-sweep that backs every newly-mapped leaf via the **CPU-mapped** `gpga_obj` and
  marks it `va_seen`) runs FIRST; only THEN does the `nvkvm_chan_execute(s)` loop (`:2641`) decode
  the channel's `LAUNCH_DMA` and reach M5.60 where the window would be registered.
- So by the time the window exists, the dst leaves are already CPU-mapped + `va_seen` (idempotent
  dedup → the M5.60 re-walk won't redo them GPU-only). Corroborated by m564: gpga_obj count went
  89 (8 MiB cup7) → 148 (64 MiB cup7), i.e. the **doorbell walk itself** backed the ~59 user-buffer
  leaves CPU-mapped, before any copy decode.

The window code IS built, compiles, gated behind `m2cefwd`, and is reusable — it just needs the dst
VA known BEFORE the walk. Two clean redesigns (a decision, recorded for the next session):

- **(A) Pre-walk window** — in `exec_doorbell`, BEFORE the M5.10 sweep, peek the pending user-CE
  channel's GPFIFO/pushbuffer for `LAUNCH_DMA` OUT addresses, register the window, then walk →
  those leaves back GPU-only. Localized to `exec_doorbell`; pulls a slice of P2 (pushbuffer decode)
  forward. Smallest change; reuses all P1 code.
- **(B) Lazy GPU-only-by-default + promote-on-CPU-touch** — back EVERY vidmem leaf GPU-only (zero
  BAR1) at walk time; promote a leaf to a CPU map only when the guest CPU actually touches its GPGA
  through the overlay (the trap point). No window/heuristic; correct by construction; this IS
  rewrite Pillar 3 ("trap only what has a side-effect; back the rest lazily"). Bigger: adds a
  promotion path to the overlay hot path (alloc CPU map + copy + redirect on first guest write).
  Pushbuffers/USERD/GPFIFO (guest-written, host-fetched) are exactly the leaves that get promoted.
- Note: the as-built window ALREADY helps a MULTI-copy workload (LLM streaming many weight tiles):
  copy #1's window guides copies #2..N. It only fails the single-shot cup7 microbench. So (A) is
  the targeted fix for the benchmark; (B) is the architecturally-right model.

### P1 EMPIRICAL CONFIRMATION (m565, 2026-06-15) — ordering defeats the window; M5.60 doesn't fire

Ran cup7@64 MiB with `NVKVM_M2CEFWD=1` and the new window/gpu_only code (compiles, gated). Result:
- **windows registered: 0, gpu_only objs: 0, M5.60 events: 0** — the window code never ran.
- **gpga_obj count: 149, gpga FAILED count: 232** (all `client=0xc1d00003`, st=0x51), **Xid 31 CE2
  FAULT_PDE @0x71ae9b800000** = first un-backed leaf, **cup7 rc=124 (HANG)** — identical D2 wall to
  m564. (Harness `RC=$?` captured ssh's rc=0, not cup7's; inline `exit rc=124` is authoritative.)
- Sharper finding: the EXISTING Phase-A hook M5.60 fired **0×** despite `chan_execute` decoding 727
  LAUNCH_DMA COPYs. Dominant CE client = `0xc1d00001` (kernel scrubber, correctly M5.39-excluded);
  the user buffer is `0xc1d00003`. M5.60's gating (`m2_gr_client` set + `is_user_ce` + un-backed)
  does NOT catch cup7's HtoD — most likely the HtoD runs before any GR kernel sets `m2_gr_client`.
  The forwarded host CE still ran and faulted on the un-backed dst (the CE2 Xid) → wedge.

Bearing on the A/B fork: **(A) pre-walk window inherits M5.60's gating fragility** (it must identify
the dst at doorbell time — exactly what fired 0× here) so (A) = fix the gating + move it pre-walk.
**(B) lazy gpu_only-by-default + promote-on-touch is robust to mis-identification** (backs every
vidmem leaf off-BAR1 unconditionally; the host CE never faults because every dst is real-backed) and
is rewrite Pillar 3. Tradeoff: (B)'s failure mode is SILENT (a missed CPU-touch promotion → stale
bytes), (A)/today's is LOUD (fault→hang). Decision pending (user chose "confirm empirically" → done).

### CHOSEN DESIGN (2026-06-15, user): MAP-ON-TOUCH promotion (B's model over the existing trap path)

Supersedes the window/pre-walk P1 above. User direction: "not pure A, a bit of B … map-on-touch is
the most correct setup as in that case ORDER no longer matters." Correct — every failure this
session was an ordering race; map-on-touch dissolves it. The design:

- **Default = GPU-only.** The doorbell walk backs compute-client user-vidmem leaves `gpu_only` (real
  host vidmem object + `map_dma` into the host VAS, NO `RM_MAP_MEMORY`/mmap → zero host BAR1), in
  ANY order. The `fbback`/control-channel paths stay as-is (protect the milestone).
- **Promote on CPU touch.** The guest's CPU access to vidmem already TRAPS to QEMU (GPA-window path,
  `nvkvm_fb_read/write`, PROT_NONE mmaps — the ~100 MB/s path). On the first trap to a `gpu_only`
  object: `RM_MAP_MEMORY` the SAME host object (coherent — identical bytes), **replay any pre-
  promotion `fb_pages` writes into it** (the existing M5.44 copy-preserve loop, which `gpu_only`
  currently skips), store `cpu_qva`, redirect. BAR1 is consumed ONLY for pages the CPU actually
  touches. The dst buffer (CE-written, CPU-never-touched) stays `gpu_only` forever → off BAR1 → past
  the 256 MiB wall.
- **Why ordering no longer matters:** produce-then-consume holds — the guest WRITES a control page
  (traps → promotes → copies) BEFORE it rings the doorbell, so the host fetch sees real bytes; the
  bulk dst is written by the host CE (GPU side) and read back via CE DtoH, so the CPU never touches
  it and it never promotes. No window, no `m2_gr_client` gating (which fired 0× in m565), no race.
- **Refinements over the raw brainstorm:** (1) a READ trap must serve the object's REAL bytes, not a
  zero page (zero-RO is only a *usage tripwire* for the perf path, not the correctness path); (2) we
  do NOT need page-protection (PROT_NONE/RO-then-write-trap) in C — we already trap every FB access;
  the userfaultfd/mprotect tripwire + 16 MiB batching is the REWRITE's fast-path job; (3) "silent
  stale read" (B's classic risk) is MITIGATED in C precisely because all FB access traps → no missed
  touch; that risk only re-emerges in the rewrite's fast-memslot world, where the tripwire is the fix.

**Rewrite role:** implement this SAME model fast+safe — real RW memslots over the GPA window, touch
detection via userfaultfd/mprotect (zero-RO tripwire, RO-on-read, promote-on-write, batched), promotion
as a typed state machine instead of a hook in `nvkvm_fb_write`. The C version is the executable spec +
correctness oracle (cup7 = its conformance test); building it now is NOT build-twice slop (we reuse the
existing trap, not the memslot/uffd subsystem) — it's the minimal thing that clears the apps-pass gate.

**Build steps (gated `m2cefwd`):** (1) walk → `gpu_only` default for compute-client user vidmem;
(2) promotion hook in the FB trap path (alloc CPU map of same hMem + replay fb_pages + redirect);
(3) cup7@64 MiB metric: `gpga FAILED=0`, BAR1 reflects only CPU-touched pages, no Xid, no hang, and
(once the host CE actually moves the bytes) byte-exact. The `gpu_only` primitive + `gpga_obj_ex` are
already built/compiling/gated; the window/`m2_cefwd_dst` plumbing is now superseded (keep or strip).

### BUILT + VERIFIED — map-on-touch clears the D2 wall (m566, 2026-06-15) ★★★★★

The map-on-touch design above is implemented (gated `m2cefwd`) and **PROVEN on the bench**. cup7@64 MiB
— the EXACT workload that hung (rc=124, Xid 31 CE2 FAULT_PDE, `gpga FAILED=232`) at the host-BAR1 wall
in m564/m565 — now **PASSES byte-exact**:

| signal | m565 (window, pre-design) | m566 (map-on-touch) |
|---|---|---|
| cup7@64 MiB VERDICT | HANG rc=124 | **PASS=D1 byte-exact** (bad=0) |
| `gpga FAILED` | 232 (BAR1 wall) | **0** |
| `gpu_only` objs | 0 (window lost the race) | **147** (walk default engaged) |
| eager CPU-mapped objs | (all) | **8** (the written control leaves only) |
| PROMOTED (lazy CPU map on touch) | n/a | **57** (replayed=0x0, coherent) |
| GIVE-UP (BAR1 full at touch) | n/a | **1** × 64 KiB → fb_pages, cup7 still PASS |
| host Xid this run | 31 (fresh) | none (the logged one is stale m565 pid 663933) |
| GPU health | wedge risk | D-state 0, idle |

What the numbers say: the **ordering-immune walk default** (leaf_flush requests `gpu_only` for compute
clients; `gpga_obj_ex` keeps it `gpu_only` iff the run is blank at walk time) did ALL the work —
**M5.60 fired 0×**, so the decode-time net is now just a backstop. 147 blank dst/scratch leaves went
off-BAR1 (real host vidmem + `map_dma`, zero `RM_MAP_MEMORY`); only 8 already-written control leaves
took an eager CPU map + copy-preserve. 57 of the `gpu_only` leaves were later CPU-touched and promoted
lazily (RM_MAP_MEMORY the SAME hMem, `replayed=0x0` because a pure dst is blank pre-promotion → the CPU
view sees exactly the host-CE/GR bytes → coherent). The single GIVE-UP (a 64 KiB obj that couldn't get
a CPU map at a momentary BAR1-full) fell back to fb_pages WITHOUT breaking correctness — the graceful
degrade path working as designed. Net: D2 host-BAR1 (256 MiB) exhaustion is **solved** for a 64 MiB
user buffer with host GR compute, byte-exact. Implementation: `nvkvm_gpu_emul.c` — `m2_objs[].promote`
state (0/1/2), `nvkvm_m2_host_map_existing_vidmem` + `nvkvm_m2_promote_gpu_only` (overlay hot-path
hook), `gpga_obj_ex` blank-vs-written gate, `leaf_flush` compute-client default. Harness:
`scripts/mode2_diag/m566_maptouch_host.sh`. NEXT: scale-test (cup4 matmul / m553 LLM at size) to
confirm general-compute correctness; then the perf half of CE-forward (bulk HtoD via host CE).

**GENERALIZED to real compute (m567/cup8, 2026-06-15):** a REAL grid fp32 matmul (N=2048, 48 MiB
A/B/C, 2D grid 128×128 blocks, host GR engine) PASSES byte-exact (bad=0, maxerr=0) with
`m2cefwd` — **host GPU util 100%** (un-forgeable: the CE software path cannot multiply/sum), gpga
FAILED=0, gpu_only=105, eager CPU-mapped=8, PROMOTED=65 (the HtoD-written A/B inputs promote on the
CPU write; replayed=0x0), GIVE-UP=1 (graceful), M5.60=0, no new Xid, GPU healthy. So map-on-touch
holds for general compute (two large input reads + output write), not just cup7's add-1 kernel.
Test `tests/mode2/cup8.c`, harness `scripts/mode2_diag/m567_matmul_scale_host.sh`. KNOWN LIMIT this
exposes for the LLM: HtoD-written buffers (LLM **weights**) promote-on-write → consume host BAR1, so
a GB-scale weight set will hit the 256 MiB wall again until the CE-forward PERF half lands (forward
the bulk HtoD as a real host CE so the dst is written GPU-side and never CPU-promoted).

### ★★★★★ LLM RUNS ON MODE-2 — coherent generation (m568, 2026-06-15)

North-star step 3 FUNCTIONALLY PASSES: a small Qwen2 GGUF (469 MB) runs through the emulated GA106 +
faked GSP + map-on-touch and **generates coherent text** — prompt "Explain in two sentences why GPU
virtualization is useful for cloud computing" → *"GPU virtualization is useful for cloud computing
because it allows cloud providers to deploy virtualized…"*. Real host GR compute (coherent tokens
prove the engine read the right weights), 471 weight/KV buffers backed gpu_only off-BAR1, gpga
FAILED=0, GIVE-UP=1 (graceful), **no new Xid**, GPU D-state 0.

Two bugs were fixed to get here (both on top of map-on-touch):
1. **M6.3b VA squat (investigated, NOT the cause).** The osdesc self-test FIXED-maps guest RAM at
   0x300000000 (libcuda's low-GB range). Gating it off did NOT move the fault → reverted (kept the
   milestone untouched). Eliminated hypothesis.
2. **Doorbell re-sweep cap (THE fix).** `exec_doorbell` re-walks the compute VAS per new submission
   (M5.48c, `m548_newwork` latches GP_PUT) to back newly-mapped working-set leaves, capped at 1000
   total sweeps. cup3/4/7/8 stay under it; a real LLM issues FAR more than 1000 submissions, so a
   buffer mapped after the 1000th sweep (e.g. 0x302000000) never got backed → GR GPC VIRT_WRITE
   fault → wedge. Raised the cap 1000 → 200000 (the sweep is per-new-submission + idempotent via
   `va_seen`, so a high cap is safe). The LLM then ran to coherent generation.

REMAINING = PERFORMANCE, not correctness: ~0.1 tok/s (vs Mode-1's ~60). Two causes, both known/
addressable: (a) the now-thousands of per-submission full-VAS re-sweeps (each O(page-table); a
smarter trigger — sweep only on a genuinely-new MAPPING, not every GP_PUT advance — would cut most);
(b) the ~100 MB/s GPA-window CPU data path (the CE-forward PERF half: forward bulk HtoD/DtoH as real
host CE). Harness `scripts/mode2_diag/m568_llm_maptouch_host.sh` (boots NVKVM_M2CEFWD=1, LLM_TIMEOUT).

#### Sweep trigger fixed → LLM 0.1 → 20 tok/s (m568d, 2026-06-15) ★★★★★

Cause (a) above FIXED. MEASURED first: of 91960 GR-VAS walks in the LLM run, **91932 (99.97%) backed
NOTHING** — pure waste (the 7777-run compute VAS was re-walked 10463×). The walk ran per NEW submission
(`m548_newwork`), but the working set is stable after warmup, so almost every walk was redundant.
FIX = re-sweep ONLY when a GR page table actually CHANGED: `enum_gr_sysmem` records the vidmem PT pages
it walks (`m2_gr_pt_set`, an 8192-slot hash set, rebuilt each sweep); a guest write to any tracked PT
page sets `m2_gr_vas_dirty` (in `nvkvm_fb_write`, behind a lo/hi range pre-filter) → the next doorbell
sweeps. Fault-safe because every PTE/PDE edit writes a tracked page or an ancestor of one (the root PDB
is always tracked), so a mapping is always backed before the engine that uses it runs. Extra triggers:
new GR VAS (`chan_vas_n` grew), budget-truncated walk, and a sparse every-256-submissions insurance net.
RESULT (m568d): re-sweeps **11487 → 41**, walks **91960 → 392**, **Generation 0.1 → 20.1 tok/s (200×)**,
run COMPLETES (rc=0) with coherent output (correctness identical: gpga FAILED=0, no new Xid). Remaining
perf headroom is the ~100 MB/s GPA-window data path (the CE-forward PERF half, still deferred).

### Guest (emulated) BAR1 → 16 GiB — DONE + verified (m564c, 2026-06-14)

Separate from the host-BAR1/CE-forward work: the EMULATED device BAR1 was a 256 MiB stub, capping
how much vidmem the guest CPU can aperture-map (a prod-correctness ceiling). Bumped `bar1_size` to
**16 GiB** (covers the ~11.7 GiB fake FB). Only the chip def + `memory_region_init` reference it;
the GMMU-walked aperture (`bar1_pdb`) is size-agnostic, so only the addressable RANGE grew. Verified
non-regressing: guest enumerated `00:07.0 BAR1 [0x380000000000-0x3803ffffffff 64bit pref]` = 16 GiB
in the q35 above-4G window, booted clean, **cup7 @ 8 MiB → PASS=D1 byte-exact, gpga FAILED=0**. NOTE:
this is the GUEST aperture; the host GPU's 256 MiB BAR1 (the D2 wall) is physical + not resizable here.

---

## PERF investigation — generation 22→60 tok/s (2026-06-15, measure-first)

The LLM runs byte-exact at ~22–24 tok/s. This pass chased the generation bottleneck. Key discipline:
**measure before building** — it overturned the plan twice and saved a multi-cycle wrong build.

### Step 1 — time-share instrumentation (SHIPPED, commit 6217ca3)
Added `qemu_clock` REALTIME timers + `nvkvm_timeshare_dump` (periodic, survives a killed run) around
the hot paths: emulated-CE copy, PRAMIN-window guest-CPU traps, doorbell re-sweep, doorbell forward,
`chan_execute`. Turned the perf guess into data. **The GPA-window data path is a non-issue** (6ms /
17524 calls ≈ 0%) — the earlier ">=100k window traps" worry was a capped *diag* counter, not the path.

### Step 2 — CE copy was per-4-byte; page-batched it (SHIPPED, 6217ca3)
The CE `LAUNCH_DMA` copy loop called `nvkvm_chan_translate` (a full multi-level GMMU walk) +
`nvkvm_fb_read` (O(n) overlay scan) **twice per 4 bytes**. Histogram: 73% of CE copies <4K; the 16 MB
ones are one-time weight loads. Fix (`nvkvm_fb_host_ptr` + page-batched COPY/MEMSET in case 0x300):
translate + resolve once per 4 KiB span, then `memcpy`/`memset`. Result: `ce_emul` **42%→0.7%** (~18×),
model load **34s→28.6s** (~16% faster), cup7 byte-exact, no Xid. **But generation t/s unchanged** — the
CE copy was *not* on the gen critical path; the "42% of gen" was load-phase-weighted.

### CE-forward (host CE runs the copy) — RULED OUT by measurement
Built approach A (forward user-CE channels to the host CE) behind gated `m2cexec` (default OFF,
milestone-safe). Probe (m570): inert — the bulk copies are **PHYSICAL-mode** (`dst_phys=1 verdict=gpga`,
guest-fb-phys meaningless to the host), so a verbatim pushbuffer-forward can't translate them; and 73%
are <4K where host-CE submission overhead would *lose* to a CPU memcpy. Not worth the multi-cycle build.

### Step 3 — the real gen bottleneck = COMPLETION-SYNC LATENCY
- m571 (guest-cpu vs wait): during gen the guest is ~75% **idle**, ~1 of 4 vCPUs busy at ~100%, host
  GPU **util 0%** → single-thread serialized per-op cost, not trap/cache/CPU-saturation.
- m573 (real GPU-path perf, fresh boot + full setup): hot path =
  `common_sampler_sample → llama_synchronize → cudaStreamSynchronize → cuStreamSynchronize → spin on
  clock_gettime` (entry_SYSCALL/SYSRETQ ~27%). **libcuda busy-spins waiting for the completion signal;
  the host GPU finishes in µs (util 0) but the completion takes ~10–20 ms to become visible to the
  guest.** This is "not a million MMIO traps" — it's clock_gettime a million times in *userspace*.
- Unifies with `nvidia-smi` taking seconds and `cudaMemGetInfo` dominating load: all the **same slow
  control/completion round-trip**.
- (Correction: an earlier profile showed CPU matmul — that was an artifact of an ad-hoc llama launch
  that skipped the module/UVM setup, so CUDA failed init and llama fell back to CPU. Ignore it.)

### Clocksource red herring — RULED OUT for throughput
The guest's `kvm-clock` is **not vDSO-capable** here: 500k `perf_counter()` = 500,450 `clock_gettime`
*syscalls* vs **0** under `tsc` (strace-proven). So libcuda's spin traps to the kernel every clock
check (the ~27%). **But a fair 4-boot A/B (m575) showed NO throughput difference**: tsc {19.5, 26.6}
mean ~23.0 vs kvm-clock {26.5, 23.9} mean ~25.2 — run-to-run variance swamps it. ⇒ the clocksource only
wastes CPU; it does **not** set wall-time. This **confirms by elimination** that the limiter is the sema
updating slowly, not the spin's cost.

### NEXT LEVER (measure-first): completion-sema latency
Instrument *submit → sema-visible-to-guest* in QEMU and determine the polled sema's aperture/mapping
(sysmem-coherent vs vidmem/BAR1-UC/stale) to localize the ~10–20 ms. Candidates: spin-poll never sees
the write → kernel fallback; stale CPU cache; wrong/UC mmap; or a poll-interval/interrupt-delivery
delay. Same fix likely also collapses `nvidia-smi`/load latency. Control for the large (~19–27 t/s) run
variance (avg ≥3 runs). Harnesses: `m569` (timeshare), `m570` (cexec probe), `m571` (cpu-vs-wait),
`m573` (gen profile + nvidia-smi), `m574`/`m575` (clocksource A/B). `m2cexec` scaffolding stays gated
OFF (repurpose for a translate-and-reissue CE path, or remove).

# The native `cup2` data plane on a real GA106 — ring, pushbuffer, semaphore

**STATUS: LIVE — measured 2026-08-12 on `vh2` (vast 47373001), GA106 `[10de:2504]`, host driver
open **580.159.04**, kernel 6.8.0-84-generic, NATIVE (no QEMU, no emulated GPU, no kayfabe).**
Harness `tests/mode2/nvdp/nvdp.c` (+ `nvdp_run_host.sh`), repo revision `1e6fc945`.
Raw capture: `traces/native_dataplane_ga106/run_20260812T111414Z/`.
Supersedes nothing. Fills a gap: **this project had no data-plane oracle at all.**

---

## 0. ⊘⊘ READ FIRST — two things that contradict the brief this was commissioned under

**(a) "In our guest the eight GR slots at `+0xf80…+0xff0` are never written" is SUPERSEDED, by
one rung, on the same day.** It was true at `w267`. At `w268` — one variable,
`KAYFABE_GR_ROUTE=refuse→passthrough` — *all eight* GR `SET_REPORT_SEMAPHORE` slots carry
`payload=1` and a **distinct GPU timestamp**, `COMPLETION-WATCH → OBSERVED = 8` against the
control's `NOT-OBSERVED = 8`, and the page fills in real time (`RESUME_HERE_2026_08_12.md:159-161`;
`memory/the_completion_landed_and_cup2_still_hangs.md:33-34`: *"'The guest waits for a semaphore
nobody writes' is dead… **And `cuCtxCreate` still hangs.**"*).
⇒ **The question "why is it written natively and not by us" is no longer the live question.**
Both halves are now known to write it. `CUP2_RC=124` on **both** arms at w266/w267/w268.
⚠ The stale premise is still accurate for the **shipping default**, which is `refuse`.

**(b) The comparison the brief asks for cannot be completed from existing evidence.**
I asked the tree for a decode of the **guest's** `cuMemcpyHtoD` pushbuffer. **It does not exist.**
Nothing in `docs/`, the memory dir, or `src/qemu/nvkvm_gpu_emul.c` records one; the only I2M-shaped
hits are our own emulator's *CE* method table. So the brief's item — *"whether a native run has a
launch method our guest's never had"* — **cannot be answered by comparison today.** This document
supplies the native half; the guest half is an unmeasured gap and is named as such in §7.

---

## 1. Why this capture exists

Every oracle this project owns is **control plane**: the 56-row RM control table, the `nvdiff`
ioctl differential, the ogkm-compiled parsers. And the C artifact's green runs were **CPU copies
with emulator-written completions** (`CLAUDE.md`, *SCOPE THE ORACLE*, 2026-08-12: `m2cefwd` =
*"completion/CPU-copy unchanged"*, `m2cexec` **off**, `finishPayload` completed by the emulator).

⇒ Nobody had ever recorded what a native, unvirtualised `cup2` does on the data plane.

**The insight that made it cheap:** run natively and the ring, the pushbuffer, USERD and the
semaphore are all in the process's **own address space**. No emulator, no BAR window, no
page-table descent — a plain userspace program reads them directly.

---

## 2. ITEM 1 — the ring

`cuCtxCreate` allocates **16** `AMPERE_CHANNEL_GPFIFO_A` (`0xc56f`) channels, alongside 8
`AMPERE_COMPUTE_B` (`0xc7c0`) and 16 `AMPERE_DMA_COPY_B` (`0xc7b5`) objects (100 `RM_ALLOC`s total).

| field | value |
|---|---|
| `gpFifoOffset` (ch0) | `0x200200000` |
| `gpFifoEntries` | **1024** (8 KiB ring) |
| stride between channels | `0x3000` — 8 KiB ring at `+0`, 4 KiB USERD at `+0x2000` |
| `hUserdMemory[0]` | `0x5c000014` — **one** allocation shared by all 16 channels |
| `userdOffset[0]` | `0x2000 + n*0x3000` |
| USERD ch0 | `0x200202000` (`GP_GET` `+0x88`, `GP_PUT` `+0x8c`) |
| mapping | `0000000200200000-0000000200400000 rw-s /dev/nvidia0 +0x0` |

★ **`gpFifoOffset` is a GPU VA and it is also the CPU VA.** The window `mmap(/dev/nvidia0, len
0x200000)` returns exactly `0x200200000`. UVA identity holds for the ring, for USERD and for the
pushbuffer, which is what makes this whole capture possible from userspace.

At the moment of capture ch0's ring held **111 live entries** (`gpe[0]`…`gpe[110]`), all
`priv=0 lvl=1 sync=0`. The entry carrying our copy is **`gpe[110]`**:

```
gpe[110] @0x200200370 = 0044c544 00003e02 -> pbuf 0x020044c544 len=15 dw
```

Raw: `raw/ring.bin`, `raw/ring_post.bin` (8 KiB each, the full ring both sides of the copies).

---

## 3. ITEM 2 — the pushbuffer, and it is not what we assumed

The pushbuffer lives in a **different** aperture from the ring:
`0000000200400000-0000000203400000 rw-s /dev/nvidiactl` — a 48 MiB **sysmem** window.

The whole submission for a 4-byte `cuMemcpyHtoD` is **15 dwords / 60 bytes**:

```
+0    20022062  INC    sub=1 mth=0x0188 cnt=2
       [0] 0x0188 = 0x00007f4f   NVC7C0_OFFSET_OUT_UPPER
       [1] 0x018c = 0x66200000   NVC7C0_OFFSET_OUT          -> dp = 0x7f4f66200000  (match)
+12   20022060  INC    sub=1 mth=0x0180 cnt=2
       [0] 0x0180 = 0x00000004   NVC7C0_LINE_LENGTH_IN      (4 bytes)
       [1] 0x0184 = 0x00000001   NVC7C0_LINE_COUNT
+24   2001206c  INC    sub=1 mth=0x01b0 cnt=1
       [0] 0x01b0 = 0x00000041   NVC7C0_LAUNCH_DMA
+32   6001206d  NONINC sub=1 mth=0x01b4 cnt=1
       [0] 0x01b4 = 0xabcd1234   NVC7C0_LOAD_INLINE_DATA    <== THE PAYLOAD, A LITERAL
+40   200426c0  INC    sub=1 mth=0x1b00 cnt=4
       [0] 0x1b00 = 0x00000002   SET_REPORT_SEMAPHORE_A (offset upper)
       [1] 0x1b04 = 0x0440fff0   SET_REPORT_SEMAPHORE_B (offset lower)
       [2] 0x1b08 = 0x0000006f   SET_REPORT_SEMAPHORE_C (payload)
       [3] 0x1b0c = 0x00000000   SET_REPORT_SEMAPHORE_D (operation)
```

★★★ **A 4-byte `cuMemcpyHtoD` on native hardware uses NO copy engine.** `ce_launch_dma=0`,
`ce_setsem=0`, `host_sem=0`. It is the **compute class's inline-to-memory (I2M) unit** on the GR
channel, and **the data is a literal in the pushbuffer** — which is exactly the shape this tree
already recorded for the guest (`memory/the_payload_is_a_literal_in_the_guests_pushbuffer.md`).
It also means "`cup2` is a CE round-trip" is, at least for the HtoD half, **wrong on native**.

Operand decode (`ogkm-580.159.04 clc7c0.h:212-243`):
- `LAUNCH_DMA = 0x41` → `DST_MEMORY_LAYOUT = PITCH`, `COMPLETION_TYPE = FLUSH_DISABLE`,
  `SYSMEMBAR_DISABLE = TRUE`, `INTERRUPT_TYPE = NONE`. ⇒ the I2M itself releases nothing.
- `SET_REPORT_SEMAPHORE_D = 0` → `OPERATION = RELEASE`, `STRUCTURE_SIZE = FOUR_WORDS`,
  **`AWAKEN_ENABLE = 0`** ⇒ **polled, not interrupt-driven**. Identical to the guest
  (`RESUME_HERE_2026_08_11.md:24-26`).

The second watched copy's segment (`raw/pushbuffer_last.bin`) is byte-identical in shape with
`LOAD_INLINE_DATA = 0x5a5a1234` and `SET_REPORT_SEMAPHORE_C = 0x70`.

---

## 4. ITEM 3 — the report semaphore

| property | measured value |
|---|---|
| **VA** | **`0x2_0440_fff0`** |
| **page offset** | **`+0xff0`** |
| **aperture** | **SYSMEM (host RAM)** — phys `0x103ea1ff0`, in **no** GPU BAR |
| declaring class | `NVC7C0` (**GR / compute**) `SET_REPORT_SEMAPHORE` |
| declaring channel | ch0, the `AMPERE_CHANNEL_GPFIFO_A` running the I2M |
| structure | 4 words `[payload, pad, ts_lo, ts_hi]`, 16 bytes |
| declared payload | `0x6f`, then `0x70` |
| landed payload | `0x6f`, then `0x70` — **`landed == declared`** |

The aperture is measured, not inferred: the harness takes the PFN from `/proc/self/pagemap` and
compares the physical address against the GPU's own BARs read from
`/sys/bus/pci/devices/0000:00:07.0/resource` (`BAR0 0xc0000000-0xc0ffffff`,
`BAR1 0x3800000000-0x380fffffff`, `BAR3 0x3810000000-0x3811ffffff`).
⚠ Note the two aperture views **disagree by design**: by *mapping* it is a `/dev/nvidiactl`
window, which looks device-ish; by *physical address* it is ordinary host RAM. The physical test
is the authoritative one, and it is why an ordinary volatile load can see the write.

### The page is a 16-slot pool, and our slot is slot 15

`raw/sempage_0.bin`, bytes `+0xf00…+0xfff`:

```
+0xf00: 01 00 00 00 00000000 00 70 95 92 88 0a cb 18
+0xf10: 01 00 00 00 00000000 20 69 95 92 88 0a cb 18
...
+0xf70: 60 00 00 00 00000000 60 66 30 94 88 0a cb 18
+0xf80: 01 00 00 00 00000000 00 04 a2 92 88 0a cb 18
...
+0xff0: 6f 00 00 00 00000000 00 70 95 92 88 0a cb 18   <== ours
```

Sixteen 16-byte slots at `+0xf00…+0xff0`, **every one of them carrying a payload and a distinct
GPU timestamp**. This is the same pool the C-era work already identified:
`docs/design/how_the_c_passed_the_gr_wall.md:91-95` — *"16 per-channel semaphores in a sysmem pool
at guest VA `0x20440ff00..0x20440fff0`, 16 slots × 0x10"*.

★★★ **The native GA106 and our guest put the report semaphore at the SAME virtual address,
`0x2_0440_fff0`, in the SAME slot of the SAME 16-slot sysmem pool.** That address is not something
we chose or something the emulator produced — it is what libcuda picks on bare metal.

---

## 5. ITEM 4 — who writes it

⊘ **A watchpoint cannot answer this and was not used to.** A GPU semaphore release is a **DMA
write**: it never touches the CPU MMU, and x86 debug registers watch CPU accesses only. Silence is
the expected behaviour of *any* DMA and proves nothing. The harness therefore treats the
write-breakpoint strictly as a **negative control** and leads with three positive instruments.

**(a) The GPU's own clock — decisive.**

| | value |
|---|---|
| report ts at the discovery copy | `1786533258906766336` |
| report ts after the watched copy | `1786533259629691904` |
| **GPU-clock delta** | **0.722925568 s** |
| CPU wall-clock delta between the same two submissions | **0.722895 s** |
| **agreement** | **31 µs over 0.72 s — 43 ppm** |

★ The value in the report is a **live hardware nanosecond clock that tracks our wall clock to
43 ppm**. It is not a constant, not a copied payload, and nothing on the CPU side has access to
it. This is the strongest single piece of evidence in the capture.

**(b) The payload matches what the pushbuffer declared.** `SET_REPORT_SEMAPHORE_C = 0x6f` →
landed `0x6f`; next submission declares `0x70` → landed `0x70`. That ties the write to *that*
submission, not to some other writer that happened to touch the page.

**(c) Negative control, with its known-positive.**

```
GR_SEM cpu-store count = 0   (4 thread breakpoints)
GP_PUT cpu-store count = 1   (4 thread breakpoints)   <== KNOWN-POSITIVE
```

A hardware write-breakpoint is armed on **every thread** of the process (`perf_event_open`,
`PERF_TYPE_BREAKPOINT`, `HW_BREAKPOINT_W`, one fd per tid — a per-process event would miss
libcuda's workers). `GP_PUT` fires exactly once, so the instrument is demonstrably live; the
semaphore never fires. ⚠ On its own that zero would mean nothing — it is interpretable only
because the known-positive is non-zero, and even then it only **fails to refute** GPU authorship.

⇒ **Verdict: the GPU wrote it.** Established by (a) and (b); (c) merely fails to refute.

---

## 6. ITEM 5 — `GP_GET` vs `GP_PUT`, sampled

All 16 channels' cursor pairs **and** all 16 semaphore slots are read in the **same loop
iteration**, each sample carrying its own `CLOCK_MONOTONIC` stamp (so a dump at teardown still
reports time, not just order). Full series: `raw/samples.csv`.

```
t=2.506477  ch0 111/111   slot15 = 6f
t=2.506616  ch0 111/112   slot15 = 70      <- PUT advanced; slot already released
t=2.506633  ch0 112/112   slot15 = 70      <- GET catches up
t=2.506666  ch12  2/2     slot3  = 2       <- the DtoH, on a DIFFERENT channel
```

**Firm:** `GP_PUT` (the CPU doorbell) precedes both the release and the `GP_GET` advance by
~140 µs. Steady state is `GP_GET == GP_PUT`, i.e. fully drained.

**Not firm, and I will not claim it:** in both runs the semaphore is observed already released in
a sample where `GP_GET` still trails, with `GP_GET` advancing in the *next* sample ~17 µs later.
⚠ But within a sample the cursors are read **before** the slots, which biases in exactly the
direction of that observation, and 17 µs is one sample period. **The instrument cannot order the
release against the `GP_GET` advance.** What it does establish is weaker and still useful: the
release does **not** require `GP_GET` to have already advanced past the entry — no sample ever
showed `GP_GET` moving first.

★ **The `DtoH` runs on a different channel.** `ch[0]` `GP_PUT 111→112` and `ch[12]` `GP_PUT 1→2`;
ch[12] releases into **slot 3 (`+0xf30`)**. So the two halves of `cup2` use two channels and two
slots of the same pool. Any guest-side instrument that watches one channel will miss half of it.

---

## 7. Native vs guest — what actually compares

| axis | native GA106 (this capture) | our guest | source |
|---|---|---|---|
| semaphore VA | `0x2_0440_fff0` | `0x2_0440_fff0` | `the_fb_crossing_is_the_majority_not_the_successor.md:9` |
| page offset | `+0xff0` | `+0xff0` | same |
| pool shape | 16 × 16 B at `+0xf00…+0xff0` | 16 × 16 B, same range | `how_the_c_passed_the_gr_wall.md:91-95` |
| aperture | **sysmem**, phys not in any BAR | **GuestRam** `gpa=0x43b0fff0` | `five_arming_flags_nobody_carried_forward.md:18-22` |
| declaring class | `NVC7C0` GR `SET_REPORT_SEMAPHORE` | `NVC7C0` GR `SET_REPORT_SEMAPHORE` | `RESUME_HERE_2026_08_12.md:221-222` |
| `AWAKEN_ENABLE` | 0 → polled | 0 → polled | `RESUME_HERE_2026_08_11.md:24-26` |
| report structure | 4 words `[payload,pad,ts_lo,ts_hi]` | 4 words, same | `the_gpu_wrote_the_completion_semaphore.md:48-49` |
| slots written | **all 16**, distinct GPU ts | `refuse`: GR 8 zero / `passthrough`: **all 8 written**, distinct GPU ts | `RESUME_HERE_2026_08_12.md:159-161` |
| `GP_GET` vs `GP_PUT` | `GET == PUT`, drained | `refuse`: `GET=0 PUT=1` for 167 s / `passthrough`: `GET` catches `PUT` on all 8 | `the_completion_landed_and_cup2_still_hangs.md:17` |
| `cuMemcpyHtoD` pushbuffer | **GR I2M + `LOAD_INLINE_DATA` literal, no CE** | **NEVER DECODED** | — |
| `cuMemcpyDtoH` | separate channel, separate slot | not measured | — |

**★ The first difference that could explain a silence — and the honest answer.**
On every axis the capture *can* compare, native and guest **agree**: same VA, same offset, same
pool, same aperture class, same declaring class, same polled release, same report structure.
The only measured divergence left is the one already known and already closed by `w268`:
on the `refuse` arm the host engine **never fetches** (`GET=0 PUT=1` for 167 s), so nothing
releases; on `passthrough` it fetches and the GPU writes all eight slots.
⇒ **The capture does not support naming a *new* first difference.** Per the brief, I am saying
that rather than picking the most appealing candidate. The genuinely open item is not the
semaphore at all — it is that `cuCtxCreate` still hangs *after* the completion lands.

---

## 8. ⊘ What this capture CANNOT show

- **Nothing about the emulated path.** It is a native reference. It says what hardware does; it
  says nothing about what our device does or should do.
- **It cannot order the semaphore release against the `GP_GET` advance** (§6). ~17 µs apart, one
  sample period, with the intra-sample read order biasing toward the observed answer.
- **It cannot see the doorbell itself.** `GP_PUT` is observed changing and its CPU store is
  counted, but the BAR write that rings the channel is not located or watched.
- **It cannot show the guest's `cuMemcpyHtoD` method stream** — that measurement does not exist
  anywhere in this tree (§0b). The native/guest pushbuffer row above is deliberately empty.
- **It cannot see interrupts or event delivery.** `AWAKEN_ENABLE = 0`; this workload polls.
- **`cuMemcpyDtoH` is only partly characterised**: we know it lands on ch[12] and releases slot 3;
  its method stream was not decoded (it is not the segment the anchor scan finds).
- **One GPU, one driver, one workload.** GA106 / 580.159.04 / a 4-byte copy. A larger copy will
  very likely leave the I2M path for a real CE `LAUNCH_DMA`, and nothing here bounds where.
- **The `pad` word is always 0** in every slot observed; whether hardware ever writes it is
  unmeasured, not measured-zero.

---

## 9. Reproducing

```
scp tests/mode2/nvdp/nvdp.c tests/mode2/nvdp/nvdp_run_host.sh <gpu-host>:/workspace/nvdp/
ssh <gpu-host> 'NVDP_GITREV=$(git rev-parse HEAD) bash /workspace/nvdp/nvdp_run_host.sh'
```

`nvdp.c` is self-contained: no CUDA toolkit, no `cuda.h`, no `LD_PRELOAD`. It `dlopen`s
`libcuda.so.1` and interposes `ioctl`/`mmap` **from the executable itself** (executable symbols
win over libc for libcuda's PLT calls). `nvdp_run_host.sh` refuses to run if a guest boot is in
flight — the GPU is a serial resource — and writes a start marker plus an explicit exit-status
terminator so a truncated artefact is detectable.

### Four instrument defects this harness hit, all measured, all now encoded in the source

1. ★★ **`process_vm_readv` cannot read a `VM_PFNMAP` mapping.** It goes through
   `get_user_pages_remote`, which refuses device mappings — so **every `/dev/nvidia0` BAR window,
   which is where the ring and USERD live, read back as "unreadable"**, and two runs concluded the
   ring was not in the address space. It was; the safe reader could not see it. Fix: fall back to
   a **direct volatile load fenced by a `SIGSEGV`/`SIGBUS` handler** (per-thread `sigjmp_buf` — the
   poller reads too). ⚠ Same shape as this tree's recurring lesson: *an empty capture is evidence
   of nothing.*
2. ★★ **The `RM_ALLOC` header must be sampled BEFORE the call.** Read after, `paramsSize` comes
   back **0** on all 16 channel allocs — RM clobbers the header — so a post-call read reports
   *"this call had no parameters"* for a call that plainly had them. All 16 channels silently
   vanished. Fix: pointer and size from the **pre** image, handle from the **post** image
   (`hObjectNew` is `[OUT]`).
3. ★ **`NVC36F_DMA_METHOD_ADDRESS` is bits `11:0` holding `addr>>2`, not the Kepler-era `12:2`**
   (`ogkm clb06f.h:203`). Decoding with the old field mis-names every method above `0x1FFC` —
   which is most of the GR class, including `SET_REPORT_SEMAPHORE` at `0x1b00`. Caught before the
   first run.
4. ★ **A census zero needs a known-positive, twice over.** The class census (printing *every*
   `RM_ALLOC` class) is what proved the channels were there while the decoder dropped them; the
   `GP_PUT` breakpoint is what makes the semaphore's zero interpretable.

---

## 10. Artifacts

`traces/native_dataplane_ga106/run_20260812T111414Z/`

| file | what |
|---|---|
| `nvdp.log` | the full annotated capture, 613 lines, every line stamped with its **emission** time |
| `raw/ring.bin`, `raw/ring_post.bin` | ch0's 8 KiB GPFIFO ring, before and after the watched copies |
| `raw/pushbuffer.bin` | the 60-byte I2M + report-semaphore segment (`gpe[110]`) |
| `raw/pushbuffer_last.bin` | the second submission, payload `0x5a5a1234` |
| `raw/sempage_0.bin` | the whole 4 KiB semaphore page, including the 16-slot pool |
| `raw/pb_anchor_context.bin` | 4 KiB of pushbuffer around the anchor |
| `raw/samples.csv` | the full polled series: `t, gp_get, gp_put`, and all three semaphores' 4 words |
| `status.txt`, `provenance.txt` | serial-resource check, GPU/driver, source sha256, revision, ENOSPC check |
| `dmesg.log`, `gpu.txt`, `kernel.txt` | environment, persisted beside the run (the serial log is never where the driver's output is) |

# Mode-2 #12 — 2nd-CUDA-context hang: root cause (CE completion sema)

Status: **RESOLVED 2026-07-05 (cont.34) — `cupctx2_min` rc=0 (CTX1 AND CTX2
create+destroy), cup2 rc=0 byte-exact.** See cont.34 at the tail.

Status: diagnosed 2026-06-17 (root cause proven end-to-end via an instrumented
full-source guest driver). The wrap-wedge *layer* is fixed and committed
(`37d15c5`); the CE-completion *layer* documented here is the remaining blocker.

> ### UPDATE 2026-06-19 (cont. 4) — SOURCE + LOG forensics: it's a VAS-RESOLUTION miss (transient-VAS free), NOT sharing/lifecycle; map-loss pinpointed
>
> Read the open RM source (`ce_utils.c`, `channel_utils.c`, `mem_utils_gm107.c`, `video_mem.c`)
> + mined the persisted `cupctx2` QEMU log. Six facts, then the exact map-loss point.
>
> **Architecture (source).** CeUtils is **kernel RM** (CPU-side), via
> `rmapiGetInterface(RMAPI_GPU_LOCK_INTERNAL)`; it allocates its **own** `NV01_ROOT` client and
> its **own fresh VAS**, `bUseBar1=TRUE`. The scrub channel buffer (gpfifo+pushbuffer+semas, one
> memdesc) is vidmem by default (`ADDR_FBMEM`). The teardown wait `channelWaitForFinishPayload`
> is a **kernel CPU busy-poll** (`while(READ_CHANNEL_PAYLOAD_SEMA < target)`, 4 s timeout,
> services scrubber interrupts but completion authority = the sema *value*). The sema is read by
> `channelReadChannelMemdesc` → `memmgrMemDescBeginTransfer(USE_BAR1)` → `MEM_RD32(pbCpuVA +
> finishPayloadOffset)` — a **CPU read through BAR1 of the channel buffer's vidmem**, at
> `pbGpuVA + finishPayloadOffset` (`= gpfifo_va + 0x8004`).
>
> **CORRECTION to cont.-era claims — who allocates FB phys.** *We do NOT assign vidmem phys.* The
> **guest's CPU-side PMA is the sole FB allocator**: `vidmemConstruct` → `pmaAllocatePages`
> (unconditional, CPU-side); `NV_RM_RPC_ALLOC_VIDMEM` is gated `!IS_GSP_CLIENT` (a vGPU alias
> step, not the allocator) and our GSP-client guest skips it. Our fake GSP only declares the heap
> *bounds* at boot (`pmaRegisterRegion`); the guest picks every offset within them and writes
> them into PTEs (GPU VAS + BAR1). **One allocator ⇒ a GSP-vs-driver phys overlap is structurally
> impossible** — so the `0x31f0000` "collision" is **not** sharing, **not** temporal reuse, **not**
> a lifecycle bug. It is our resolver guessing.
>
> **Map-loss pinpointed (log).** Hang channel = scrub, client `0xc1e00007`, gpfifo VA
> `0x120064000`, **instance block `0x2efa6e000`**. The mapping is lost at three points:
> 1. **Transient-VAS free.** Guest allocs VASPACE `0x0c` → we capture PDB `0x2efba5000` (L3476) →
>    guest **frees `0x0c`** (L3482) → *then* allocs the channel (L3523). So our recorded `cli_vas`
>    for this client is the dead `0x2efba5000`; every probe FAULTs. The **sibling** scrub channel
>    (`0xc1e00008`, VAS `0x0a`/PDB `0x2efa6c000`) kept its VAS, resolved via `res=cli_vas`, and its
>    finishPayload `0x42006c004` → **sysmem `0x144a48004`** was written correctly. *Same code path,
>    correct result — the only difference is the transient free.*
> 2. **Instance-block PDB reads empty.** `M5.14` reports "PDB empty (GSP-managed)" for these
>    channels — `instblk + RAMIN_PDB_off` is zero in our mirrored FB, so the bulletproof fallback
>    (read the channel's own GMMU root) yields nothing.
> 3. **The forge bypasses VAS resolution** and uses the `bar1_wpg` MRU → `finFB=0x31f8004`, the UVM
>    channel's buffer (`0x1210d0000`/phys `0x31f0000`). Collision → guest reads its real
>    finishPayload elsewhere → `ce_utils.c:349`.
>
> **Fix anchors (both match the address-table directive; retire `bar1_wpg` for finishPayload):**
> (1) **Instance-block PDB** — make `instblk 0x2efa6e000 → PDB` resolve, then
> `finishPayload phys = walk(PDB, gpfifo_va + 0x8004)`; immune to VAS-handle churn, general to
> every channel. (2) **Don't lose the real VAS** — bind the channel to the VAS its `c56f` params
> reference (the `cli_vas` capture that already works for the sibling), don't drop it on the
> transient's free. **Open read-only question gating (1):** *why does RAMIN+0x200 read empty here*
> — wrong offset for this channel class, not mirrored into our FB, or genuinely GSP-populated (our
> fake GSP never wrote it)? That's the next step. (See `[[mode2_address_table_of_truth]]`.)
>
> ### UPDATE 2026-06-18 (cont. 3) — FIX ATTEMPT BENCH-DISPROVEN: the wall is memslot/fb_write INCOHERENCE, and owning-client overlay-release is UNSAFE
>
> Built + bench-ran the agreed first fix (release a root-freed client's GPGA overlays so
> the scrub finishPayload de-aliases to base FB) on `cupctx2` (2-context #12 repro), fresh
> QEMU, forge active. **Result: STILL HANGS** — CTX1 matmul byte-exact PASS, CTX1
> `cuCtxDestroy` returns, then CTX2 `cuCtxCreate` hangs (rc=124). Guest dmesg is unchanged:
> `scrubberDestruct: Timed out` (4 s) + `nvAssertFailedNoLog … lastCompletedPayload ==
> lastSubmittedPayload @ ce_utils.c:349`. Two hard facts fell out:
>
> 1. **The forge fires with the EXACT right values, and the guest still doesn't see them.**
>    45 `#12 FORGE finishPayload` lines on the scrub channel (`client 0xc1e00007`,
>    `gpfifo=0x120064000`): `finFB=0x31f8004` written `0→1→…→8`, then the ring re-fragments
>    mid-run (`b1off 0x128004 → 0xa8004`) and `finFB=0x3138004` written `9→10→…→20+` — a
>    clean monotonic `lastSubmittedPayload`. So the *value* and the *channel* are correct;
>    the **delivery target is not the memory the guest reads.** The guest read is
>    memslot-served (poll-spin still 0), so QEMU's `nvkvm_fb_write` (which routes through
>    `nvkvm_fb_host_overlay` → an overlay `host_qva`, else base `fb_page`) writes a
>    *different* host backing than the KVM memslot the guest's BAR1 read resolves to. This
>    incoherence — not the value, not the offset — is THE remaining wall.
>
> 2. **Owning-client overlay-release is UNSAFE (cross-client sharing), so it was reverted.**
>    The release fired for UVM `client=0xc1d00003`, dropping the overlay for FB `0x3130000`
>    — **but the scrub channel (`client 0xc1e00007`) reads its ring/finishPayload from that
>    same FB phys.** Freeing the backing on the *owner's* free yanks it out from under a
>    still-polling *different* client — exactly the cross-VAS/cross-client sharing the
>    address-table model forbids without a refcount over **all** referencing clients/VAS
>    (the user's "phys freed only when every reference drops"). The naive
>    `m2_objs[].client == fClient` scope violates that. Reverted to the committed baseline
>    (only an explanatory NOTE remains in `ctx_free_drop`).
>
> **Where this leaves the fix (pick one; (b) is the principled "real not fake" path):**
> - **(a) Coherent backing for the scrub ring.** Stop the scrub channel's FB phys from
>   COLLIDING with a UVM client's object (the `M5.24` map-FAIL is the symptom): give the
>   GSP-managed scrub its OWN host object + memslot so its finishPayload page is written and
>   read through the SAME backing. Forge then lands where the guest reads.
> - **(b) Execute the scrub CE on the host.** Forward the GSP-managed scrub channel's
>   pushbuffer (resolve its VAS via the address table, not the `bar1_wpg` heuristic) so the
>   host GPU's real `SET_SEMAPHORE` writes the real finishPayload through the guest-coherent
>   backing. This is `mode2_real_forward_not_fake` + the address-table execution piece.
> - **(c) Memslot-punch.** Exclude the finishPayload page from the non-trapping memslot so
>   the guest's poll TRAPS into `baraperture_read`, where QEMU returns `c->fin_payload`
>   directly (no backing-coherence needed). Smallest blast radius; least "real".
>
> Incidental blockers cleared this session (not #12): the VAST guest auto-upgraded its
> kernel to `6.8.0-124` while `nvmods` are vermagic `6.8.0-117` (`Invalid module format` →
> `cuInit=100`) — pinned grub to 117 + QEMU restart (see `[[mode2_bench_kernel_drift]]`);
> and the `mode2_diag` orchestrators slurp the piped script via a stdin-less inner `ssh`
> in the boot-wait loop (silent truncation) — fixed with `</dev/null`.
>
> ### UPDATE 2026-06-17 — CORRECTED ATTRIBUTION (ground truth wins): the blocker is the finishPayload `0x…6c004`, NOT `0x121000010`
>
> **Correction of a same-day mis-attribution.** An intermediate write-up of this section
> (commit `99bb828`) named `va=0x121000010` (client `0xc1e00007`) as the root cause —
> the backward-aliased write the resolver mis-routes to a foreign VAS. That is a **real but
> SEPARATE** phenomenon (the UVM *tracking* sema, the `#12-L3` line, already contained by the
> `37d15c5` backward-defer). It is **not** what `scrubberDestruct` waits on. The
> **instrumented-guest ground truth** (counters printed from *inside* the guest driver at the
> hanging destruct — stronger than any QEMU-side parse, because only the guest knows what its
> destruct polls) is decisive:
> ```
> destruct #1 PASSES: lastSub=2  hwsema=2  finVa=0x42006c004  bUseBar1=0  (sysmem finishPayload — parser writes it → match)
> destruct #2 HANGS:  lastSub=63 hwsema=0  finVa=0x12006c004  bUseBar1=1  (VIDMEM finishPayload — NEVER written → timeout)
> ```
> **EXACT ROOT CAUSE:** the hanging channel is gpfifo **`0x120064000`** (pbGpuVA
> `0x120000000`, a GSP-managed UVM CE channel). `chan_exec` shows **`picked_pdb=0`** → we
> cannot resolve its pushbuffer VAS, so we *drain* its ring (gp_get 0→63) but **do not parse
> its methods**. Its CE `SET_SEMAPHORE`/release to the **finishPayload sema
> `0x12006c004`** (= pbGpuVA + `finishPayloadOffset 0x6c004`, in **VIDMEM**, `bUseBar1=1`)
> therefore **never executes** → the sema reads 0 → `ceutilsDestruct`→
> `channelWaitForFinishPayload(63)` times out (4 s) → `ce_utils.c:349` → device `StateUnload`
> botched (last-context destroy de-inits the whole GPU) → CTX2 cold `StateLoad` hangs.
> Confirmed both nights: **zero writes to `0x12006c004`** in the QEMU log; the analogous
> *sysmem* sibling gpfifo `0x420064000` DOES get its `CE_SEM_RELEASE → 0x42006c004` and
> passes (`bUseBar1=0`, parser resolves + writes it).
>
> Today's `m2trace` is **consistent** with this once read correctly: a re-mine query for any
> `SET_SEMAPHORE` targeting `0x12006c004` came back **empty** (never parsed), while the
> `0x121000010` releases ARE parsed — i.e. we parse *part* of the channel's ring but miss the
> GP entry carrying the finishPayload release. `picked_pdb=0` is why.
>
> **Why single-context (LLM/cup8) passes:** the finishPayload that goes unwritten is on a
> **vidmem, `bUseBar1=1`** channel that only appears at the *2nd* context's device-reinit
> teardown; the sysmem sibling (`bUseBar1=0`) we DO write covers the 1st.
>
> **The fix targets `0x12006c004`, not `0x121000010`:**
> 1. **Parse the GSP-managed channel's finishPayload release even when `picked_pdb=0`** — via
>    the `bar1_wpg` FB-backing fallback (the guest BAR1-wrote the ring to vidmem we mirror);
>    then `nvkvm_chan_sem_wr32` writes the vidmem sema via `fb_write` (it already handles
>    vidmem). The finishPayload VA→FB mapping is already known (the guest's own
>    `DIAG BAR1 WR off=0x12006c → FB 0x31f006c`). The missing piece is **parsing the
>    release**, not resolving a sema VA. ⇒ `bar1_wpg` is **load-bearing** here — the earlier
>    "pin the own VAS" attempt regressed precisely by *disabling* it.
> 2. **OR forge-complete at teardown** (the "wait if real work, else complete now" rule):
>    finishPayload is at a fixed `+0x8004` offset in the same buffer we already resolve for
>    `GP_PUT`; on a `bUseBar1` scrub channel whose ring we can't fully parse, write
>    `finishPayload = lastSubmittedPayload`. Safe because the scrubbed memory is being freed.
>
> The `0x121000010` foreign-VAS aliasing (MISS=FAULT, address-table population) is still a
> valid cleanup — but it is a **separate** task, not the `#12` destruct blocker. Do not
> conflate them again.

The clean fix is the address table — see `mode2_address_table.md`.

> ### UPDATE 2026-06-18 — forge MECHANISM bench-validated; target-DELIVERY is the remaining wall
>
> Implemented the "complete the no-op scrub" forge (option 2) and ran cupctx2 on the
> vast.ai vh bench (RTX 3060) over two rounds. **The mechanism is validated; the value is
> exact; CTX1 passes; no UVM poison. cupctx2 still hangs (rc=124)** — the forged value never
> reaches the backing the guest actually polls. Precise findings (the value of this round):
>
> - **Forge fires on EXACTLY the right channel/value.** `#12 FORGE finishPayload ch[0]
>   gpfifo=0x120064000 … client=0xc1e00007` — the CeUtils scrub channel from the instrumented
>   ground truth — with a **monotonic, exact** value (`c->fin_payload` = cumulative GPFIFO
>   entries == `lastSubmittedPayload`, since `channelPbInfo.payload = lastSubmitted+1` and one
>   entry per op, `ce_utils.c:611`). Never a backward write ⇒ no `uvm_gpu_semaphore` poison.
>   CTX1 fully passes; the hang is unchanged at CTX2's first `cuCtxCreate`.
> - **The guest reads finishPayload via a non-trapping MEMSLOT — CONFIRMED (round 3).** A
>   poll-spin detector with the LOFB window REMOVED (fire on any address read ≥2000× in a row)
>   fired **0** times during the 4 s timeout, while `DIAG BAR1 WR` fired **1032**. So the
>   finishPayload reads NEVER reach a QEMU trap at all — they are served by a KVM RAM memslot
>   mapped into the guest's BAR1. **QEMU is structurally blind to the poll**, and
>   `nvkvm_fb_write(fin_fb,…)` only reaches it if `fin_fb`'s overlay IS that memslot's host RAM
>   — which M5.16 aliasing breaks. Proof the page is wrong, not just lagging: the resolved page
>   `0x31f8004` reached the **full** count (45) yet the guest still timed out. ⇒ **No QEMU-side
>   read-trap diagnostic can locate the target; resolution must come from the guest's BAR1
>   mapping authority (address table) or a guest-side oracle.**
> - **FB+0x8004 is wrong (buffer is FB-fragmented); BAR1-offset+0x8004 via the BAR1 PTEs is
>   the right primitive but still hits the memslot mismatch.** The ring pages jump
>   `0x31f0000 → 0x3130000` in FB, so `chan_gpfifo_phys+0x8004` lands on a foreign fragment.
>   Resolving through `walk_pdb(bar1_pdb, chan_gpfifo_bar1off + (gpfifo_va&0xfff) + 0x8004)`
>   correctly absorbs the fragmentation — but the guest still reads elsewhere (memslot).
> - **M5.16 cross-channel ALIASING.** The shared `bar1_wpg` pool makes M5.16 sometimes pick
>   the **compute** channel's ring page (`b1off=0xa0000 → FB 0x3130000`, the known compute
>   gpfifo) as the scrub channel's base, so the monotonic sequence splits across the real page
>   and a wrong one — and a forge write can corrupt the compute channel's buffer (a real
>   regression risk; the forge is therefore gated behind `m2trace`, default OFF, NOT a default
>   fix yet).
> - **`M5.24 GPFIFO double-mmap … phys=0x31f0000 sz=0x8000 → map-FAILED`** for this channel,
>   and finishPayload (`+0x8004`) is past the `0x8000` gpfifo span regardless.
>
> **The remaining problem is now precise:** deliver the (correct, exact) forged value to the
> **exact backing the guest polls** (a memslot we don't currently write), and stop M5.16 from
> aliasing other channels' pages into this channel. Both are the "one address table of truth"
> work (`mode2_address_table.md`): per-channel-isolated VA→backing with the guest's BAR1
> mapping as the authority. **Next experiment:** stage the instrumented 580.159.04 guest
 (the oracle that prints the finishPayload CPU-VA + the value it reads each poll) to nail the
> exact read backing in one run, rather than guessing QEMU-side. Forge code lives in
> `nvkvm_chan_execute`'s doorbell loop (the `#12 FORGE` block + `c->fin_payload` +
> `chan_gpfifo_bar1off` + `bar1_wpg[].off`), gated behind `m2trace`.

> ### UPDATE 2026-06-18 (cont.) — ROOT CAUSE FOUND by log forensics: an FB-physical COLLISION (no oracle needed)
>
> Mining the persisted QEMU log (zero extra bench rounds) decoded the layout AND the bug:
> - **BAR1↔FB is contiguous-linear here** (`off 0x120000→FB 0x31f0000`, `off 0x12006x` are the
>   GP ring entries, e.g. `0x20000648 / 0xd801`), so `finishPayload = gpfifo_va + 0x8004 →
>   FB 0x31f8004` IS geometrically correct. The forge writes the right *offset*.
> - **But emulated-FB phys `0x31f0000` is CLAIMED BY TWO DIFFERENT GUEST CHANNELS** — the
>   decisive lines:
>   ```
>   M5.7 back_and_map VA=0x1210d0000 phys=0x31f0000 size=0x10000 client=0xc1d00001  OK PLACED
>   M7  gpga_obj      va=0x1210d0000 gpga=0x31f0000 cpu_qva=0x77a7e5e49000 obj=8
>   M5.24 GPFIFO      va=0x120064000 phys=0x31f0000 sz=0x8000 client=0xc1e00007  -> map-FAILED
>   ```
>   The **CeUtils scrub channel** (`client=0xc1e00007`, VA `0x120064000`) and a **UVM kernel
>   channel** (`client=0xc1d00001`, VA `0x1210d0000`) both resolve to FB phys `0x31f0000`. The
>   scrub channel's `M5.24` double-mmap FAILED *because the UVM channel already owns that phys*.
> - **Consequence:** the finishPayload page `0x31f8004` is inside the UVM channel's `gpga_obj`
>   `[0x31f0000, 0x3200000)`, so `fb_write(0x31f8004)` lands in the **UVM channel's** host RAM
>   (`cpu_qva 0x77a7e5e49000+0x8004`). The guest reads the **scrub** channel's finishPayload
>   from *its* own (different) mapping → never sees the forge. Hang persists.
>
> **This is the page-reuse / free-realloc aliasing the `#12-L3c` note predicted, now proven
> concretely:** when the scrub channel tears down, its vidmem (`0x31f0000`) is reused by the
> UVM channel — but our FB-phys model lets both VAs alias the same emulated page, so a write
> to "the scrub finishPayload" actually hits the other channel's buffer. **No instrumented
> oracle is needed** — the collision is fully visible in the QEMU log.
>
> **THE FIX is squarely the address table (`mode2_address_table.md`): per-channel-isolated
> VA→FB-phys so two live channels can never alias the same emulated-FB page, plus free/realloc
> lifecycle tracking so a torn-down channel's phys isn't silently reused under a still-polling
> owner.** Once VA→backing is per-channel-correct, the (already-exact) forge writes the right
> RAM and the guest's poll completes. The forge is the *completion policy*; the address table
> is the *delivery*. Recommended order: fix the FB-phys collision first (it likely also removes
> the M5.16 aliasing symptom, since pages stop being shared), then re-enable the forge default-on.

> ### UPDATE 2026-06-18 (cont. 2) — mechanism verified in code+log; isolation test narrows it to CROSS-CLIENT
>
> **Step 1 (mechanism, verified):** `bar1_wpg` (our MRU cache of guest-BAR1-written FB pages,
> which M5.16 uses to resolve the GSP-managed scrub ring) is **never invalidated** — grep shows
> only write + read sites, no clear path. And the guest **does** issue the completing action we
> ignore: `INVALIDATE_TLB`-class + `0x20800a6c`/`0x20800a61` FB-flush/membar control RPCs
> (`status=0x0`, we just ack), including right after the channel free. So we trust a stale
> VA/BAR1→FB cache instead of a refcounted GPGA reference, and drop the guest's membar.
>
> **Step 2 (isolation/coherence test `tests/mode2/chshare.c`):** two channels (streams) in the
> SAME client/VAS → **PASS** (each fills its own 64 KiB buffer with a distinct pattern, no
> contamination; + a shared buffer written by ch1 and read by ch2 is coherent). So the backing
> model is correct *within one client*. The #12 collision is therefore **cross-client / cross-VAS**:
> the CeUtils scrub channel (`client 0xc1e00007`) vs a UVM channel (`client 0xc1d00001`) — and
> `bar1_wpg` + the FB-phys overlays are global, not client-isolated.
>
> **Stale-after-free confirmed by timeline:** UVM channel `0x1210d0000` is **freed at log line
> 362099**, but `nvkvm_m2_ctx_free_drop` only drops bookkeeping (chans/chanbuf/devvas/cvas/
> chanvas) — it does **NOT** release the channel's `m2_fbback`/`m2_gpga`/host-object backing or
> invalidate `bar1_wpg`. So the dead UVM channel's overlay at FB `0x31f0000` persists, and the
> scrub channel's finishPayload (collided onto that phys) reads the dead channel's RAM at the
> teardown poll (after 362099) → the 4 s timeout.
>
> **Fix (the address-table lifecycle, per the agreed model):** on channel/object free, release
> that channel's FB-phys backing (refcounted — freed only when the last VAS/BAR mapping drops,
> never while still referenced) and invalidate the matching `bar1_wpg` entries; honor the guest's
> TLB-invalidate/flush as the membar that gates phys reuse; and key resolution per-client so two
> live clients can't alias one emulated-FB page. Then the (already-exact) forge delivers to the
> right, un-collided backing.

## Symptom

A 2nd CUDA context after the 1st tears down hangs. Markers: `CTX1` runs and exits
cleanly; `CTX2` stops at its **first** `cuda.synchronize()` (after `randn`),
never reaching matmul. dmesg at the hang shows, from **CTX1's teardown**:

```
NVRM: scrubberDestruct:  Timed out when waiting for the scrub to complete ...
NVRM: nvAssertFailedNoLog: pCeUtils->lastCompletedPayload == lastSubmittedPayload @ ce_utils.c:349
```

So it is CTX1's teardown (the CE memory scrubber) that wedges shared driver state,
and CTX2 then blocks on its first GPU op. "2nd context hangs" is the *observable*;
"1st context's scrubber teardown never completes" is the *cause*.

## Two distinct layers (don't conflate)

1. **Wrap-wedge (FIXED, committed `37d15c5`):** the UVM tracking semaphores
   (`0x121000xxx`) hit a double-writer → backward jump → `uvm_gpu_semaphore.c:776`
   / `uvm_channel.c:205`. Fixed by exact-GPA + backward-only parser defer.
   Validated: cup8 byte-exact + PyTorch-heavy, no regression.
2. **CE completion at teardown (THIS doc, open):** a *different* sema, in a
   *different* place, surfaced only after layer 1 was fixed.

## Root cause (proven with guest counters)

An **instrumented** guest `nvidia.ko` (built from full 580.159.04 source, with
`NV_PRINTF` at the `ce_utils.c` destruct) printed the decisive values:

```
destruct #1 PASSES: lastSub=2  hwsema=2  finVa=0x42006c004  bUseBar1=0
destruct #2 HANGS:  lastSub=63 hwsema=0  finVa=0x12006c004  bUseBar1=1
```

- The hanging channel is **`gpfifo 0x120064000`** (`pbGpuVA=0x120000000`), a
  **GSP-managed** UVM CE channel. `chan_exec` shows **`picked_pdb=0x0`** — we
  cannot resolve its pushbuffer address space.
- Because the VAS is unresolvable, we **drain its ring** (`gp_get 0→63`) but
  **never parse/execute its methods**, so its CE `SET_SEMAPHORE` to the
  finishPayload sema is never performed.
- That finishPayload sema is **`0x12006c004`** (= `pbGpuVA + finishPayloadOffset
  0x6c004`), in **vidmem**, `bUseBar1=1`. It reads **0** forever
  (`m0_qemu.log` grep: zero writes to `0x12006c004`).
- `ceutilsDestruct` / `scrubberDestruct` (`mem_scrub.c: _isScrubWorkPending` →
  `ceutilsUpdateProgress` → `READ_CHANNEL_PAYLOAD_SEMA`) wait for that sema to
  reach `lastSubmitted=63`, time out (4 s), assert, wedge.

The *passing* destruct #1 channel (`0x420064000`, `bUseBar1=0`) has its
finishPayload in **sysmem**, which our parser *does* write (`CE_SEM_RELEASE
addr=0x42006c004`) — so no hang. The difference is purely resolvable-vs-not.

The whole night's earlier `0x121000xxx` analysis chased the **wrong sema** (that
is the UVM tracking sema; the CeUtils finishPayload is a separate sema in the
channel's own vidmem buffer).

## Why "can't resolve the pushbuffer VAS"

The channel is `bGspOwned` (`kernel_channel.c`); GSP manages its page tables, so
the CPU-side instance-block PDB reads empty and there is no walkable root. The
binding existed at channel-create (the guest knows the channel buffer's phys via
`memdescGetPhysAddr(AT_GPU)`) but the data plane *threw it away* and tried to
reverse-resolve at exec time — which fails for GSP-managed channels. This is the
exact failure the address-table design removes.

## REFINEMENT 2026-06-17 (re-derived from the overnight `m0_qemu.log`)

The "unresolvable VAS" framing above is **imprecise** — corrected here from the
full candidate dump in the overnight trace (`/tmp/m0_qemu.log` on `vh`):

- The CeUtils gpfifo VA `0x120064000` **DOES resolve via PDB walk.** Both captured
  roots of its VAS (`hVASpace=0xcaf00005`: `0x3114000` from RESERVED_PDES and
  `0x3400000` from SET_PAGE_DIRECTORY) agree it maps to **sysmem phys
  `0x165664000`** (`DIAG vas[2]/vas[3] … eva=0x120064000 -> SYS phys=0x165664000`).
  So the gpfifo ring is in **sysmem**, and the walk is correct.
- `picked_pdb=0` is **not** a resolution failure — it is a **content-gate
  artifact.** `nvkvm_chan_execute` only pins a PDB if the GP entry reads
  **non-zero** (`val != 0`). At the moment of the sweep the ring slot reads `0`
  (an idle/empty GP slot — `val=0x00000000`), so the value-gated pin is **skipped**
  and we fall through to the `bar1_wpg` heuristic, which pins the **wrong** (a
  vidmem) pushbuffer page. The gate conflates "ring momentarily empty" with "wrong
  VAS."
- Proof the channel is otherwise parseable: `M5: CE COPY … gpfifo=0x120064000`
  (line ~136257) — when the ring is non-empty we *do* decode its CE methods.
- Aperture split: the **gpfifo is sysmem** (`0x165664000`) but the **finishPayload
  sema is vidmem** (`bUseBar1=1`, guest counters) — different memdescs in the same
  channel. So the sema must be resolved on its **own** VA (not gpfifo+offset into
  sysmem); under the same VAS its VA resolves to the vidmem phys we must write.

**Corrected failure chain:** content-gate false-rejects the (idle) sysmem gpfifo →
`chan_pdb=0` → `bar1_wpg` heuristic pins the wrong (UVM-tracking) ring → the
CeUtils completion `SET_SEMAPHORE` is never parsed → its vidmem finishPayload sema
is never written → scrubberDestruct times out.

**Implication for the address table:** this is *exactly* the false-reject the
forward table removes — a forward binding (`gpfifo_va 0x120064000 → 0x165664000
sys`, recorded at map/FILL_PTE time) carries no content-gate, so an idle ring never
demotes resolution to a heuristic. The sema's own VA resolves the same way. Still
to confirm by one targeted run: (a) the finishPayload sema VA and what each root
resolves it to (phys+aperture), (b) whether a `FILL_PTE_MEM` (0x801802) /
`INVALIDATE_TLB` (RPC fn 200 / ctrl 0x80180c) carries that sema's binding forward,
(c) whether trusting the PDB-walk gpfifo result (drop the content-gate, or gate on
"resolves" not "non-zero") lets the `SET_SEMAPHORE` parse + complete the scrub.

## CONFIRMED on current HEAD 2026-06-17 (cupctx2 repro, dc5c24c)

Reproduced cleanly with `tests/mode2/cupctx2.c` (create→matmul→destroy ×2) on a
fresh boot, `m2cefwd=on`, guest open-580 rebuilt in-guest (vermagic 6.8.0-117):

```
[CTX1] SYNC OK → RESULT bad=0 C[0]=256 → PASS      # ctx1 matmul correct
[CTX1] cuCtxDestroy (fires CeUtils scrubberDestruct) → CTX DESTROY OK
[CTX2] cuCtxCreate...                               # HANGS (rc=124)
NVRM: scrubberDestruct: Timed out waiting for the scrub to complete the pending work.
NVRM: nvAssertFailedNoLog: pCeUtils->lastCompletedPayload == lastSubmittedPayload @ ce_utils.c:349
```

QEMU-side log at teardown nails the mechanism:
- `chan_exec … picked_pdb=0x0 gpfifoVA=0x120064000` — the content-gate false-reject
  (refined cause above) still fires; the channel falls to the `bar1_wpg` heuristic.
- The channel **is** partially parsed under the heuristic — `M5: CE COPY … gpfifo=
  0x120064000` decodes the scrub copies, and `#12-L3c SEMW va=0x121000010 …` writes
  the **UVM tracking sema** (sysmem; here also handled by the instrumented build's
  backdoor, `SEMW-DEFER backdoor owns …`). So the wrap-wedge layer is contained.
- The **finishPayload sema is never written**: the only event touching its region is
  `DIAG BAR1 WR off=0x12006c -> FB 0x31f006c <- 0xd801` (the *guest* CPU initializing
  it). **No `SEMW`/`SET_SEMAPHORE` ever targets `0x12006c…`** — its release method sits
  in a GP entry the heuristic never reaches. So `lastCompletedPayload` stays < 63,
  scrubberDestruct times out, asserts, wedges → CTX2 hangs.

Decisive corollaries for the fix: (a) the finishPayload's **FB backing is known**
(`0x31f006c`, from the guest's own BAR1 write) and the channel's VAS resolves via PDB
walk — so the only thing missing is *reaching + resolving the release method*; (b) the
fix is to stop the content-gate from demoting a PDB-resolvable channel to partial
heuristic parsing, so the **whole** ring (including the finishPayload `SET_SEMAPHORE`)
is parsed and written. This is the forward-binding / drop-the-content-gate change the
address table generalizes. Repro is now turnkey: `cupctx2_run_guest.sh`.

## The fix

**Clean (do this): the address table** (`mode2_address_table.md`). Record the
channel-buffer binding at channel-create; resolve the finishPayload sema by table
hit; no PDB, no `picked_pdb=0`. The bug dissolves with no special case. The same
pushbuffer decoder that parses `SET_SEMAPHORE` also parses the `MEM_OP`
TLB-invalidate that maintains the table — one piece of plumbing serves both.

**Interim QEMU-side alternative:** when a GSP-managed CE channel drains with
`picked_pdb=0`, locate its buffer in FB via the existing `bar1_wpg` fallback
(the GPFIFO and the sema are in the *same contiguous channel buffer* —
`finishPayload = gpfifo_base − 0x64000 + 0x6c004`), and `nvkvm_fb_write` the sema.
Risk to verify: GPU-VA contiguity ≠ physical-FB contiguity; if the FB pages are
not adjacent, scan FB for the sema's backing page instead.

## Guest-side hack attempts — inconclusive (do NOT trust)

A guest-side force-complete (report completion for `bUseBar1` channels in
`ceutilsUpdateProgress` / short-circuit `channelWaitForFinishPayload`) was tried
to demonstrate the fix end-to-end. It **never landed**: the writes hit the same
unresolvable vidmem wall, and a build/NVOC quirk meant `ce_utils.c` edits past the
first instrumentation never took effect at runtime even after `make clean`
(unresolved — next session: objdump `nv-kernel.o`, or full `rm -rf` re-extract).
The diagnosis above does **not** depend on the hack; it is proven by the printed
counters. The hack is not a fix and was abandoned.

## Reproduction / instrumentation pipeline (reusable)

Bench is on the vast host (`vh` build+runtime, `vg` guest). Build an instrumented
guest driver:

1. On `vh`: `git clone --depth 1 --branch 580.159.04
   https://github.com/NVIDIA/open-gpu-kernel-modules` (network reachable from vh).
2. Apply the `0xFFF508` backdoor to `kernel-open/nvidia-uvm/uvm_channel.c` (needed
   to reach the CE layer) + add `NV_PRINTF` probes to `src/.../ce_utils.c`.
3. **Build IN-GUEST** (24.04 / gcc-13 / glibc-2.38 objtool; a `vh` 22.04
   cross-build fails on objtool GLIBC + a `module_layout` CRC mismatch — the guest
   kernel is a 24.04 build). Tar the source to the guest via the writable
   `nvkvm_src` 9p; `make -j2 modules SYSSRC=/lib/modules/$(uname -r)/build` (+ a
   `/swapf` for OOM safety on 4 GiB). ~10 min; loads natively.
4. **Build gotcha:** incremental `make modules` does NOT reliably relink
   `nv-kernel.o` after an `src/nvidia` edit (the RM combines all objects) → always
   `make clean && make modules`, and verify with a *runtime* marker (NV_PRINTF
   literals are invisible to `strings`).
5. Deploy `nvidia.ko` to guest `~/nvmods` (back up `~/nvmods.good` first), fresh
   boot, run `pt_ctx2`, read `dmesg | grep NVKVM-`.

## UPDATE cont. 5 — forge bench-disproven; the "FB backing known" corollary was wrong

A run with the `#12 FORGE` active (markers present, `cupctx2`, 117-pinned guest)
**still hangs**, and the persisted log (`vh:/tmp/m0_qemu.log`, 30 MB) lets us correct
two earlier claims and reframe the fix.

**Verified, unchanged:** our RAMIN PDB offsets are correct — `NV_RAMIN_PAGE_DIR_BASE_LO
= word 128 = byte 0x200`, `_HI = word 129 = 0x204` (ga102 inherits gm107
`dev_ram.h`), matching `NVKVM_RAMIN_PDB_{LO,HI}_OFF`. The instblk path is not
mis-offset; the instblk at `0x2efa6e000` is simply **never populated** — every channel
logs `M5.14 … PDB empty (GSP-managed)` because our fake GSP (which owns instblk
construction in GSP-client mode) never writes RAMIN+0x200. So "read the instblk PDB"
is a dead end for *all* channels, not just this one.

**The channel names no VAS.** `M5.3 DIAG c56f … hVASpace@28=0x00000000` — confirmed
from both source (`hVASpaceId = NV01_NULL_OBJECT`) and log. The only VAS the client
(`0xc1e00007`) ever created (obj `0x0c`, PDB `0x2efba5000`) is **RM-handle-freed at
M5.49 *before* the channel even allocates** (`hVASpace`-less). So `picked_pdb=0` on
every `chan_exec`; there is no handle to bind to.

**The decisive asymmetry (this is the bug, precisely):** the channel has two
semaphores in *different apertures*, and only one resolves —
- host/progress sema, GPU-VA `0x121000000` → **SYSMEM** `0x14444d000`. Resolves
  (`res=translate`, GPA-backed, already in the address table) → the ring drains, the
  guest's progress sema advances.
- finishPayload, GPU-VA `0x12006c004` → **VIDMEM**. Needs the channel's VAS PTEs to
  translate; we have none → VAS-walk through the only candidate PDB (`0x3114000` =
  the UVM client `0xc1d00001`'s VAS, line 5110) **FAULTs**; the resolver falls to the
  `bar1_wpg` MRU heuristic.

**Correction to cont. 4 corollary (a).** The earlier note "finishPayload FB backing is
known = `0x31f006c`, from the guest's own BAR1 write" is **wrong**. BAR1 off `0x12006c`
→ FB `0x31f006c` is the **gpfifo ring entry at byte `0x6c`** (first page) — a
coincidental digit-match with the finishPayload *GPU-VA* `0x12006c004`. They are not
the same thing. finishPayload lives at **channel-buffer offset `0x8004`** (`gpfifo_va +
0x8004`), i.e. BAR1 off `~0x128004`. The captured BAR1 PTEs for this buffer cover only
off `0x120000–0x1200xx` → FB `0x31f0000–0x31f00xx` (the first page). **No captured PTE
— BAR1 or VAS — ever maps the finishPayload page.**

**Why the forge fails.** It computes `finFB = gpfifo_FB(0x31f0000) + 0x8004 =
0x31f8004` — a *contiguity extrapolation* from the first page. The doc's own interim
section flagged the risk ("GPU-VA contiguity ≠ physical-FB contiguity"); this run
**confirms the risk materialized**: the forge wrote `0x31f8004` for the entire run
(counter climbed `0→39+`) and `CeUtils` still read `0` → timeout → assert → wedge.
The buffer is provably **non-contiguous** — the host sema is sysmem and the
finishPayload is vidmem, so they are *separate memdescs*; `gpfifo_FB + 0x8004` is the
wrong page.

**Reframed fix (still the address table, now sharper).** The write authority for the
finishPayload is the CE `SET_SEMAPHORE` method targeting GPU-VA `0x12006c004` in the
channel's VAS — which we cannot resolve. But the guest *also* maps the same memdesc
through **BAR1** for its own CPU read (`memmgrMemDescBeginTransfer(USE_BAR1)` in
`channelReadChannelMemdesc`). Both tables point at the *same* finishPayload FB page.
The BAR1 table is the one we **can** populate authoritatively (the guest writes BAR1
PTEs; we snoop them) — so the principled fix is: **walk the BAR1 page table for the
finishPayload page's BAR1-VA to get its true FB, then write there** — *not* extrapolate
`+0x8004` from the gpfifo. Open sub-question gating this: the guest's `USE_BAR1`
transfer mapping for the finishPayload is **transient** (created per-read during the
poll, torn down after), so at method-execute time there may be no live BAR1 PTE for it.
If so, the table must record the finishPayload memdesc's FB at **channel-buffer
construction** (the `c56f`/memdesc RPC carries the sema memdesc) and resolve by table
hit at drain — exactly the "record the channel-buffer binding at channel-create" clean
fix above, with the emphasis that the *sema* memdesc (not just the gpfifo) must be the
recorded unit, since the two are not contiguous.

Net: the clean address-table fix is unchanged in direction but the *unit of binding*
is corrected — bind each **channel-buffer memdesc separately** (gpfifo, sysmem host
sema, **vidmem finishPayload**), never assume one contiguous span. The `bar1_wpg`
+`+0x8004` interim is disproven and should be retired for finishPayload.

## UPDATE cont. 6 — both cheap hooks closed; (C) sysmem-aperture is guest-hardcoded

Re-converged after re-reading the in-tree `#12 NOTE` (line ~1677) + the open RM
scrubber init. Two QEMU-side "cheap" hooks are **both structurally closed**, and the
clean sysmem angle is **not ours to flip**:

- **Write emulated-FB (the forge): dead.** The guest reads the finishPayload through a
  **non-trapping KVM memslot** whose backing is not coherent with `nvkvm_fb_write`
  (the promoted/GPA-window page ≠ the emulated-FB `g_malloc`), so even a correctly
  *located* write would not be seen. (And cont. 5 showed the forge's *location* is also
  wrong — `gpfifo_FB + 0x8004` assumes a contiguity the sysmem-hostsema/vidmem-fin
  split disproves.)
- **Intercept the poll read: dead.** Same non-trapping memslot — the finishPayload poll
  does **not** trap (BAR1 reads aren't traced; the 100k trapped reads are the BAR0
  PRAMIN/CRASHWIN window, which the guest stops using before the wait). Nothing to hook
  on the read side.
- **(C) make finishPayload land in sysmem: not GSP-controllable.** The aperture split is
  hardcoded in the guest driver: general CeUtils passes `_NO_BAR1_USE_TRUE`
  (`mem_mgr.c:4134`) → `bUseBar1=FALSE` → **sysmem** (this is the sibling that resolves
  and works); the memory scrubber passes `_VIRTUAL_MODE_TRUE` with no `_NO_BAR1_USE`
  (`mem_scrub.c:154`) → `bUseBar1=TRUE` → **vidmem**. `bUseBar1` is purely
  `FLD_TEST_DRF(_NO_BAR1_USE, allocFlags)` — no GPU-cap / GSP input — so our fake GSP
  cannot nudge the scrubber onto the sysmem path without modifying the (unmodified)
  guest driver.

**Net.** There is no patch-sized fix. The crux is singular and unavoidable: **resolve
the GSP-managed, `_VIRTUAL_MODE` scrubber channel's VAS** so its own buffer VA
`0x12006c004` translates to FB. The scrubber issues its CE *copies* through this VAS and
we already forward those (`CE COPY … out=…(phys)`), so the data plane works; only the
channel's *self-referential* finishPayload sema is unresolvable, because the VAS it runs
in has `hVASpace=0`, an empty (GSP-owned) instblk, and a transient VAS handle
(`0x2efba5000`) freed before the channel ran. Once that PDB is known, the fix is either
(A) coherent-write the translated FB through the promoted memslot backing, or (B)
forward the `SET_SEMAPHORE` to the host CE alongside the copies. Both are real work;
neither is a heuristic. This is the address-table directive's "PDB = communication, the
channel must carry its binding" applied to a channel that deliberately discards its
binding — the open question is what authoritative signal *does* carry the scrubber
channel's PDB (candidate: the instblk *would*, if our fake GSP synthesized RAMIN+0x200
from the channel-alloc memdescs at construction — making us populate what real GSP
populates).

## UPDATE cont. 7 — ROOT CAUSE: we key VAS by client; HW keys by PDB (instance block)

The cheap-hook dead-ends in cont. 5/6 were a symptom. The real root cause, found by
walking the *map* failures instead of the sema:

**The `map-FAILED` is deterministic, not racy, and not a host-ioctl error.** Every
`M5.19 fwd-map pushbuffer … client=0xc1e00007 -> map-FAILED` (42×) bails at the **first
line** of `nvkvm_m2_back_and_map_sys`: `hDev = m2_devvas[client].dev; if (!hDev) return
false;` — there is **no device/VAS registered for client `0xc1e00007`** (confirmed:
`M5.7 grmapper: no dev/vas for client 0xc1e00007`; every `M5.49` drop shows
`devvas=0 cvas=0 chanvas=0`). No `M6.5 back_sys …` line is ever emitted for it, proving
it returns *before* any host RM call. The **identical** VA→GPA
(`0x120800000→0x155e00000`) mapped fine under client `0xc1d00001` (`-> MAPPED`), so the
mapping is valid and shareable — just unreachable under the scrub client's key.

**Why the scrub client owns no VAS — the keying divergence (open-RM ground truth).**
`kernel_channel.c:1030`: `pKernelChannel->hVASpace =
pKernelChannel->pKernelCtxShareApi->hVASpace;` — a channel's effective VAS comes from
its **KernelCtxShare** (subcontext, under the TSG), **not** from the channel's own
`hVASpace` param (which is `0` here). With `hVASpace=0` and no explicit ctxshare, the
implicit TSG binds the **device's default VAS**. So the VAS is owned by the
**ctxshare/TSG and identified by its PDB (instance block) — client-independent and
shareable.** Hardware roots every channel's translation at `RAMIN+0x200` (the PDB); the
client handle is irrelevant to translation.

We do the opposite: `m2_devvas[**client**]`. A channel whose VAS is inherited/shared
(not owned by its own client handle) is **invisible** to a per-client lookup → `hDev=0`
→ everything for that channel "doesn't exist." **The 42 `map-FAILED`s and the
unresolvable vidmem finishPayload are the same bug.**

**PDB chase result (cont. 7): the scrub channel's true root is not observable in our
state.** No captured PDB correctly roots its own buffer (`gpfifo 0x120064000 → real FB
0x31f0000`, or the finishPayload): `0x2efba5000` (its freed explicit VAS) FAULTs;
`0x2efa6c000` (sibling) maps to a sparse/wrong page; `0x3114000` (UVM) resolves only the
**sysmem** host-sema; `picked_pdb=0` otherwise. There is **no `PDB=`, `SET_PAGE_DIRECTORY`,
or page-table write** that establishes the device-default VAS for this channel. It is
created GSP-side and our fake GSP never models it, so its root was never materialized in
our world (even though the guest's PMA allocated the page directory — we never associated
that allocation with this VAS).

**Fix (three parts, HW-faithful):**
1. **Key the VAS table by PDB (instance-block root), not by client** — a global
   `pdb → {host device, host VAS, page-table view, isolate}` map.
2. **Resolve a channel's VAS via the ctxshare/TSG → PDB chain** (mirroring
   `kernel_channel.c:1030`), with `hVASpace=0` → the **device-default VAS** — never via
   the client handle. Channels/clients sharing a ctxshare share one entry → one isolate.
3. **Model (or capture) the device-default VAS's PDB**, since cont. 7 shows it is not
   observed for this channel: our fake GSP must create/assign it (as real GSP does) or
   capture the guest PMA's page-directory allocation and bind it to the device-default
   VAS. Then `back_and_map_sys` finds the shared VAS and **reuses** the existing host
   placement (generalize the `mst==0x51` ALREADY-MAPPED path to "shared VAS, reuse"),
   and the finishPayload VA `0x12006c004` walks through the now-known PDB.

This is the address-table-of-truth (`mode2_address_table.md`) stated precisely: one
table per VAS keyed by PDB, channels finding their VAS through the instance-block/
ctxshare chain like silicon — never through the client handle.

## UPDATE cont. 8 — the fix: a QEMU-owned "system VAS" + coherent write (supersedes cont.5)

Co-designed with the user; this supersedes two earlier errors and names the build.

**Correction to cont. 5 (the finishPayload IS contiguous).** cont. 5 claimed the
finishPayload was a *separate, non-contiguous* memdesc. Wrong — it conflated two
different semaphores: the channel's **host/progress sema** (genuinely SYSMEM,
`0x14444d000`, VA `0x121000000`) with the **finishPayload** (VIDMEM, VA `0x12006c004` =
`gpfifo_va + 0x8004`). The instrumented guest's backdoor reported the finishPayload
region at **FB `0x31f8000`** (`= gpfifo_FB 0x31f0000 + 0x8000`), i.e. **contiguous**
with the gpfifo inside the same 64 KB channel-buffer object. The forge's location
(`0x31f8004`) was therefore **correct**; it failed only on coherence (below). The FB was
knowable all along.

**Exhaustive PDB check (settles cont. 7's open question).** No captured GMMU VAS maps
the scrub buffer to its real backing: the sibling's `0x2efa6c000` has VA `0x120064000`
only as a **sparse/identity reservation** (`fb=0x64000 val=0`), `0x3110000`/`0x2efba5000`
FAULT, UVM's `0x3114000` resolves only the sysmem host-sema. The buffer's *real* FB
(`0x31f0000`) is reachable **only via the BAR1 VAS** (`bar1_pdb`, statically
pre-allocated) — because the scrubber is `bUseBar1=TRUE` and the guest reaches it through
BAR1. So: don't chase the absent GMMU channel PDB; the address is already observable.

**Root cause, final form.** `hVASpace == NV01_NULL_OBJECT (0)` → the channel uses the
**device-default VAS** (via ctxshare, `kernel_channel.c:1030`). For kernel-internal /
GSP-managed work that default is a **shared kernel/system address space**, and it is
**GSP-managed — i.e. ours to define.** Today we key per *client* (`m2_devvas[client]`),
so the scrub client (which owns no VAS of its own) is invisible → `no dev/vas` →
map-FAILED + unresolvable finishPayload, all one bug. FB `0x31f0000` (incl. the
finishPayload at `0x31f8004`) is actually backed by UVM client `0xc1d00001`'s `gpga_obj`
obj 8 (`cpu_qva=0x75d18c023000`), **aliased** by the scrub channel — and a (disproven)
`#12 LIFECYCLE release … de-alias` even tore that backing out mid-run.

### The build: a QEMU-owned system VAS, forward-populated, keyed by a minted PDB

1. **`m2_system_vas` (per kernel device).** A QEMU-owned default VAS with a **PDB we
   mint** (we are GSP). One per kernel device (NOT one GPU-wide — avoid cross-device VA
   aliasing). Fields: `{ device, client, pdb_synth, va→fb interval map }`.
2. **Resolve `hVASpace=0` → `m2_system_vas[device]`.** In the channel VAS-resolution
   chain (`nvkvm_chan_own_pdb_rs`), when the channel names no VAS and `cli_vas` doesn't
   resolve, fall to the system VAS for the channel's device. Key the table + host
   isolate/device on the **minted PDB**, not the client — so sibling kernel channels
   share one entry (matches HW: instance-block PDB, client-independent).
3. **Forward-populate from observation.** We already derive `VA 0x120064000 → FB
   0x31f0000` for the scrub channel (`M5.24 GPFIFO double-mmap`, from the BAR1-written
   page). Record that interval into the system VAS regardless of host-placement success.
   Then `VA 0x12006c004 = base + 0x8004 → FB 0x31f8004` falls out by contiguity within
   the 64 KB buffer object. (Address-table directive verbatim: forward-populate, never
   reverse-resolve.)
4. **Give the system-VAS client a device** so `back_and_map_sys` stops bailing at
   `hDev=0`: resolve the scrub client to the system-VAS device, and treat an existing
   placement at the same VA→GPA as **reuse** (generalize the `mst==0x51` ALREADY-MAPPED
   path) instead of a FIXED-map collision → kills the 42× map-FAILED.
5. **Coherent write (separate, smaller fix).** Resolution finds `0x31f8004`; completion
   must write the **host page the guest actually reads**. The guest reads the
   finishPayload via BAR1; per the in-tree note that path is a **non-trapping memslot**
   whose backing ≠ the emulated-FB `g_malloc` page the forge wrote. Fix: route the
   completion write through the **same overlay/memslot backing** (obj-8's `cpu_qva`, via
   `nvkvm_fb_host_overlay`), and **do not de-alias** a shared kernel buffer while a
   channel still references it. Write value = the channel's `lastSubmittedPayload`
   (already tracked as `c->fin_payload`); advance-only, never backward.

### Open questions to settle at implementation (bench-verifiable)
- **Coherence path (load-bearing):** does the guest's BAR1 read of `0x31f8004` route
  through `nvkvm_fb_host_overlay` (then fb_write is already coherent and step 5 is
  trivial), or through a separate non-trapping KVM memslot (then we must write *that*
  backing)? One instrumented read-trap confirms which.
- **PDB minting:** any value is fine for QEMU-internal resolution, but if we later
  **forward** the scrub CE to the host (option B), the host VAS needs real page tables —
  out of scope for the in-QEMU completion, which is all #12 needs.
- **De-alias safety:** ensure the disproven `#12 LIFECYCLE release` de-alias path is gone
  / gated so it can't yank a shared kernel buffer's backing.

This is the address-table-of-truth (`mode2_address_table.md` §13) made concrete for the
`hVASpace=0` kernel case: one QEMU-owned VAS per kernel device, minted PDB, populated by
observation, keyed by PDB — exactly the Rust core's `HashMap<PdbRoot, IntervalMap<…>>`.

---

## UPDATE cont. 9 (2026-06-20) — IMPLEMENTED + BENCHED: #12 is LAYERED. L1+L2 fixed, L3 found.

Built the fix and ran it (`cupctx2`, `NVKVM_M2TRACE=1`, `m12_forge_orch.sh`). The result:
the documented finishPayload root cause is **fixed**, and the hang **moved twice**, exposing
#12 as a *multi-layer GSP re-acquire* problem — each layer a stage of a teardown+reboot the
one-shot fake-boot never exercised.

**Correction to cont. 3/8's premise.** BAR1 is **pure MMIO** (`memory_region_init_io`,
`nvkvm_aperture_ops`) — there is NO memslot over it, so every guest BAR1 read traps
`nvkvm_baraperture_read` and walks the same `bar1_pdb` as `nvkvm_fb_write`. Forge-write and
guest-read are therefore **coherent by construction**; the "memslot-served / no-trap" claim
was a false read of the 2000×-consecutive-spin heuristic (it resets on any interleaved read,
so a real poll loop never trips it). The coherent-backing step is **not needed**.

**Layer 1 — finishPayload split (the real root cause). FIXED, commit `c6b4150`.** The forge
resolved its target FB from the *global* `chan_gpfifo_phys`/`chan_gpfifo_bar1off` (the M5.16
content-heuristic MRU scan over `bar1_wpg`), stomped per doorbell by whichever channel last
decoded plausibly. Persisted-log proof: the *same* scrub channel (`0xc1e00007`,
`gpfifo 0x120064000`) alternated `finFB 0x31f8004 ↔ 0x3138004`, **splitting its monotonic
payload across two FB pages** so the guest's real sema (VA `0x12006c004` → FB `0x31f8004`)
only ever saw a subset and never reached `lastSubmitted` → 4 s `scrubberDestruct` timeout +
assert + wedge. Fix = the address-table principle scoped to one channel: forward-populate the
finishPayload FB **once**, on the first doorbell the channel advances (its just-written ring
page is freshest in `bar1_wpg`, so the MRU scan picks the *right* page), cache it in
`chans[].fin_fb`, **pin it** — never heuristically re-resolve. Verified: 58 forge writes all
to the single page `0x31f8004`; one `FORGE-RESOLVE … (pinned)`; **no scrub timeout/assert;
`[CTX1] CTX DESTROY OK`** (clean teardown — previously asserted+wedged here).

**Layer 2 — GSP re-boot keeps WPR2 down. FIXED, commit `bf36f63`.** With L1 fixed the hang
moves to `[CTX2] cuCtxCreate`. `cuCtxDestroy` of the *last* context sends **fn-47 UNLOADING**
(a GPU-idle release while the module stays loaded — not just rmmod) → `gsp_suspended` + WPR2
down. The next `cuCtxCreate` re-acquires: it **reloads the GSP falcon image** (`DMATRFCMD`)
then `STARTCPU`s to re-boot. But the teardown-phase gate (`teardown = gsp_suspended`) treated
that single re-boot `STARTCPU` as a *bare trailing-teardown* STARTCPU and **kept WPR2 down**,
so the guest waited forever for a `GSP_INIT_DONE` that never came. The gate assumed *two*
post-UNLOADING STARTCPUs (defensive-unload then boot); a re-acquire has only *one*, and it
must boot. Fix: latch `gsp_reloaded` when the guest issues a `DMATRFCMD` transfer **while
suspended** (the unambiguous genuine-reboot signal); `teardown = suspended && !reloaded`.
Verified: a 2nd `M3: GSP STARTCPU → FWSEC ran, WPR2 up` now fires and the re-boot progresses.

**Layer 3 — SEC2 Booter Load on re-acquire (OPEN, next).** After WPR2 re-raises, the guest
re-establishes WPR2 through the **SEC2 Booter** (the "separate SEC2 path we don't model" the
STARTCPU comment already flagged): it DMA-loads the Booter ucode into SEC2 (`0x84011c`
stepping `0x4000,0x4100,…` = IMEM/DMEM; `0x840118` `DMATRFCMD` transfers), runs it (SEC2
`STARTCPU`), then **polls `0x8403c0` / `0x8400f4` for a Booter-done signal our emulation never
produces** → hang. Decisive asymmetry: the **first** boot touched SEC2 (`0x840xxx`) **zero**
times (it used the GSP-falcon FWSEC fake directly); only the **re-acquire** uses SEC2. So the
remaining #12 work is a scoped sub-project: fake the SEC2 Booter Load completion (read the
driver's SEC2 Booter completion check; set the polled status/mailbox to its success values on
SEC2 `STARTCPU`; keep WPR2 up), analogous to the original M3 GSP-boot faking. Rejected
alternative: prevent the fn-47 idle-release to keep the GSP alive across contexts (hacky,
fights the guest driver, doesn't generalise to real teardown).

---

## UPDATE cont. 10 (2026-06-20, evening) — L3 RE-DIAGNOSED from guest dmesg; assert #2 fixed

**cont.9's L3 ("guest hangs polling SEC2 `0x8403c0`/`0x8400f4` for a Booter-done signal") was
WRONG.** The SEC2 register polls all complete cleanly (DMATRFCMD→IDLE, CPUCTL→HALTED,
MAILBOX0→0). The real wall was found by reading the **guest kernel log** (`dmesg`) — the ground
truth I should have pulled first. It shows two asserts, neither in the GSP-boot/INIT_DONE path:

```
scrubberDestruct: Timed out when waiting for the scrub to complete the pending work
nvAssertFailedNoLog: pCeUtils->lastCompletedPayload == lastSubmittedPayload @ ce_utils.c:349
kgspExecuteBooterUnloadIfNeeded_TU102: failed to execute Booter Unload: WPR2 is still up
nvAssertFailedNoLog: rmStatus == NV_OK @ osinit.c:2363
```

So L3 is a multi-part **teardown/re-acquire** problem, not a GSP-boot-faking problem:

**L3a — SEC2 Booter UNLOAD must lower WPR2. FIXED, commit `ea219c4`.** On a context
teardown/re-init the driver runs `kgspExecuteBooterUnloadIfNeeded_TU102` (SEC2 Booter Unload)
and then asserts WPR2 reads **down** (else "WPR2 is still up" → `osinit.c:2363`). Our model only
ever *raised* WPR2 (GSP-falcon FWSEC STARTCPU) and never lowered it on the SEC2 path. From BAR0
alone Load vs Unload differ **only by the mailbox args** before the SEC2 STARTCPU: a NORMAL
Booter Unload writes `MAILBOX0/1 = 0xff` (`kgspExecuteBooterUnloadIfNeeded_TU102`; Load writes 0
or the WprMeta GPA; GC6 writes `0xdeaddead`). Fix: latch `NV_PSEC_FALCON_MAILBOX0` (0x840040);
on SEC2 CPUCTL STARTCPU, if it is `0xff` drop `fwsec_ran` → WPR2 reads down. Verified: trace
shows `SEC2 Booter Unload (mbox0=0xff) → WPR2 down`, WPR2_HI then reads 0, assert #2 gone.

**L3c — GSP_INIT_DONE re-post + seqNum preservation. LANDED `ea219c4` (correct, not yet
exercised — guest faults at L3b first).** On a re-acquire the guest reuses the existing boot-args
+ message queue and does **not** re-write the boot-args mailbox, so the mailbox-keyed
`nvkvm_m3_dump_bootargs` never re-runs → INIT_DONE never re-posted. Re-post it on the genuine
re-boot (`was_suspended`) from the cached boot-args GPA. **Critical seqNum fact** (from the open
driver): `MESSAGE_QUEUE_INFO` (and `rx/txSeqNum`) is built in `kgspConstructEngine` and freed
only in `kgspDestruct` (module *unload*), NOT on the idle-release, so it **persists** across the
re-boot. Per boot `GspStatusQueueInit→msgqRxLink` resets only the **position** (`rxReadPtr=0`),
never the seqNum (there is **no** `rxSeqNum=` reset anywhere in the gsp tree — only `++`). So the
re-post must carry the **continuing** seqNum (== guest `rxSeqNum`), not 0 — else
`message_queue_cpu.c:762,768` treats it as an old package and ignores it. Implemented by
preserving `stat_seqnum`/`cmd_readptr` across re-acquire and resetting only `stat_writeptr`
(first boot keeps them 0 from realize, so this is first-boot-neutral).

**L3b — PMA/heap CeUtils scrubber `scrubberDestruct` timeout. OPEN (current wall).** The memory
scrubber torn down in `RmShutdownAdapter` (`mem_scrub.c scrubberDestruct` → `_isScrubWorkPending`
→ `ceutilsUpdateProgress` → `READ_CHANNEL_PAYLOAD_SEMA`) times out waiting for
`lastCompletedPayload == lastSubmittedPayload` (`ce_utils.c:349`). This is a **second**
finishPayload target, distinct from L1's cuCtxDestroy CeUtils (different channel/client). Same
class as L1, but the right fix is the documented one — **CE-forward / coherent backing** so the
real `SET_SEMAPHORE` writes the page the guest reads (task #2), rather than extending the forge
heuristic to yet another channel. Next focused session.

Repro/forensics: `m12_forge_orch.sh` on vh (now also copies `mode2_regs_ga10x.h`), then
`ssh -p 2223 ubuntu@localhost sudo dmesg` for the driver-side asserts (the qemu log alone hides
them — the qemu-side "scrub timeout" grep checks the wrong place).

---

## UPDATE cont. 11 (2026-06-21) — L3b ROOT-CAUSED + FIXED (per-channel finishPayload)

cont.10 said L3b's "right fix is CE-forward (task #2), not more forge heuristic." That was the
wrong framing: forensics show the scrubber's finishPayload completion was failing for **two
mechanical reasons**, and the principled fix **removes** a heuristic (the global-stomp) rather
than adding one — squarely the [address-table-of-truth] directive.

**Evidence (the L3a-fix run, /tmp/m0_qemu.log + guest dmesg):**
- Guest dmesg ground truth: `scrubberDestruct: Timed out` (132s) → `nvAssertFailedNoLog:
  pCeUtils->lastCompletedPayload == lastSubmittedPayload @ ce_utils.c:349` (136s).
- CE-INSTR census: the timing-out scrubber traffic is **client `0xc1d00001`** (the guest-RM
  kernel CeUtils — ch[3]×125, ch[7]×213, plus ch[4..10]); the forge only EVER fired for
  `0xc1e00007` (the cuCtxDestroy CeUtils): `FORGE-RESOLVE` once, `FORGE` ×58, all done by log
  line 361774 — well before teardown.
- The scrubber's parsed `CE_SEM_RELEASE`s target a **shared tracking sema at `0x121000010`/
  `0x121000000`**, NOT the per-channel finishPayload at `gpfifo_va+0x8004` (e.g. ch[7]
  gpfifo=`0x1210d0000` → finishPayload `0x1210d8004`). Distinct semaphores.
- Open-driver confirm: the kernel CeUtils channel buffer (and thus its finishPayload) is
  **SYSMEM by default** (`NV01_MEMORY_SYSTEM`/NCOH; `mem_utils_gm107.c`), unlike `0xc1e00007`'s
  VIDMEM ring. finishPayload is the channel-HOST semaphore released per GP entry
  (`channelWaitForFinishPayload` polls exactly it) — NOT a pushbuffer CE method.

**Two mechanical bugs (both fixed):**
1. **WRONG GATE.** The old forge fired only when M5.16 had pinned a VIDMEM ring
   (`s->chan_gpfifo_phys != 0`). The scrubber resolves through its OWN VAS (`chan_pdb != 0`), so
   M5.16 never runs for it → `chan_gpfifo_phys` stays 0 → gate failed → never forged.
2. **WRONG SKIP.** `if (chan_sem_released) continue;` ran FIRST and skipped the finishPayload
   completion for any scrubber whose tracking-sema release (`0x121000010`) we parsed — but that
   release does not advance the finishPayload.

**Fix (implemented this session, gpu_emul doorbell loop):** generalise the L1 forge —
- Resolve the finishPayload **per-channel through the channel's own VAS** via
  `nvkvm_chan_own_pdb_rs()` (the same authoritative root `chan_translate` uses), walking
  `gpfifo_va + 0x8004`; **aperture-aware** write via `nvkvm_phys_rd32/wr32` (SYSMEM *or* FB).
  The M5.16 bar1off shortcut stays as a fallback for the VIDMEM-ring channel.
- Run the completion **before** the `chan_sem_released` continue for kernel CeUtils channels
  (client != GR, !user-CE). Forward-only (`cur < fin_payload`), pinned once per channel
  (`c->fin_fb` + new `c->fin_sys`).
- Gated on `m2trace` for this validation iteration (the #12 repro runs `NVKVM_M2TRACE=1`);
  graduate the own-VAS path to default-on after a cup8/LLM no-regression run.

This unifies L1 + L3b into one correct per-channel rule (no global stomp, no second heuristic),
and — with L3a (WPR2-down) + L3c (INIT_DONE re-post) already landed — should let the 2nd
cuCtxCreate re-acquire complete. Validation: `m12_forge_orch.sh` cupctx2 (in progress).

---

## UPDATE cont. 12 (2026-06-21) — BENCHED: forge generalised + L2/L3c CONFIRMED; the wall MOVED

Ran cupctx2 with the cont.11 fix (build verified, fresh boot). Results re-frame #12.

**What now works (proven in the CTX2 re-acquire trace + guest dmesg):**
- **Forge generalisation lands.** 434 `#12 FORGE` writes now cover the kernel CeUtils scrubber
  (client `0xc1d00001`, ch[3..10]) — previously ZERO. Resolution per-channel, pinned. (Apertures:
  `0xc1e00007`/`0xc1e00008` resolve via own-VAS to SYS; `0xc1d00001`/`0xc1d0000a` via the BAR1
  shortcut to FB. **cont.12 follow-up:** reordered to BAR1-PRIMARY / VAS-FALLBACK so we never
  regress the PROVEN L1 `0xc1e00007` FB page `0x31f8004` — its own-VAS walk resolves to a
  DIFFERENT sysmem alias `0x14f26c004`, NOT where the guest reads it.)
- **L2 confirmed.** CTX2 re-boot raises WPR2: `M3: GSP STARTCPU -> FWSEC ran, WPR2 up`.
- **L3c confirmed.** `M3: posted GSP_INIT_DONE (seqNum 916)` — the PRESERVED/continuing seqNum
  (first boot was seqNum 0; CTX1 ran the queue to stat_seq=915), exactly as the seqNum-persist
  analysis predicted. So the re-post carries the right seqNum.

**The scrub timeout is NON-FATAL — it was never the rc=124 hang.** `scrubberDestruct` breaks out of
its wait on timeout and `nvAssertFailedNoLog` only logs (`ce_utils.c:349`); `[CTX1] CTX DESTROY OK`
prints regardless. So L3b is a 4 s detour + cosmetic assert, NOT the deadlock. (The forge still
does not fully silence it — the scrubber's BAR1-resolved page is alias-prone and the per-channel
gp_get count may undershoot the guest's lastSubmittedPayload — but that is cosmetic, deprioritise.)

**THE REAL WALL (rc=124): the CTX2 GSP re-acquire INIT_DONE handshake.** After we post INIT_DONE
(seqNum 916), the guest issues **zero** GSP RPCs. The entire post-INIT_DONE window (682 BAR0 ops)
is **100% SEC2 falcon**: ~316× `0x840118` DMATRFCMD (Booter ucode DMA-load), status polls
(`0x8403c0`/`0x8400f4`), then `MAILBOX0/1=0xff` + STARTCPU = **SEC2 Booter Unload** → WPR2 down →
guest stalls (only AHCI IRQ noise after). I.e. the guest does NOT accept the re-boot: instead of
proceeding past INIT_DONE it runs the Booter **Unload** (teardown) and gives up.

Open question for next session (this is L2/L3c boundary, NOT L3b/scrub):
1. Is our GSP-falcon-FWSEC re-boot model (L2: raise WPR2 on a suspended-state `0x110118` DMATRFCMD)
   even the right mechanism for re-acquire? The FIRST boot used GSP-falcon FWSEC and touched SEC2
   ZERO times; the RE-acquire is ALL SEC2 Booter. Perhaps on re-acquire WPR2 must be (re)established
   by the **SEC2 Booter LOAD**, and our premature GSP-falcon "WPR2 up" + INIT_DONE post is the lie
   the guest rejects → it runs Booter Unload to undo and bails.
2. Does the guest actually CONSUME our INIT_DONE re-post? Verify rxReadPtr/rxSeqNum at the moment of
   the post vs what the guest reads (instrument the msgq read). seqNum 916 is posted, but position/
   queue-header state after `GspStatusQueueInit` on re-boot may not match where the guest reads.
3. Distinguish SEC2 Booter LOAD vs UNLOAD on re-acquire more robustly than `mbox0==0xff` — if the
   re-acquire LOAD path also transiently writes 0xff, L3a is mis-firing a WPR2-down mid-boot.

Status: forge-generalisation + BAR1-primary committed (gated on m2trace, no production path).
#12 still hangs; the wall is now the re-acquire boot-sequence model, not the scrubber.

---

## UPDATE cont. 16 (2026-06-21) — ★ MAJOR REFRAME: the "re-acquire" was POST-KILL teardown

Three benched runs (cont.13/14/15) tried to model the CTX2 GSP re-acquire boot sequence
(defer WPR2 / post INIT_DONE / SEC2 Booter Load-Unload re-key).  None fixed #12 — and chasing
*why* exposed that **the whole L2/L3c/cont.1x "WPR2 re-boot" line has been modeling the wrong
event.**  The fn-47 + FWSEC-SB + SEC2 Booter Unload sequence is **post-SIGKILL teardown cleanup**,
NOT the re-acquire.

**Proof (cont.15 log + guest dmesg, ground truth):**
- `kgspBootstrap_TU102` (cold/re-boot) = Scrubber → FWSEC → ResetIntoRiscv → ProgramLibosBootArgs
  → **BooterLoad** → SendInitRpcs → check RISCV active → GspStatusQueueInit → **WaitForRmInitDone**.
  `kgspTeardown_TU102` (driver UNLOAD) = GSP reset → **FWSEC-SB** ("put back PreOsApps during driver
  unload") → **Booter Unload**.  The observed re-acquire window (GSP-falcon STARTCPU + SEC2 Booter
  UNLOAD, mbox0=0xff) is `kgspTeardown`, not `kgspBootstrap`.
- **Only ONE fn-47 (UNLOADING) in the entire log.**  CTX1's cuCtxDestroy did NOT send it -> CTX1
  destroy did NOT unload the GSP; the GSP stayed loaded for CTX2.
- **Last real compute (forge/CE-INSTR) = line 365552.**  Then ~75K lines of **busy-polling GSP reg
  0x110094 (returns 0)** + occasional GSP RPCs, up to fn-47 at line 440331.
- **`scrubberDestruct` timed out at guest-time 161s** — AFTER the 120s `timeout` SIGKILL of cupctx2.
  In the driver, scrubberDestruct precedes `kgspUnloadRm`(fn-47)→`kgspTeardown`(FWSEC-SB+Unload).
  So fn-47 + FWSEC-SB + Booter Unload (440331-442021, the END of the log) happen at ~161-165s =
  **post-kill**.  The "2nd FWSEC ran / WPR2 up / Booter" activity prior sessions saw was ALWAYS this
  post-kill teardown.

**REFRAMED #12:** CTX2 `cuCtxCreate` hangs **busy-polling GSP register 0x110094** (BAR0, returns 0),
waiting for a GSP completion/response the emulation never delivers.  The GSP is NOT re-booted on
CTX2 (it stayed loaded since CTX1).  So: NOT a WPR2/Booter/INIT_DONE re-boot problem at all.

**Status / actions:**
- Reverted cont.13/14/15 (they modeled post-kill teardown as a re-boot); HEAD back at `5f74fc8`
  (the per-channel finishPayload forge generalisation — kept, it is a real improvement and the scrub
  timeout is non-fatal regardless).
- L1 (finishPayload), L3a (Booter-Unload→WPR2-down), L3c (INIT_DONE re-post) all operate on the
  post-kill teardown and are MOOT for the actual hang (harmless; can stay).

**NEXT (real wall):** find what CTX2 `cuCtxCreate` polls 0x110094 for.  Identify the GSP RPC the
guest issues during the SECOND context's create that goes unanswered (or whose completion/interrupt
we never deliver) — between the end of CTX1 (line ~365552) and the kill.  0x110094 is the GSP reg
the guest spins on for GSP responses (same reg as the per-token vmexit storm, see
[[mode2_execfwd_layer2]]).  Diff the RPC/interrupt sequence of CTX1's cuCtxCreate (works) vs CTX2's
(hangs) — the first divergence is the bug.  ★ Re-run with a SHORTER body but capture the 0x110094
poll context + the last guest RPC before the spin; do NOT analyse past the kill.

---

## cont.17 (2026-06-21) — host-log forensics CORRECT cont.16: the spin is a leaf-poll WAIT (sysmem completion), NOT a 0x110094 poll

Mined the live cupctx2 qemu log (`/tmp/m0_qemu.log`, 487431 lines, the run that produced the
161s `ce_utils.c:349` assert). cont.16 said CTX2 "busy-polls 0x110094"; the trace says otherwise.

**What the spin actually is.** The hang region (lines ~367400→429786, ~62K lines) is ONE tight loop:
```
RD off=0x000000 -> 0x176000a1     ; PMC_BOOT_0 = osIsGpuLost liveness check (valid id => alive)
RD off=0xb81008 (LEAF2) -> 0
RD off=0xb8100c (LEAF3) -> 0
RD off=0xb81010 (LEAF4) -> 0x08000000   ; vec155 bit27 SET but STALE (see below)
RD off=0xb81014 (LEAF5) -> 0
RD off=0xb81018 (LEAF6) -> 0
RD off=0xb8101c (LEAF7) -> 0
```
0x110094 is read only **866× in the whole log** and the bulk are early GSP boot (line ~2397); in the
spin region it appears ~23× total. So 0x110094 is NOT the spin. The spin is PMC_BOOT_0 + INTR LEAF.

**It is NOT an interrupt service routine.** (1) TOP (`0xb81600`) is read only **10× in the entire
log**, last at line 360312 (=0x4, subtree-2 pending) — after 360312 the guest NEVER reads TOP again.
A real NVIDIA stall-ISR reads TOP first. (2) The guest NEVER writes LEAF4 (`0xb81010`) to RW1C-clear
bit27 — **0 writes in the whole log**. So vec155/bit27 (our GSP SWGEN0, set at the last pre-teardown
delivery @360312) is left STALE; the guest is not servicing it. The leaf reads are the kernel poll
loop's *inline* liveness+interrupt sample, not a dispatch.

**The emulator is 100% IDLE during the spin.** Lines 367400→429700: **zero** non-BAR0 trace lines —
no `M4: RPC`, no status post, no `M7: SWGEN0`, no doorbell (`0xbb0090`), no `M5: chan_exec`,
stat_wp frozen at 10. So the guest is NOT waiting on a GSP RPC response (it got every one; last was
fn=10/seq=834 NV_OK @365827) and is NOT submitting new work. It is **polling a SYSMEM completion**
(a semaphore in guest RAM — those reads do NOT appear in the MMIO trace), and we never advance it.

**Last real work before the spin = CTX1 cuCtxDestroy teardown:** an fn=10 (FREE) RPC loop (seq
~825→834), each iteration DMA-writing guest-mem counters (`0x140c41010`→0xb, `0x140c41020`→0x2d);
CTX1 os-events (client `0xc1d00003`) freed 3→2→1→0 @365583-365601; last `M5: chan_exec` @365545 was
`gpfifo=0x120064000` (the CTX1 CeUtils scrub channel `0xc1e00007`, cont.11) gp_put=85.

**Guest dmesg (this exact run) shows ONLY the post-kill artifact:** `scrubberDestruct: Timed out` +
`nvAssertFailedNoLog: pCeUtils->lastCompletedPayload == lastSubmittedPayload @ ce_utils.c:349` at
**161s** (> the 120s SIGKILL). No kernel message for the actual hang ⇒ it is a userspace/kernel
busy-WAIT, not a kernel timeout — consistent with the sysmem-semaphore poll above.

**Corrected diagnosis:** CTX2 hang = the guest polls a **sysmem completion (CeUtils finishPayload
semaphore) that the emulator never releases to the expected value**. The same CeUtils is what times
out in teardown at `ce_utils.c:349`. This is a **finishPayload-completion gap** (cont.11's forge was
the right track but does NOT cover this final payload) — NOT a 0x110094 GSP-poll, NOT an ISR miss,
NOT a GSP re-boot. The forge (cont.11) covers CTX1's scrub but the monotonic target the guest awaits
is never met (alias-prone BAR1 page / gp_put undershoot noted in cont.12).

**OPEN (needs guest-side correlation — the one thing the host log cannot show):** is the wait the
TAIL of CTX1 `cuCtxDestroy` (scrubberDestruct, which SHOULD break at 4s — so why 120s?) or the START
of CTX2 `cuCtxCreate` (a fresh CeUtils scrub of the new ctx)? Disambiguate by capturing cupctx2
STDOUT: does `[CTX1] CTX DESTROY OK` print (⇒ hang is in CTX2 create) or not (⇒ hang is CTX1
destroy)? That single boolean picks the fix site. ★ NEXT EXPERIMENT: fresh boot, run cupctx2 with
full stdout saved + (optionally) a kernel trace of the CeUtils finishPayload poll site; correlate the
stdout boundary with the qemu-log spin start.

---

## cont.18 (2026-06-21) — DEFINITIVE: hang is CTX2 cuCtxCreate waiting on an UNDELIVERED GSP os-event (SWGEN0), not a CE scrub

Fresh instrumented repro (cupctx2 N=256, full stdout captured). The stdout settles every prior
ambiguity:
```
[CTX1] ... RESULT bad=0 -> PASS        ; CTX1 compute CORRECT
[CTX1] CTX DESTROY OK                   ; CTX1 fully torn down, clean
[CTX2] cuCtxCreate...                   ; LAST line printed, then rc=124 (hang)
```
So: **CTX1 fully works (compute + clean destroy); the hang is unambiguously inside CTX2
`cuCtxCreate`** (`[CTX2] CTX OK` never prints). The test's inline comment calling cuCtxSynchronize
"the #12 hang point" is MISLEADING — the hang is earlier, in create.

**The 4s CE-scrub timeout is NOT the 90s hang (kills the cont.11/L1 finishPayload theory for #12).**
Guest dmesg shows only `scrubberDestruct: Timed out ... ce_utils.c:349` at **131s** (>90s SIGINT) =
post-kill teardown. That wait's timeout is **4000 ms** (`_threadNodeCheckTimeout: Timeout was set to
4000 msecs`), so it BREAKS at 4s — it cannot be the source of a 90s hang. The finishPayload forge
(cont.11) fired 372× and the qemu log shows NO scrub timeout during the run; CTX1's CeUtils is fully
covered. #12 is a *separate, ~indefinite* wait.

**What CTX2 actually waits on (qemu-log forensics):**
- Spin region lines 432035→442363 (~10K lines): the SAME bare poll loop as cont.17 — `RD 0x0`
  (PMC_BOOT_0 = osIsGpuLost liveness) + `RD 0xb81008..b8101c` (INTR LEAF2-7), leaf4=0x08000000
  (vec155/SWGEN0 bit27 STALE-set), never RW1C-cleared, TOP (0xb81600) never read. = an indefinite
  *interrupt/event* wait, NOT an ISR, NOT a 0x110094 GSP-queue poll.
- **Emulator is 100% idle during the spin** (0 non-BAR0 trace lines): no RPC, no `M5: chan_exec`,
  no doorbell (0xbb0090), no `M7: delivered os-event`.
- Last RPC before spin = `fn=76 cmd=0x20800a38` = `NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE`
  (a GR/FECS internal control, part of GR-context setup in cuCtxCreate), answered with the golden
  24-byte `ctl_20800a38` reply (status=0). Appears EXACTLY ONCE in the whole log (unique to CTX2).
  The guest got a valid answer, then waits for an event.

**Root-cause mechanism (os-event/SWGEN0 delivery, gpu_emul.c):**
- `nvkvm_gsp_deliver_events()` (≈L1541) posts a POST_EVENT per registered os-event + `raise_swgen0`
  (sets leaf4 bit27 vec155 + MSI). It is **gated**: `if (gsp_swgen0_pending) return;` (≈L1553), and
  is **called from exactly ONE site** — the work-doorbell handler, only when a channel completed
  (`any_completed`, ≈L3546). `gsp_swgen0_pending` is set true by raise (L1524) and cleared (L3619)
  only when the guest writes FALCON IRQSCLR `0x110004` bit6 during kgspService.
- **Log proof of the poison:** the LAST `raise_swgen0` is line 360163 (pending→true); the guest's
  LAST IRQSTAT (0x110008) read is line 360110 — BEFORE that raise. After 360163 the guest never
  reads IRQSTAT and never clears (0 writes to `0x110004` in the entire log). So
  **`gsp_swgen0_pending` is stuck TRUE for all of CTX2** — and there are ZERO work-doorbells after
  360163, so `deliver_events` is never even invoked for CTX2.

**Two compounding gaps (either alone can hang CTX2):**
- **(A) Stuck gate.** CTX1's final raise (360163, for events that are then freed at the
  osevent_drop @~365k) leaves `gsp_swgen0_pending` permanently true → blocks every future delivery.
- **(B) Missing trigger.** `deliver_events` only fires on a work-doorbell completion; CTX2's
  cuCtxCreate registers an os-event and waits BEFORE submitting any work-doorbell, so the trigger
  never fires.

**FIX DIRECTION (next session):** make GSP os-event delivery robust to the 2nd-context case.
Candidates, cheapest first: (1) clear `gsp_swgen0_pending` when `osevent_n` drops to 0 in
`nvkvm_m2_osevent_drop` (a stale pending for freed events must not poison the next context); (2)
add a delivery trigger independent of work-doorbells (e.g., deliver on os-event REGISTER, or when a
GSP RPC that the guest will block-wait on is answered); (3) re-raise SWGEN0 if events are pending
but the gate has been closed with no guest IRQSTAT activity. **VERIFY-FIRST instrumentation:** log
(a) every os-event REGISTER (hclient/hevent) and (b) every `deliver_events` EARLY-RETURN (gate hit)
vs actual delivery — re-run cupctx2 to confirm CTX2 registers an event and the gate/trigger is the
blocker, THEN pick the fix. Do NOT analyse past the SIGINT (131s teardown).

---

## cont.19 (2026-06-21) — CORRECTION to cont.18: os-event delivery is RULED OUT (osevent_n=0 for CTX2); the wait is a SYSMEM completion, target invisible to the host log

cont.18 concluded "undelivered GSP os-event (SWGEN0)". That was PREMATURE — I reasoned from the
delivery code without checking whether CTX2 registers any os-event. It does not. Corrected facts:

**os-event delivery is NOT the CTX2 hang cause.** The register site already traces every
NV01_EVENT_OS_EVENT (0x0079). In the log: ALL os-events are client `0xc1d00003`, in two batches
(#1-7 reg@146374-248882 drop@250944-255353; #1-3 reg@353128-353966 drop@365548-365566). **The last
drop brings osevent_n to 0 at line 365566, and ZERO os-events are registered after that** (the spin
is at 432035). So during CTX2's create/spin `osevent_n==0`, and `nvkvm_gsp_deliver_events`
early-returns at its FIRST line (`if (osevent_n<=0) return`) — it never reaches the
`gsp_swgen0_pending` gate. The gate/trigger gaps (cont.18 A/B) are real code smells but are NOT this
bug. (Leaving cont.18's top-line as the diagnosis would mislead — hence this correction.)

**What IS confirmed (still solid from cont.18):** hang is CTX2 `cuCtxCreate` (CTX1 fully works:
compute PASS + clean destroy); the wait is ~indefinite (90s, not the 4s scrubberDestruct).

**What the wait is (narrowed, but target is host-log-invisible):**
- During the spin (432035→442363): **no BAR1 reads at all** + emulator 100% idle + osevent_n=0.
  The bare poll loop is only PMC_BOOT_0 (liveness) + INTR LEAF2-7 (timeout/interrupt housekeeping).
  ⇒ the guest is polling a **SYSMEM location** (guest RAM — NOT in the MMIO trace), i.e. a CeUtils/
  channel **finishPayload semaphore in sysmem**, waiting for a release that never lands.
- **Forge wrong-aperture smell:** the #12 forge writes the finishPayload to **FB** `0x31f8004`
  (`finPHYS`), but every forge logs `FB 0->N` with OLD value ALWAYS 0 (0->80,0->81,...0->85) — the
  FB page it writes reads 0 each time, i.e. nothing accumulates there. Meanwhile cont.11 notes the
  kernel CeUtils channel buffer is **SYSMEM by default**. Strong hypothesis: the guest's REAL
  finishPayload is in sysmem and the forge has been writing to an unrelated/aliased FB page — a
  no-op from the guest's view. CTX1 still completes because its scrub is satisfied by the actual CE
  **method** semaphore (the `M5.15 DMAW` counter releases at 0x140c41010/0x140c41020), not the
  forge; CTX2's create-time scrub waits on the finishPayload the forge fails to land.

**Why the host log is now EXHAUSTED:** the wait target is a sysmem poll (guest RAM reads don't trap),
so no amount of qemu-log mining can reveal the exact address/value. **The definitive next step is
GUEST-SIDE instrumentation:** ftrace / printk the CeUtils `channelWaitForFinishPayload` poll in the
guest open driver (nvidia/src/.../ce_utils.c) to capture the VA/aperture + expected-vs-actual payload
it spins on during CTX2 cuCtxCreate; then fix the forge to land THAT sysmem location (or forward the
real CE method release to it). Guest driver source is mounted at /usr/src/nvidia-580.159.04.
★ Don't analyse past the SIGINT.

---

## cont.20 (2026-06-21) — open-driver source (channel_utils.c/ce_utils.c) GROUNDS the diagnosis: the spin IS channelWaitForFinishPayload; finishPayload is SYSMEM read via pbCpuVA; the #12 forge's FB target is the WRONG aperture (real path = DMAW-to-sysmem)

Read the guest open driver (it's mounted /usr/src/nvidia-580.159.04 AND vendored research_clones/ogkm)
instead of inferring from MMIO. This should have been done first (cont.18 lesson, doubled). Findings:

**The spin is `channelWaitForFinishPayload` (channel_utils.c:344).** One loop:
```
gpuSetTimeout(GPU_TIMEOUT_DEFAULT, BYPASS_THREAD_STATE);
while(1){ if (READ_CHANNEL_PAYLOAD_SEMA(ch) >= targetPayload) break;
          if (gpuCheckTimeout()==TIMEOUT) break;
          if (rmGpuLockIsOwner) channelServiceScrubberInterrupts(ch); else osSchedule(); }
```
This UNIFIES cont.17/19's two rival reads: the invisible part is `READ_CHANNEL_PAYLOAD_SEMA` (a
sysmem MEM_RD32, below) and the INTR-LEAF2-7 reads are `channelServiceScrubberInterrupts` →
`intrServiceStallList` INSIDE the same loop. PMC_BOOT_0 = gpuCheckTimeout/osIsGpuLost. So the spin
is NOT a separate interrupt-wait and NOT a 0x110094 poll — it is this finishPayload poll.

**The poll target + aperture (channel_utils.h:116, channel_utils.c:276-285):**
`READ_CHANNEL_PAYLOAD_SEMA(ch) = channelReadChannelMemdesc(ch, ch->finishPayloadOffset)
 = MEM_RD32(ch->pbCpuVA + finishPayloadOffset)`. pbCpuVA is a CPU mapping of
`pChannelBufferMemdesc`; the kernel CeUtils channel buffer is **SYSMEM** (cont.11) — which is why
the spin shows ZERO BAR1 reads (the guest reads its own RAM, untrapped). The GPU writes the SAME
location via `pbGpuVA + finishPayloadOffset` (NVC8B5_SET_SEMAPHORE_A/B, channel_utils.c:671). Layout
(mem_mgr.c:2236): semaOffset = channelPbSize + GPFIFO_SIZE; finishPayloadOffset = semaOffset + 4
(CHANNEL_HOST_SEMAPHORE_SIZE=4). So finishPayload = gpfifo_va + 0x8004 — the offset the forge already
uses; the bug is the APERTURE, not the offset.

**Reframed root cause (forge writes wrong aperture):** in Mode-2 the host GPU's CE writes HOST memory,
so the emulator must land the completion in the GUEST's copy of the channel buffer = the guest sysmem
page that pbCpuVA reads. The correct mechanism is the **`M5.15 DMAW gpa=...`** path (emulator writes
the guest physical page). The `#12` forge writes **FB `0x31f8004`** (vidmem) — a DIFFERENT aperture —
which is exactly why every forge logs `FB 0->N` with OLD value ALWAYS 0 (it writes a page the guest
never reads). CTX1 completes via the DMAW-to-sysmem path (not the forge); CTX2's finishPayload never
reaches targetPayload in its guest sysmem page.

**Open (now NARROW): why CTX2's sysmem finishPayload falls short** — (i) emulator stops executing
CTX2's scrub submissions, or (ii) CTX2's completion is routed through the FB forge instead of the
sysmem DMAW, or (iii) CTX2's channel-buffer guest-phys page isn't resolved the way CTX1's was. NEXT =
instrument/trace the emulator's write to `finishPayloadOffset` (gpfifo_va+0x8004) for CTX2's channel:
does a DMAW land in CTX2's guest-phys channel page, and to what value vs targetPayload? FIX direction
(address-table-aligned): write CTX2's finishPayload into the guest SYSMEM channel-buffer page (DMAW
path) via the per-channel forward-populated table; retire the FB content-scan forge. ★read the open
driver BEFORE inferring from traces.

---

## cont.21 (2026-06-21) — log-confirmed mechanism: CeUtils 0xc1e00007 is a SINGLETON; forge froze at payload 85; nothing re-fires it for CTX2 (no usermode doorbell after CTX1)

Verified against the same run's qemu log:
- **Last `#12 FORGE` = line 365517, payload 85, channel `0xc1e00007` (gpfifo 0x120064000).** AFTER that:
  ZERO `chan_exec gpfifo=0x120064000`, ZERO work-doorbells `WR 0xbb0090`, ZERO forge writes — all the
  way to the spin at 432035.
- The forge loop (and `chan_exec`) is driven by the **usermode work-doorbell `0xbb0090`**. CTX1's
  matmul rings it constantly, so CTX1's kernel CeUtils scrubs on `0xc1e00007` got executed + forged
  (payload→85). CTX2's `cuCtxCreate` runs scrubs **before any usermode compute**, so `0xbb0090` is
  never rung → the doorbell loop never runs → the forge never advances `0xc1e00007` past 85.
- The pre-spin scrub-like DMAW activity (page-zero + counter bumps at gpa 0x140c4xxxx) comes through
  the **fn=10/RPC path**, not `chan_exec` — i.e. a different code path that does NOT touch the per-
  channel finishPayload forge.

**Mechanism (high confidence): CeUtils is a per-GPU SINGLETON** (the scrubber channel `0xc1e00007`
persists across contexts; finishPayload is monotonic). CTX1's compute drove it to 85 via the
usermode-doorbell-gated forge. CTX2's create-time scrub raises `targetPayload` above 85 but our forge
is **only re-fired by the usermode doorbell**, which CTX2's create never rings → finishPayload frozen
at 85 → `channelWaitForFinishPayload(target>85)` spins until the 90s test kill.

**Why this is the real shape (ties cont.18/19/20 together):** it is NOT os-event delivery (osevent_n=0),
NOT a GSP re-boot, NOT 0x110094. It is a **completion-forwarding gap**: our finishPayload advance is
coupled to the wrong trigger (usermode work-doorbell) instead of to the actual scrub submission. The
FB-vs-sysmem aperture question (cont.20) matters for WHERE to write, but the primary defect is WHEN/
WHETHER we advance it at all for a kernel scrub submitted outside the usermode doorbell path.

**NEXT EXPERIMENT (instrument + rerun, precise):** add emulator logging that fires on EVERY channel
submission path (not just `0xbb0090`): for the kernel CeUtils ring, log {client, gpfifo_va, the
doorbell/RPC that submitted it, gp_put, resolved finishPayload phys+aperture, current value,
targetPayload}. Re-run cupctx2; confirm CTX2's `cuCtxCreate` raises `0xc1e00007` targetPayload>85 and
that no path advances it. **FIX direction:** advance (forge/forward) the CeUtils finishPayload on the
SCRUB SUBMISSION itself (the kernel doorbell / GSP work-submit for `0xc1e00007`), decoupled from the
usermode `0xbb0090` — landing it in the aperture the guest's pbCpuVA reads (cont.20). Then retire the
content-scan forge in favour of the per-channel forward-populated table.

---

## cont.22 (2026-06-28) — MINIMAL repro (create→destroy→create, NO compute) reproduces #12: the hang is in CTX2's GR-context SETUP, not compute (refutes cont.21's cause)

Built `tests/mode2/cupctx2_min.c` (cuCtxCreate→cuCtxDestroy×2, no module/H2D/launch/sync) +
`scripts/mode2_diag/cupctx2_min_run_guest.sh`. Fresh boot, current build (no emulator change).
Result — hangs IDENTICALLY:
```
[CTX1] CTX OK → [CTX1] CTX DESTROY OK → [CTX2] cuCtxCreate...  (rc=124)
```
**So #12 does NOT need CTX1 compute — bare create→destroy→create reproduces it.** This REFUTES
cont.21's stated CAUSE ("CTX1 matmul drove the singleton to 85"): there were ZERO matmul doorbells
(`0xbb0090` count = 0 the whole run). The forge still reached payload **84** on `0xc1e00007` — driven
purely by CTX1's create/destroy scrubs — so the frozen-singleton OUTCOME holds; the trigger was
create/destroy, not compute. `cupctx2_min` is now the canonical, faster #12 repro.

**Clean-log forensics (minimal run):** last forge = line 361804 (payload 84, `0xc1e00007`); last
`chan_exec` on it = 361795 (gp_put=84). CTX2's create activity (361810→spin) is:
- CTX1 teardown: `fn=10` FREE ×38 + `M5.49 ctx-free drop` of clients `0xc1d00003`, `0xc1d0000a`,
  `0xc1e00008`.
- CTX2 GR-context setup: `fn=76` controls `0x20800a38` (GET_FECS_TRACE_HW_ENABLE), `0x20800a6c`,
  `0x20800a70`×2; `M6: BAR2_BLOCK` rebind; `M5.31 GRPT-WR` (GR page-table writes); `fn=70`×1.
- Then the spin. **ZERO `chan_exec`, ZERO `0xbb0090` doorbells in the whole window.**

**Refined location:** the hang is in CTX2's **GR-context setup** (after FECS_TRACE + GR PT writes),
in a `channelWaitForFinishPayload` whose completion the emulator never lands — and CTX2 submits NO
work through a path we execute (`chan_exec`) before waiting. So either (i) the GR-ctx CE scrub is
submitted via a path we don't execute/forge, or (ii) CTX2 waits on a finishPayload from a re-created
CeUtils whose initial scrub we never run. The exact channel + targetPayload live in guest kernel
memory (pChannel->lastSubmittedPayload, the sysmem sema) — NOT visible in the host trace.

**NEXT (definitive): guest-side printk.** Add a print in the open driver's `channelWaitForFinishPayload`
(channel_utils.c:344) logging {channel id/handle, targetPayload, READ_CHANNEL_PAYLOAD_SEMA current}
each spin, rebuild the guest `nvidia.ko` (src mounted /usr/src/nvidia-580.159.04; current .ko in
/home/ubuntu/nvmods), re-run `cupctx2_min`. That nails the exact channel + target CTX2 waits on →
then land that completion (DMAW into the guest sysmem page pbCpuVA reads, per cont.20) on the right
trigger. Host-log analysis is exhausted for this question.

---

## cont.23 (2026-06-29) — ROOT CAUSE PROVEN via guest kprobe + host log correlation: forge writes finishPayload to FB, guest reads pbCpuVA (wrong aperture)

trace_kprobe refuses the nvidia module ("Could not probe notrace function" — module built
without -pg, so ftrace marks all fns notrace). Used the RAW `register_kprobe()` API via a tiny
built module (`scripts/mode2_diag/cupctx2_min_kprobe2_guest.sh`, planted by addr from kallsyms)
on `channelWaitForFinishPayload(OBJCHANNEL *pChannel /*rdi*/, NvU64 targetPayload /*rsi*/)`.
Ran `cupctx2_min`. Result (the 4 calls of the whole run):
```
cwfp pChannel=0xffff8d63c292d408 target=1   ; CTX1 create scrub
cwfp pChannel=0xffff8d63c292d408 target=2   ; CTX1 create scrub
cwfp pChannel=0xffff8d63c292d408 target=2   ; CTX1 destroy (completes -> CTX DESTROY OK)
cwfp pChannel=0xffff8d63f45f0a08 target=84  ; CTX2 cuCtxCreate -> HANGS (never returns)
```

**Guest ground truth:** CTX2 waits in `channelWaitForFinishPayload` on a **NEW** CeUtils channel
object (`pChannel=...f45f0a08`, distinct from CTX1's `...292d408` — CeUtils is destroyed at
CTX1-destroy and re-created for CTX2), for **targetPayload=84**. (CTX2 re-scrubs the global GR
buffers CTX1-destroy freed → 84 vs CTX1's 2.)

**Host correlation (same run qemu log):** the channel the emulator sees as `0xc1e00007`
(gpfifo 0x120064000) advances gp_put 0→**84** (666 chan_execs) and the `#12 FORGE` drives its
finishPayload to **84** — but `#12 FORGE-RESOLVE(BAR1)` pins `finFB=0x31f8004` and every write is
to **FB**. Guest reads `READ_CHANNEL_PAYLOAD_SEMA = MEM_RD32(pbCpuVA + finishPayloadOffset)` — the
channel buffer's own CPU mapping, a DIFFERENT location/aperture than FB 0x31f8004.

**=> ROOT CAUSE (proven both ends): aperture mismatch.** The forge tracks the right channel + right
target (84) but writes the finishPayload to FB `0x31f8004` (BAR1-shortcut resolution), while the
guest reads it via `pbCpuVA`. The guest's sema never reaches 84 → infinite
`channelWaitForFinishPayload`. Exactly cont.20's hypothesis, now confirmed by guest target=84 ==
host forge=84 at different apertures. (CTX1's target-2 scrub completes because its small
completion lands where the guest reads it; CTX2's re-created channel's finishPayload does not.)

**FIX (well-defined now):** write CTX2's channel finishPayload to the location the guest's `pbCpuVA`
actually reads — i.e. resolve gpfifo_va+finishPayloadOffset through the CHANNEL'S OWN GMMU/aperture
(the FORGE-RESOLVE(VAS) path, aperture-aware) and write THERE, not the BAR1-shortcut FB page; or
better, let the real CE SET_SEMAPHORE execution (chan_exec → DMAW to the channel's resolved phys)
carry it and retire the FB content-scan forge (address-table-of-truth). VERIFY with the same kprobe:
after fix, the cwfp(target=84) call must return (sema reaches 84) and cupctx2_min rc=0.

**Repro/tools added:** `tests/mode2/cupctx2_min.c`, `scripts/mode2_diag/cupctx2_min_run_guest.sh`,
`scripts/mode2_diag/cupctx2_min_kprobe2_guest.sh` (register_kprobe module, builds in-guest).

---

## cont.24 (2026-06-29) — the aperture is a RED HERRING: the CeUtils channel is GSP-managed (no VAS we own), so the finishPayload's physical backing is OPAQUE. Three resolutions, three wrong pages. Forge cleanly re-routed through the canonical writer (necessary but INSUFFICIENT).

Implemented cont.23's stated fix (resolve the finishPayload through the channel's OWN VAS,
aperture-aware) and TESTED it on the bench (fresh boot, NVKVM_M2TRACE=1, register_kprobe on
`channelWaitForFinishPayload`, `cupctx2_min`). Then refined it to route the forge through the
hardened M5.18 completion-sema writer `nvkvm_chan_sem_wr32` (client-keyed `m2_cli_vas` resolve +
BAR1-relative redir — the SAME primitive the CE SET_SEMAPHORE pushbuffer parser uses). **Both still
HANG (rc=124).** The fix in cont.23 is DISPROVEN as stated.

**What the run proved (forge VA is right; physical resolution is the problem):**
- The finishPayload VA the emulator forges = `gpfifo_va + 0x8004` = **0x12006c004** (gpfifo
  0x120064000). This is provably correct: GPU releases it via `NVC8B5_SET_SEMAPHORE_A/B` at
  `pbGpuVA + finishPayloadOffset`, and since the captured `gpfifo_va == pbGpuVA + channelPbSize`
  and `finishPayloadOffset == channelPbSize + GPFIFO_SIZE(0x8000) + 4`, the offset reduces to
  `+0x8004` independent of channelPbSize (channel_utils.c:242-250, 671-672). VA is NOT the bug.
- Across THREE attempts the SAME VA 0x12006c004 resolved to THREE DIFFERENT physical pages, none of
  which the guest reads:
    - cont.23 (BAR1 shortcut)         -> FB 0x31f8004   (actually a c1d00001 SCRUBBER's page via the
                                         stomped global bar1off — a mis-attribution; not c1e00007's)
    - cont.24a (own-VAS content-probe) -> SYS 0x149e6c004  (pdb 0x3114000)
    - cont.24b (cli_vas, client-keyed) -> SYS 0x102626004  (climbs 0x44->0x54=84; guest never sees it)
  Forge reaches payload **84** at each page (matches the kprobe's CTX2 `target=84`), but
  `cupctx2_min` still hangs => none of these pages is `pChannelBufferMemdesc`'s real backing.

**ROOT WALL (the real cont.24 finding):** the CeUtils channel (client 0xc1e00007, gpfifo
0x120064000) is **GSP-managed** — its instance-block PDB is EMPTY (emulator logs "M5.14 instblk PDB
empty (GSP-managed)"; `chan_pdb=0` in every `#12-L3c SEMW`). So we hold **no authoritative VAS** for
this channel. The HW/guest path doesn't need one: the guest CPU reads via `pbCpuVA =
memmgrMemDescBeginTransfer(pChannelBufferMemdesc)` (channel_utils.c:276-285), i.e. straight off the
channel-buffer **memdesc's physical pages**; the GPU writes via `pbGpuVA` through the GSP-internal
VAS — both alias the SAME physical backing. We know NEITHER the GSP VAS nor the memdesc PA, so every
emulator-side resolve of 0x12006c004 is a heuristic guess landing on the wrong page. `redir=0x0`
too: `chan_gpfifo_phys` is unset for c1e00007 (M5.16 never pinned its ring), so the BAR1-relative
mirror never fired either.

**=> #12 is NOT an aperture pick.** It is: *we cannot locate the physical backing of a GSP-managed
kernel channel's buffer.* The finishPayload, the GP entries, the pushbuffer all live in that one
opaque memdesc.

**Read-path detail that bounds the fix (channel_utils.c:272):** the CPU read uses BAR1
(`bUseBar1` -> TRANSFER_FLAGS_USE_BAR1) OR a sysmem shadow (SHADOW_ALLOC + SHADOW_INIT_MEM that
DMA-copies the real memdesc in). Spin-window log forensics (after the last payload=84 forge): the
window is dominated by CTX2 GR page-table writes (10310 GRPT-WR), NOT a hot trapping BAR1 read
(~164 plain BAR1 RD over ~47s) — consistent with the NON-BAR1 (sysmem-shadow) path, i.e. the
channel buffer is sysmem and the read is a plain guest-RAM read (no trap; invisible). If so the
memdesc backing is a guest-RAM GPA and a correct `pci_dma_write` there WOULD be seen — we just don't
know the GPA.

**Code state (committed, m2trace-gated, no default-path impact):** the doorbell forge no longer does
its own bespoke (A)BAR1/(B)own-VAS resolve + `nvkvm_phys_wr32`; it now calls
`nvkvm_chan_sem_wr32(s, gpfifo_va+0x8004, fin_payload, &redir)` — one resolver, client-keyed, shared
with the CE-parser, address-table-aligned. This is the RIGHT substrate (once we have the true PA, it
lands there) but is **necessary-not-sufficient**. The `fin_fb/fin_sys` struct fields are now unused
(retained); `fin_via_vas` was added then removed within this cont.

**NEXT — get the memdesc PA (two routes, pick one):**
  (1) **Guest ground-truth (fast, decisive):** extend the register_kprobe to dump, for the hanging
      `pChannel`, `bUseBar1` + `pChannelBufferMemdesc` -> its `_pteArray[0]`/PhysAddr (+ the current
      finishPayload value via `pbCpuVA+finishPayloadOffset` and `slow_virt_to_phys`). Needs OBJCHANNEL
      + MEMORY_DESCRIPTOR field offsets (objdump `channelReadChannelMemdesc` / the channel-setup fn,
      or offsetof from the open headers). Confirms the GPA and whether it's sysmem vs BAR1.
  (2) **RPC-snoop (the production fix, address-table-of-truth):** capture the channel-buffer
      memdesc's PA list from the channel-alloc / MAP RPC to GSP (forward-populate the table), so the
      forge (and the real CE SET_SEMAPHORE parse) write the guest-known backing. This is the
      "forward-populated by RPC" path docs/design/mode2_address_table.md mandates.
  Then VERIFY with the SAME kprobe: cwfp(target=84) returns + cupctx2_min rc=0, then cup8/LLM/PyTorch
  no-regress.

**Tools added this cont:** `scripts/mode2_diag/m570_ctx2_fix_verify_host.sh` (fresh-boot +
kprobe2 + forge-resolution verdict greps).

---

## cont.25 (2026-06-29) — GROUND TRUTH via guest OBJCHANNEL/memdesc kprobe: the hang channel reads its finishPayload through **BAR1 (vidmem)**, stuck at 0. cont.24 forged SYSMEM = wrong aperture (opposite direction).

Built a register_kprobe that reads OBJCHANNEL fields by offsets computed from the open headers
(`g_mem_mgr_nvoc.h` OBJCHANNEL, `g_mem_desc_nvoc.h` MEMORY_DESCRIPTOR) — validated in-run
(`finishPayloadOffset=0x6c004 == channelPbSize 0x64000 + 0x8004`; `pbGpuVA=0x120000000 → gpfifo
0x120064000` matches the emulator). Tool: `scripts/mode2_diag/cupctx2_min_kprobe3_guest.sh` +
`m571_ctx2_groundtruth_host.sh`. The decisive field: `slow_virt_to_phys(pbCpuVA + finishPayloadOffset)`
= the guest-physical address the read actually hits, plus `bUseBar1` and the live value.

**Result (cupctx2_min, the 2 CeUtils channels):**
```
CTX1 chan (target 1,2):  bUseBar1=0  finGPA=0x12867a004 (SYSMEM)  CURVAL=1 then 2  -> COMPLETES
CTX2 chan (target=84):   bUseBar1=1  finGPA=0x108824004 (BAR1)    CURVAL=0          -> HANGS
```

**=> ROOT CAUSE (ground truth, both semantics + aperture):**
- The CTX2 CeUtils channel has **`bUseBar1 = NV_TRUE`** — `channelReadChannelMemdesc` maps the
  channel buffer through **BAR1** (`TRANSFER_FLAGS_USE_BAR1`, channel_utils.c:272), so the finishPayload
  lives in **VIDMEM (FB)** and the guest's poll `MEM_RD32(pbCpuVA+finishPayloadOffset)` is a **BAR1
  read** that traps into the emulator and is served from an FB page. `slow_virt_to_phys` of that CPU VA
  = **0x108824004** (a BAR1-window GPA), and its CURRENT value is **0** (needs 84).
- cont.24's forge resolved the finishPayload VA to **SYSMEM 0x102626004** (cli_vas) and wrote there —
  the guest never reads sysmem for this channel. **WRONG APERTURE, and the OPPOSITE direction** from
  what cont.23/24 chased: the guest reads FB-via-BAR1, we wrote sysmem.
- CTX1's CeUtils is `bUseBar1=0` (sysmem channel buffer); its finishPayload IS satisfied (CURVAL
  tracks target) by the existing sysmem completion path — which is why only the *2nd* context (whose
  CeUtils happens to come up `bUseBar1=1`) hangs. (Why CTX2's is BAR1 and CTX1's is sysmem: not yet
  pinned down — likely BAR1 CPU-access availability differs after the 1st ctx's teardown; not blocking.)

This VINDICATES cont.16/17's original BAR1→FB instinct (right aperture) and explains why cont.23's
"BAR1→0x31f8004" still failed: that was a *scrubber's* page via the **stomped global
`chan_gpfifo_bar1off`**, not this channel's. The aperture was right; the per-channel FB page was wrong.

**THE FIX (well-scoped):** write `fin_payload` to the **FB page the guest's BAR1 finishPayload read
resolves to** — i.e. `bar1_pdb` walk of the channel's finishPayload BAR1 offset
(`chan_gpfifo_bar1off + (gpfifo_va & 0xfff) + 0x8004`, the cont.16/17 shortcut) using a **correct
PER-CHANNEL `chan_gpfifo_bar1off`** (the struct field is global and gets stomped — comment at the
`fin_fb` decl). The blocker to close: **M5.16 does NOT pin `chan_gpfifo_phys`/`chan_gpfifo_bar1off`
for this c1e00007 CeUtils channel** (cont.24 showed `redir=0x0`, so the BAR1-relative mirror in
`nvkvm_chan_sem_wr32` never fired). So:
  (a) make the M5.16 ring resolution capture + store the gpfifo BAR1 base **per-channel** (in
      `nvkvm_chan_entry`), set when the emulator resolves THIS channel's BAR1-written ring, and
  (b) in the forge, for a `bUseBar1` kernel channel, resolve `bar1_pdb(per_chan_bar1off + 0x8004)` →
      FB page and write `fin_payload` there (forward-only).
  Alternative (more general, address-table-of-truth): capture the channel-buffer→BAR1 mapping from the
  RM map RPC / BAR1 PTE writes, keyed by channel, and resolve from that.
  VERIFY with the SAME kprobe: CTX2 `CURVAL` must reach 84 and `cupctx2_min` rc=0, then cup8/LLM/
  PyTorch no-regress.

**Note:** the kprobe's MEMORY_DESCRIPTOR walk gave junk (`addrSpace=0`, `pte[0]=0`) — those memdesc
offsets are off (likely ListNode/checked-build padding) — but it does not matter: `bUseBar1` +
`slow_virt_to_phys(pbCpuVA)` are the authoritative ground truth and both validated.

**Tools added:** `scripts/mode2_diag/cupctx2_min_kprobe3_guest.sh`,
`scripts/mode2_diag/m571_ctx2_groundtruth_host.sh`.

---

## cont.26 (2026-06-29) — BAR1-aperture forge: DIRECTION PROVEN (guest CURVAL 0→84) but ring-capture heuristic is unreliable + can't validate rc=0 under m2trace slowdown.

Implemented the cont.25 fix: (1) in M5.16's scan, capture THIS channel's GPFIFO ring BAR1
page-offset into a new per-channel `chan_fin_ring_off`/`chan_fin_ring_found` even when the
pushbuffer-VAS validation fails (GSP-managed bUseBar1 case), identified by the GP entry decoding
to a pushbuffer pointer `pb < gpfifo_va && gpfifo_va - pb <= 0x100000`; (2) in the doorbell forge,
for a kernel channel, ALSO write `fin_payload` to the FB page the guest's BAR1 poll reads =
`walk_pdb(bar1_pdb, chan_fin_ring_off + (gpfifo_va&0xfff) + 0x8004)`, forward-only, alongside the
existing sysmem `sem_wr32` (for bUseBar1=0 channels like CTX1).

**RESULT — direction PROVEN:** the kprobe3 GT line for the CTX2 hang channel went from
`CURVAL=0` (cont.25, stuck) to **`CURVAL=84`** (this build). So the BAR1 forge DOES reach the page
the guest polls — the aperture fix is correct in principle. cupctx2_min still rc=124, for TWO
reasons that are NOT the aperture:

**(1) m2trace validation is too slow to ever pass.** CTX2's `cuCtxCreate` doesn't *enter* `cwfp(84)`
until t=287s (m2trace logs every doorbell + fb-access → ~5–10x+ slowdown); CURVAL is already 84 at
that entry, so it would return, but no timeout (tried 60s, 240s) catches 287s. The forge is
m2trace-GATED, so I can't run fast (forge off) — must ungate the forge LOGIC (keep logging gated) to
validate at speed.

**(2) ring capture flip-flops → can write the WRONG (scrubber's) FB page.** `ring_off` cycles
0x0 / 0xa0000(→FB 0x3138004) / 0x120000(→FB 0x31f8004) across doorbells; the last payloads 75–84
logged `barFB=FAULT` (not captured). ROOT: a bUseBar1 channel buffer has a **0x64000-byte pushbuffer
= ~100 BAR1-tracked pages**; reading each candidate page at `gp_get*8` and decoding as a GP entry,
the PUSHBUFFER pages' method bytes coincidentally decode to a `pb` in the `< gpfifo_va` window — so
the single-`pb` check matches pushbuffer pages, not just the one gpfifo ring page. 0x31f8004 is a
scrubber's page (cont.23) → writing 84 there is a corruption risk that BLOCKS ungating for cup8/LLM.

**NEXT (two fixes, then validate):**
  (A) **Deterministic ring identification** (replace the single-`pb` heuristic). Options, best-first:
      - Piggyback on however `chan_execute` already CONSUMES this channel's ring (gp_get advances to
        84, so it reads the entries SOMEHOW) — capture the FB page it actually reads the GP entries
        from. READ that path first (why does gp_get advance for c1e00007 with chan_pdb=0 / phys=0?).
      - Or require the candidate page to hold a SEQUENCE of valid GP entries (consecutive 8-byte
        pairs all decoding to pb in the same pushbuffer range) — rejects pushbuffer pages (one stray
        in-range word won't pass).
      - Or capture the gpfifo BAR1 page at GP_PUT-doorbell/GP-entry-WRITE time, keyed by channel.
  (B) **Ungate the forge LOGIC from m2trace** (keep qemu_log gated) so validation runs at speed —
      ONLY after (A) makes the write target reliable (else risk corrupting scrubber pages under
      cup8/LLM). Then: cupctx2_min rc=0 (CURVAL→84, cwfp returns) + cup8/LLM/PyTorch no-regress.

**Code state (committed, m2trace-gated, 0 default impact):** new `chan_fin_ring_off`/`_found` fields
+ M5.16 capture + forge BAR1 write. The aperture mechanism is right; the ring-ID predicate needs to
be deterministic. Tools: `scripts/mode2_diag/cupctx2_min_kprobe3_guest.sh`,
`m572_ctx2_barfix_verify_host.sh`.

---

## cont.28 (2026-06-29) — REFRAME: the 2nd-ctx hang is a USERSPACE busy-poll in libcuda cuCtxCreate, NOT channelWaitForFinishPayload (which only fires at teardown). cwfp/cont.22-27 was a wrong turn.

After the cont.26/27 BAR1 forge made the guest's finishPayload reach CURVAL=84 but cupctx2_min
STILL rc=124, I stopped trusting the cwfp framing and instrumented the PROCESS directly
(/proc/PID/stack, wchan, syscall + gdb), which the cwfp kprobe never did.

**Ground truth (m574 hang-stack + m575 gdb bt + m576 ioctl-decode + m577 random sampling):**
- The hung cupctx2_min main thread is **State=R (running), wchan=0, syscall=running, EMPTY kernel
  stack** for the entire CTX2 `cuCtxCreate` — it is **busy-spinning in USERSPACE (libcuda)**, never
  blocked in a kernel wait.
- Random-interrupt sampling: **13 of 14 samples at the SAME userspace RIP** (a high/vDSO-region
  addr, e.g. 0x7ffc…bb2) = a **tight spin-wait-with-timeout loop** (the lone `ktime_get_raw_ts64`
  /clock read in the hang-stack fits: poll value, check clock, repeat).
- gdb bt: the loop is under `cuCtxCreate_v2` -> several libcuda frames; caught variously in a
  `memset` and in `ioctl`s.  The ioctl stream (m576) is a *progressing* setup sequence, NOT one
  repeated control: RM_ALLOC (0x2b) of GR class 0xc7c0, CE class 0xc7b5, mem class 0x3e; RM_CONTROL
  (0x2a) cmds 0x0080170d / 0x906f0101 / 0xc36f0108 / 0x20801218 / 0x00000d01; a 0x4e MAP — i.e.
  cuCtxCreate makes forward progress through object setup, THEN enters the userspace busy-poll that
  never completes.
- **channelWaitForFinishPayload is a RED HERRING:** it is entered only at teardown
  (consistently test-timeout + ~47s = the RM gpuCheckTimeout after SIGINT), NOT during the test.
  cont.22/23 mistook "last cwfp logged" for "the hang."

**=> ROOT CAUSE (reframed): a libcuda USERSPACE busy-poll in cuCtxCreate waits on a value/state the
emulator never satisfies** (a correctness gap), so it spins until killed.  This MATCHES the older
pre-cwfp framing — [[mode2_cuctxcreate_999_diagnosis]] (MC_SERVICE_INTERRUPTS / interrupt-delivery
hang) and [[mode2_cuctxcreate_pagetable_poll]] (RM busy-walking GR VAS page tables).  The 1st context
passes because its setup state is satisfied; the 2nd context's is not (something not reset/re-armed
across teardown — interrupt delivery, a poll-sema, or a GR/page-table state).

**Code state:** the cont.23-27 finishPayload BAR1 forge (chan_fin_ring_off capture + bar1_pdb write)
is a real-but-SECONDARY completion (it correctly drove the teardown cwfp's CURVAL 0->84) — RE-GATED
behind m2trace (cont.28) so it is NOT default-on (was briefly un-gated in cont.27; unvalidated).
Zero default-path impact.  Keep it for when the teardown completion is needed post-fix.

**NEXT (find the polled value — the actual #12 fix):**
  1. Get the libcuda spin frame's caller (gdb `bt` full at the spin) + disassemble it to find the
     LOAD address X it polls and the value it awaits.
  2. Identify X's backing (sysmem GPA vs BAR/vidmem) and what should set it — likely an
     interrupt/event completion or a GPU-written status the emulator must deliver for the 2nd ctx.
  3. Cross-check against [[mode2_cuctxcreate_999_diagnosis]] (MC_SERVICE_INTERRUPTS) — the 2nd-ctx
     interrupt/event arming may not be re-established after the 1st ctx tears down.
  VERIFY: cupctx2_min rc=0 (CTX2 CTX OK) with a normal timeout, then cup8/LLM/PyTorch no-regress.

**Tools added:** `scripts/mode2_diag/cupctx2_hangstack_guest.sh` (+m574),
`cupctx2_gdb_guest.sh`, `cupctx2_ctrlpoll_guest.sh`, `cupctx2_sample_guest.sh`.

---

## cont.29 (2026-07-04) — Fable 5 fresh-eyes: found the ACTUAL polled value (16 completion semaphores) + root-caused it (stale cross-teardown host backing, {client,VA}-keyed, client+VAs reused). cont.28 confirmed; cwfp saga (cont.22-27) refuted for good.

Bench rebuilt from scratch on a fresh vast box (overlay lost); a Fable-5 subagent was pointed at
#12 with the full ledger + "verify, don't trust me". It CONFIRMED and sharpened cont.28 and
delivered the concrete root cause I (Opus) never reached.

**PROVEN a genuine hang, not slowness:** left CTX2 spinning **22 minutes** (etimes=1350), State=R,
all wait targets frozen. Not the nested-virt tax.

**THE POLLED VALUE (finally identified):** gdb/objdump of libcuda `cuCtxCreate_v2`'s spin frame =
a **wait-ALL on 16 per-channel completion semaphores** living in ONE sysmem pool page at guest CPU
VA **0x20440f000** (16 slots at +0xf00..+0xff0; a `/dev/nvidiactl` mmap). All 16 read 0 → infinite
spin. This is exactly what cont.28 said we needed and couldn't find. `channelWaitForFinishPayload`
is confirmed TEARDOWN-only — the entire cont.22-27 finishPayload BAR1-forge was chasing the wrong
semaphore.

**ROOT CAUSE (new, evidence-backed): stale host backing reused across teardown.**
- Guest RM client **0xc1d00003 is PERSISTENT** across both contexts (only its sub-objects are
  freed+recreated). CTX2 reallocates its compute channels / ctx-buffers / semaphore-pool at the
  **SAME GPU VAs** (gpfifo 0x2002xxxxx, buffers 0x203–0x204xxxxx).
- The emulator's `va_seen` dedup set (`m2_mapped_va[]`, keyed by **{client, VA}** — verified L587:
  `struct { uint32_t client; uint64_t va; }`) and the host FB backings it gates are **NEVER cleared
  on teardown**. So CTX2's doorbell re-sweep finds every VA already "seen" and backs **nothing**
  (`M6.5 enum_gr_sysmem … backed=0`), while CTX1's teardown already tore down the VAS/channels those
  mappings pointed at. Host GPU completion-sema releases land in **stale** pages → libcuda never
  sees the 16 fresh pool semaphores advance.
- NOT a ring/registration/token failure: instrumented `nvkvm_m2_exec_doorbell` (L7241) — CTX2
  **rings all 16 host channels fine** (`RING … gp_get->1`), identical to CTX1.

**PARTIAL FIX — proven to move the needle (mechanism confirmed):** purge the GR-client's compute-
aperture VAs from `va_seen` on compute-context channel free (gpfifo_va ≥ 0x2_0000_0000) so CTX2
re-backs fresh:
- GR-client-scoped purge → pool completions went **0/16 → 8/16** (measured live via /proc/PID/mem).
- Broadening to ALL clients → REGRESSED back to 0/16 (the cross-client-sharing fragility the existing
  `#12 NOTE` comments warn about — forgetting a VA shared with a live sibling client breaks working
  mappings).
- Poking all 16 semaphores to their target (=1) via /proc/mem did **NOT** release the wait → the 16
  are NECESSARY BUT NOT SUFFICIENT: it is a **live** wait — the host must keep EXECUTING CTX2's
  channels (like CTX1), not just reach 1 once.

**COMPLETE FIX (scoped, not yet implemented):** on compute-context channel/ctx free, ACTIVELY tear
down CTX1's stale host backing — free the `m2_fbback` host vidmem objects + unmap them from the host
VAS for the freed compute VAs (so CTX2's re-back is CLEAN, not a no-op `st=0x51 ALREADY`), **scoped
to the GR client** to avoid the cross-client regression. Then forget the `va_seen` entries so re-back
runs. Enabling this needs `m2_fbback[]` (L449: currently only `{fb_base,size,host_qva}`) extended to
`{client, hMem, VA}` so the teardown can find+free the right objects. VERIFY: all 16 pool semaphores
stream to their live targets, `cupctx2_min` rc=0, and cup2 stays rc=0 (no-regression). Code sites:
`nvkvm_m2_ctx_free_drop` L1654 (where to hook the teardown), `nvkvm_m2_va_seen` L6747, `m2_fbback`
L449 (+ M7 REFACTOR note L606 flags this exact struct as debt to retire).

**Bench notes:** deploy footgun — `cp src/qemu/*.c → /opt/qemu-src/hw/misc/` breaks the Mode-1
`../../src/common/…` includes from the build tree; Fable added a symlink `/opt/qemu-src/src →
/workspace/nvkvm/src` (repo-consistent, left in place). "Fresh QEMU boot per run" confirmed load-
bearing (reload against a persistent GSP → cuInit=999). Emulator was REVERTED to clean HEAD (the
partial fix is unproven for no-regression and one variant regresses); bench idle.

**Verdict on the Fable-5 experiment:** it did NOT "break" — it produced the decisive diagnosis + a
mechanism-proving partial fix where the Opus line had stalled. Remaining work is implementing the
scoped active-teardown fix above.

---

## cont.30 (2026-07-04) — Fable-5 implemented the cont.29 backing-teardown (it FIRES correctly) but cupctx2_min STILL hangs → cont.29 is necessary-not-operative. Decisive new fact: the 2nd cuCtxCreate triggers a GSP RE-BOOT, and CTX2 does ZERO device work — it hangs on RE-INIT completions, and the cont.22-27 CeUtils scrub gap RESURFACES in that re-init path.

A Fable-5 subagent implemented the cont.29 plan faithfully and PROVED it fires, but it is not the
operative block for the no-compute repro. Tree reverted to clean HEAD (per discipline); bench idle.

**What was built (reverted, but proven to fire):** extended `m2_fbback[]` + `m2_objs[]` with
`{client, hmem, hdev, va}`; `nvkvm_m2_ctx_backing_teardown()` on the GR client's compute-aperture
**VASpace-free** (moved from channel-free after finding channel-free fires mid-CTX1 on a probe
channel — cuCtxCreate creates+frees a probe channel → don't trigger on channel-free) actively frees
host vidmem objects, munmaps CPU views (copy-back to fb_pages for coherence), drops m2_gpga, dead-
marks m2_objs, purges the aperture's va_seen. M2TRACE at CTX1 destroy:
`#12 CTX-BACKING-TEARDOWN client=0xc1d00003: fbback freed=32 objs freed=16 gpga dropped=16 va_seen
purged=1553`, then CTX2 GSP re-boots cleanly (`posted GSP_INIT_DONE seqNum 913`). Still rc=124.

**DECISIVE NEW FINDING — the operative block is upstream of stale backing:**
- The **2nd cuCtxCreate triggers a full GSP RE-BOOT** (RmInitAdapter re-runs; new GSP_INIT_DONE).
  This is why #12 is a *2nd-context* bug: CTX1 teardown drops the adapter, CTX2 re-inits it.
- After that re-boot, **CTX2 issues ZERO device work** — 0 channel-allocs, 0 GSP RPCs, 0 doorbell
  rings, 0 M6.5 sweeps — then hangs in the userspace 16-semaphore poll (State=R, empty kernel stack,
  confirmed by /proc/PID/stack). CTX1 satisfies its 16 pool semaphores precisely BECAUSE it does
  real device work at CTX1 time (channel allocs + M6.5 sweeps + doorbell RANGs → host executes →
  releases). CTX2 never executes anything, so no releases are produced no matter how clean the
  backing is. (This is exactly cont.29's own "necessary but not sufficient — it is a LIVE wait"
  caveat, now shown to be the whole story for the no-compute cupctx2_min.)
- Guest dmesg (IDENTICAL baseline and fix): UVM `update_completed_value_locked` MAX_JUMP asserts
  (`0x45→0x100000012`, `0xd4→0x100000029`) + `scrubberDestruct: Timed out waiting for the scrub` +
  `pCeUtils->lastCompletedPayload == lastSubmittedPayload @ ce_utils.c:349`. **The CeUtils finish-
  payload completion (the cont.22-27 saga) RESURFACES — in the GSP RE-INIT CE-scrub path.** So
  cont.22-27 was not entirely wrong: it chased a real completion, but at teardown; the load-bearing
  one is the **re-init CE scrub** triggered by CTX2's GSP re-boot.

**=> REFRAMED ROOT CAUSE (cont.30):** the 2nd cuCtxCreate re-boots the emulated GSP; the guest RM's
post-reboot `RmInitAdapter` CE-scrub (and its 16-semaphore pool + CeUtils finishPayload) never
complete because the emulator doesn't forward/complete that re-init CE-scrub work — so libcuda's
userspace wait-ALL on the 16 pool semaphores (guest VA 0x20440f000, cont.29) spins forever. The
stale-backing teardown (cont.29) is a real prerequisite for when CTX2 *does* re-execute, but it is
NOT what unblocks the no-compute repro.

**PRECISE NEXT STEP (Fable's):** instrument WHY CTX2 performs no device work after the GSP re-boot —
the 16-sema poll is entered BEFORE CTX2 allocates any compute channel, so those semaphores must be
released by CE-scrub / RM-re-init work during RmInitAdapter, not user compute. Either (a) the guest
RM's post-reboot CE scrub submits to a GSP-managed channel whose finishPayload the emulator must
complete (the ce_utils.c:349 gap — reuse/repair the cont.23-27 forge but fire it in the re-init
path, on the right channel/aperture, and note the 16 pool sema at 0x20440f000), or (b) CTX2's
re-init channel-alloc RPCs are being dropped/mishandled by the emulator after re-boot (trace the
RPC stream post-GSP_INIT_DONE seqNum 913). Reintroduce the cont.29 backing-teardown TOGETHER with
the re-init-execution fix, not alone. VERIFY: cupctx2_min rc=0 + cup2 no-regress.

**Fable-5 experiment status:** 2 substantial rounds, each producing decisive insight (cont.29 root
cause; cont.30 the GSP-re-boot reframe + resurrected re-init CE-scrub lead) but no green repro yet.

---

## cont.31 (2026-07-04) — Fable-5 round 3: built + PROVED-FIRING two prerequisite fixes (sysmem re-back on stale GPA; USERD-overlay drop on channel-free — a NEW, distinct teardown bug) but cupctx2_min STILL rc=124. GROUND TRUTH nailed: the operative block is UVM's re-init CE-channel completion — after the 2nd GSP_INIT_DONE the guest issues ZERO device work and libcuda spins on 16 UVM CE-channel tracking semaphores that jump to the 0x1_0000_00xx CANCEL sentinel (the MAX_JUMP asserts) because the emulator never executes/completes UVM's re-registration CE channels. Confirms cont.30.

Ran under `NVKVM_M2CEFWD=1 NVKVM_M2TRACE=1`. Baseline #12 reproduced on clean HEAD first (rc=124).
Diff of both fixes saved (not committed): `memory/12_cont31_fixes.patch`. Tree reverted to clean
HEAD `dd9df80`; bench rebuilt clean + idle.

**FIX 1 (built, fires, insufficient) — stale-SYSMEM re-back keyed by resolved GPA.** Extended
`m2_mapped_va[]` (L587) `+{gpa,hmem,reback}`; added `nvkvm_m2_va_find/va_mark_gpa`, a
`back_and_map_sys_ex(out_hmem)`, and `nvkvm_m2_host_rmfree()`. In `nvkvm_m2_leaf_flush`'s <2 MiB
sysmem chunk path: if `{client,VA}` is already backed but resolves to a **different guest GPA now**
(the guest tore down + re-created the mapping at the same VA — the 2nd cuCtxCreate re-allocs its
channels/sema-pool at the SAME VAs but FRESH pages), free the stale host OS-descriptor pin (RM
cascades the free to its map_dma, vacating the VA) and re-back at the new GPA (reback-cap 64). This
is cont.29's "COMPLETE FIX", keyed correctly by PDB-resolved-GPA per the address-table directive.
**PROVED FIRING:** 56–70 STALE-SYS re-backs per run incl. the whole pool page block (VA
0x204400000..0x204417000, e.g. `va=0x20440f000 gpa 0x13eb41000 -> 0x13f34b000 OK`), host-rmfree
rc=0 st=0. **But cupctx2_min still rc=124 and guest dmesg IDENTICAL** — sysmem staleness is a real
prerequisite but NOT the operative block (matches cont.30's "necessary-not-operative").

**FIX 2 (built, fires, insufficient) — NEW BUG: stale USERD m2_fbback overlay diverts CTX2's
GP_PUT.** Found via a decisive CE-INSTR probe: for CTX2's re-alloc'd CE channels,
`pageA(fb overlay)put=1` but `pageB(host USERD qva)put=0` = a **PAGE-IDENTITY DIVERGENCE** (0 in
working CTX1, 2 in hung CTX2). Root: each channel USERD has a paired `m2_fbback` overlay (same
fb_base→host_qva) that makes the guest's BAR1 GP_PUT write land in the host USERD object.
`nvkvm_m2_ctx_free_drop` (L1654) removes `m2_chanbuf` on channel-free but **NEVER removed the paired
m2_fbback overlay**. The guest RE-USES the same USERD FB addresses across teardown, so CTX2's fresh
`back_channel_userd` APPENDS a new fbback at a HIGHER index while the STALE CTX1 fbback (same
fb_base, dead host object) survives at a LOWER index. `nvkvm_fb_host_overlay` scans fbback in order
→ hits the stale entry FIRST → CTX2's guest GP_PUT is diverted into the DEAD host object while the
real host USERD (fresh m2_chanbuf qva the host GPU reads) stays GP_PUT=0 → host never fetches the
GPFIFO, never runs the CE SET_SEMAPHORE. **Fix:** in ctx_free_drop, when removing an m2_chanbuf
USERD, also remove its paired m2_fbback entry (matched by fb_base; swap-remove, nothing stores
fbback indices — safe). **PROVED FIRING + PROVED-CORRECT:** 41 stale overlays dropped;
**CTX2 divergence 2 → 0**. This is a genuine independent correctness bug (will bite once CTX2
executes); ready to land. But cupctx2_min still rc=124 — because the divergence was in CTX1's LATE
channel activity, NOT the CTX2 hang (see below).

**★ GROUND TRUTH — the operative block is UVM re-init CE completion, upstream of both fixes:**
- Log-region correction: the earlier "CTX2" activity (NR>139000) was actually **CTX1**'s create +
  M6.5 re-sweeps. The TRUE CTX2 window is **after the 2nd `posted GSP_INIT_DONE`** (fn-47 UNLOADING
  → GSP re-boot → 2nd INIT_DONE). After the 2nd INIT_DONE there are **ZERO cmdq RPCs, ZERO
  doorbells, ZERO chan_execs, ZERO M6.5 sweeps** — the log goes silent except `SEC2 Booter Unload
  (mbox0=0xff) -> WPR2 down` (fires RIGHT AFTER INIT_DONE) then ahci noise. Exactly cont.30.
- gdb ground truth (prior boot, still valid): the hung thread is State=R userspace libcuda spin
  (empty kernel stack) on a **wait-ALL over 16 entries** (poll-set descriptor @rsp: count=16,
  entries reference per-channel objects; `0x7506b939df90` per-iter check reads a completion word).
  The 16 sema live in a `/dev/nvidiactl` pool page (VA varies per run, ~0x2044xf000); all read 0.
- **The 16 are UVM's per-CE-channel tracking semaphores** (dmesg proves it: `nvidia-uvm:
  uvm_channel.c:205 ... CE 2 unexpected completed_value 0x100000029` and `uvm_gpu_semaphore.c:776
  ... jump from 0x45 to 0x100000012`). **`0x1_0000_00xx` = the cancel/error SENTINEL UVM writes when
  it gives up** on a CE channel whose completion never arrived. So during the 2nd `RmInitAdapter`,
  UVM re-registers the GPU and creates its CE channels, submits scrub/init work, and **the emulator
  never executes/completes that UVM re-init CE work** → the tracking sema never advance → UVM
  cancels (MAX_JUMP) → the CeUtils scrubberDestruct times out (`ce_utils.c:349`) → the whole thing
  wedges → libcuda's wait spins forever.

**=> cont.31 REFINED ROOT CAUSE:** #12 = the emulator does not complete **UVM's CE-channel work
issued during the 2nd `RmInitAdapter` GPU re-registration** (post-GSP-reboot). The 16-sema pool +
the CeUtils finishPayload (cont.22-27) are BOTH just symptoms of that one un-executed/un-completed
re-init CE-scrub. Both cont.31 fixes (sysmem re-back, USERD-overlay drop) are real prerequisites for
when CTX2 *does* execute, but neither runs because CTX2 issues no device work post-reboot.

**PRECISE NEXT STEP (cont.31):** stop treating symptoms; make the 2nd-boot UVM CE channels actually
run+complete. Two concrete leads, in order:
  1. **Why does the guest issue ZERO device work after the 2nd INIT_DONE?** Instrument the guest
     kernel (printk in `RmInitAdapter` / the UVM GPU-add path / `nvUvmInterfaceRegisterGpu`) to find
     exactly where it blocks BEFORE submitting UVM's CE work — is it the `SEC2 Booter Unload → WPR2
     down` firing right after INIT_DONE (cont.12's "guest REJECTS the re-boot": is the L3a
     mbox0==0xff unload MIS-firing during the re-boot LOAD?), or a GSP-RM control the re-init awaits?
     If the guest never gets past re-init, the 16 sema are moot — fix the re-boot handshake first.
  2. If the guest DOES submit UVM CE work (rings a doorbell we're dropping): forward+complete THAT
     UVM CE channel (the finishPayload forge exists but is m2trace-gated + was cont.22-27-insufficient
     at TEARDOWN; the load-bearing instance is the re-init scrubber). Land the 2 cont.31 prerequisite
     fixes TOGETHER with this.
Reapply `memory/12_cont31_fixes.patch` when reintroducing. VERIFY: cupctx2_min rc=0 + cup2 no-regress.

**cont.31 experiment status:** round 3 — 2 proved-firing prerequisite fixes (one a NEW bug) + the
sharpest root-cause statement yet (UVM re-init CE completion, with the 0x1_0000_00xx cancel-sentinel
as the smoking gun) but still no green repro. The wall is the post-GSP-reboot re-init path.

---

## cont.32 (2026-07-04) — Fable/Opus round 4: DISPROVES the "2nd cuCtxCreate re-boots the GSP" premise. The reigning cont.30/31 re-boot picture was an ARTIFACT of a `gsp_reloaded` state-machine MISFIRE. Fixed the handshake (correct, default-safe, DISPROVES the premise) + layered the cont.31 prereqs — STILL rc=124. The true remaining block is UVM's OWN tracking-semaphore pool (a UVM-internal RM allocation, NOT the GR-client sysmem sweep) reading a STALE/backwards value across GPU re-registration → MAX_JUMP fatal → CTX2 rings ZERO doorbells. Reverted to clean HEAD; bench idle.

**What was built + tested (reverted):**
- **Handshake fix (the task's step 1+2).** RETIRED the `gsp_reloaded` latch (DMATRFCMD-while-suspended
  = "genuine re-boot") and rewrote the GSP-falcon STARTCPU handler to mirror `kgspTeardown_TU102` /
  `kgspBootstrap_TU102` exactly: a STARTCPU while `gsp_suspended` is the TEARDOWN's own FWSEC-SB ucode
  load (kgspTeardown = FWSEC-SB → SEC2 Booter Unload) — it must NOT raise WPR2 and must NOT post
  INIT_DONE; only the SEC2 Booter Unload (SEC2 STARTCPU + MAILBOX0==0xff, already handled) lowers WPR2.
  Also stopped lowering WPR2 in the fn-47 handler (real HW keeps WPR2 up until the guest's own Booter
  Unload). A genuine re-boot re-posts INIT_DONE via the unconditional boot-args mailbox write
  (kgspProgramLibosBootArgsAddr in NORMAL bootstrap), which fn-47 already re-arms (bootargs_dumped=0).
- **+ the cont.31 prereq patch** (`docs/design/mode2_12_cont31_prereq_fixes.patch`) on top.

**★ DECISIVE FINDING — there is NO pre-kill GSP re-boot on the 2nd cuCtxCreate.** With the handshake
clean, the ONLY GSP boot in the whole run is the first one (`FWSEC ran` + `INIT_DONE seqNum 0` at
log start). The `UNLOADING → teardown FWSEC-SB → SEC2 Booter Unload` sequence fires ONCE, at the very
END of the log (post-SIGINT teardown). **cont.30/31's "2nd cuCtxCreate triggers a GSP re-boot → 2nd
GSP_INIT_DONE" was the `gsp_reloaded` MISFIRE**: during the POST-KILL teardown's FWSEC-SB ucode load,
the old latch read that DMATRFCMD as a "re-boot", raised WPR2, and re-posted a spurious 2nd
`GSP_INIT_DONE` (the "seqNum 843/913" prior rounds saw). Removing it confirms **cont.16 was right all
along**: the GSP stays loaded across CTX1-destroy→CTX2-create; the whole hang is pre-kill CTX2 spin.

**★ WHAT CTX2 ACTUALLY DOES (trace-proven, handshake+prereqs applied):**
- CTX2 create re-allocates its channels (the cont.31 "drop stale USERD overlay" fires 41× on the
  freed UVM CE channels client `0x5c0000xx`, fb_base 0x422x000; and on the compute chan 0x2), and the
  **CeUtils scrubber (client `0xc1e00007`) re-runs to finishPayload 84** (CE-INSTR gp_get 69→84) —
  matching cont.23's "target=84". So CTX2 is NOT "zero device work" in create; the scrubber executes.
- The 16-slot libcuda completion-sema pool (VA 0x204400000..) **IS re-backed to fresh GPAs** by the
  cont.31 STALE-SYS path (1609 re-backs/run; e.g. `va=0x204400000 gpa 0x10230a000 -> 0x141b88000 OK`).
  So the GR-client sysmem staleness is genuinely FIXED.
- **BUT in the CTX2 SPIN window (after the scrubber hits 84, up to the terminal UNLOADING): ZERO
  CE-INSTR from ANY client and ZERO `M5.22 RANG` doorbells.** CTX2 rings no doorbell and executes no
  channel work while spinning. It is blocked in libcuda userspace BEFORE submitting the UVM CE work.

**★ WHY (the true remaining root, cont.32):** guest dmesg (identical with/without the fixes) shows
`nvidia-uvm: uvm_gpu_semaphore.c:776 ... jump from 0x45 to 0x100000012` and `... 0xd4 to 0x100000029`.
Per source (`uvm_gpu_semaphore.c:772`): `if (new_sem_value < old_sem_value) new_value += 1ULL<<32;` —
the `0x1_0000_00xx` is NOT a "cancel sentinel", it is UVM's **32-bit wrap-around handling of a
BACKWARDS semaphore value**. UVM's tracking sema went 0x45 → 0x12 (DOWN). UVM's `uvm_gpu_semaphore_t`
(CPU VA `0xffff…39000`) is a **UVM-internal RM allocation that PERSISTS across CTX1→CTX2** (UVM GPU
registration persists — no fn-47-driven UVM teardown). Its `completed_value` holds CTX1's 0x45; CTX2's
fresh CE channel re-init writes a low value (0x12) into the SAME pool slot → UVM sees a backwards jump
→ `UVM_ASSERT_MSG_RELEASE` MAX_JUMP → UVM global fatal error → the CE channels are aborted before they
run → libcuda's cuCtxCreate wait-ALL on its 16 pool sema never completes → hang. **The cont.31
STALE-SYS re-back does NOT cover this page**: it only re-backs GR-CLIENT (`0xc1d00003`, `0xc1e0…`)
sysmem runs swept via the GR-PT M6.5 path. UVM's tracking-sema pool is a separate UVM/`nvidia-uvm`
allocation (different client/allocation path), never swept, never re-backed → keeps CTX1's residue.

**=> cont.32 ROOT CAUSE:** #12 = UVM's persistent per-CE-channel **tracking-semaphore pool** reads a
STALE (backwards) value when CTX2 re-registers the GPU, because the emulator never refreshes/zeroes
that UVM-owned pool page across the CTX1→CTX2 boundary (it is outside the GR-client sysmem re-back).
UVM's MAX_JUMP fatal then prevents CTX2 from ever submitting its CE work (0 doorbells in the spin).

**PRECISE NEXT STEP (cont.32, for round 5):**
  1. Identify the UVM tracking-sema pool page in the emulator: it is the sysmem page whose guest GPA
     maps to UVM CPU VA `0xffff…39000` (from dmesg) and whose value goes 0x45→0x12. Instrument which
     client/allocation owns it (it is NOT `0xc1d00003`/`0xc1e0…`; likely the UVM RM client or a
     `UVM_*`-path OS-descriptor). Confirm it is NOT in the STALE-SYS re-back set (grep its VA/GPA).
  2. Fix ONE of: (a) extend the STALE-SYS re-back (or a dedicated teardown hook) to cover UVM's
     tracking-sema pool so CTX2 gets a FRESH zero page (UVM then starts a fresh completed_value and no
     backwards jump); OR (b) find the guest-side reason UVM reuses the same tracking sema with a stale
     value across GPU re-registration (does UVM free+realloc the pool on GPU-remove/add? if so the
     emulator is failing to free the old host backing so the fresh alloc lands on the same GPA) and
     make the teardown free it. The clean-handshake code (cont.32) + the cont.31 prereqs are all
     necessary; land them TOGETHER with the UVM-pool fix. VERIFY: cupctx2_min rc=0 + cup2 no-regress.

**KEEP (do not re-derive):** the cont.32 handshake fix (retire `gsp_reloaded`; STARTCPU-while-suspended
= teardown FWSEC-SB, no WPR2 raise / no INIT_DONE; don't lower WPR2 in fn-47) is CORRECT and
default-safe — reapply it in round 5. It disproves the re-boot premise and gives the true clean-state
trace. RULED OUT (cont.32, trace-proven): GSP re-boot on 2nd ctx (there is none); sysmem staleness of
the libcuda 16-sema pool (re-backed fine); USERD-overlay staleness (dropped fine). The block is the
UVM-internal tracking-sema pool, upstream of any CTX2 doorbell.

**cont.32 experiment status:** round 4 — one CORRECT handshake fix that disproves the reigning premise
+ the true root nailed (UVM tracking-sema backwards-jump from an un-refreshed UVM-owned pool page) but
no green repro. Reverted to clean HEAD; bench rebuilt clean + idle.

---

## cont.33 (2026-07-05) — Fable-5 round 5: DISPROVES cont.32's "un-refreshed UVM pool page" premise. Root of the MAX_JUMP is a sema-write VAS COLLAPSE (CeUtils' tracking sema resolving onto UVM's page via a stale FOREIGN chan_pdb). Fixed it (content-validated own-VAS resolution) + enabled the finishPayload forge → BOTH defining #12 kernel errors ELIMINATED (UVM MAX_JUMP + ce_utils.c:349 scrubber timeout GONE, guest dmesg fully CLEAN, cup2 no-regress PASS). BUT cupctx2_min STILL rc=124 — a residual libcuda userspace busy-spin (State=R, no kernel error) remains. Patch saved `docs/design/mode2_12_cont33_fix.patch`; tree+bench reverted to clean HEAD.

**Method:** reproduced #12 on clean HEAD (rc=124). The unconditional `#12-L3 CE-SEM
BACKWARD` diagnostic + its `#12-L3c PROBE` VAS dump (already in the emulator, NOT
trace-gated) gave the decisive trace in ONE run — no guesswork.

**★ TRUE ROOT of the UVM MAX_JUMP (sharper than cont.32).** cont.32 said the block was
UVM's persistent tracking-sema pool reading a stale/backwards value "because the
emulator never refreshes that UVM-owned page." **That framing is wrong.** The trace:
- Writer = CeUtils scrubber `client 0xc1e00007`, completion sema at GPU VA `0x121000010`,
  channel gpfifo VA `0x120064000`.
- `#12-L3c PROBE` proved the resolution: the writer's OWN sticky VAS (`cli_vas[0]`
  pdb=0x2efba5000) **FAULTs** on 0x121000010; the channel's real content-validated VAS
  (own_pdb `pdb=0x2efa6c000`, which maps its gpfifo 0x120064000) resolves it to phys
  `0x1000010` (CeUtils' OWN distinct page). But a STALE FOREIGN global `chan_pdb=0x3114000`
  (client 0xc1d00001, left behind by a sibling channel) made `nvkvm_chan_translate` walk
  0x121000010 under that foreign VAS → phys `0x13482a010` = **UVM's persistent per-CE-channel
  tracking-sema page** (guest CPU VA `0xffff..3482a010`, confirmed by the dmesg assert VA).
  CeUtils' low completion payload (0x1b..) then overwrote UVM's live value (0x72) → 32-bit
  BACKWARD jump → `uvm_gpu_semaphore.c:776` MAX_JUMP → UVM global fatal → CTX2 aborted.
- So it is a **two-semaphores-collapse-onto-one-phys** bug (exactly the class the L3c NOTE
  comments warn about), NOT an un-refreshed page. Zeroing UVM's page (cont.32 fix (a)) would
  NOT help — UVM's `completed_value` (kernel-side, 0x46) persists, so 0x46→0 is ALSO a
  backward jump. And CTX2 REUSES the same GPA (per-boot varies), so (b) "free host backing"
  is moot — the guest, not the host backing, picks the GPA.

**★ FIX 1 (operative, PROVEN, cup2-safe) — content-validated own-VAS sema resolution.**
In `nvkvm_chan_sem_wr32`, when `cli_vas` misses, directly re-run the GPFIFO content probe
(pass 0 = same snooped client, pass 1 = blind: find the chan_vas[] root that maps THIS
channel's gpfifo VA) and walk the sema VA under it — BEFORE `chan_translate`'s blind
foreign-VAS fallback. NOTE: `nvkvm_chan_own_pdb_rs()` cannot be reused directly because it
short-circuits to the global `s->chan_pdb` (line 4208) BEFORE reaching its own gpfifo probe
— so at the CeUtils write it returned the foreign 0x3114000 (my first attempt, gated on
`p==FAULT && dbg_own`, therefore did NOT help). The inline probe fixes that. **PROVEN:**
`CE-SEM BACKWARD` count 18 → **0**; `res=gpfifo-own`, phys now `0x1000010` (CeUtils' own
page); UVM `uvm_gpu_semaphore.c:776` MAX_JUMP + `uvm_channel.c:205` asserts **GONE** from
guest dmesg. Confined to the sema write (global chan_pdb + gpfifo/pushbuffer xlate untouched
— pinning a probed root globally regressed single-context init, per the L3b note).

**★ FIX 2 (necessary, PROVEN) — enable the finishPayload forge for kernel CeUtils by
default.** With FIX 1 alone the MAX_JUMP was gone but `ce_utils.c:349` "scrubberDestruct
timed out" resurfaced (it was previously MASKED — the MAX_JUMP fatal fired first). The
finishPayload forge (#12 L3b, `gpfifo_va+0x8004`) already exists but was `s->m2_trace`-gated
("unvalidated as default-on"). Dropped that gate for kernel CeUtils channels ONLY (user-CE
+ GR still excluded → host executes those for real; cup8/LLM compute round-trip untouched).
**PROVEN:** guest dmesg then goes **COMPLETELY CLEAN** — no UVM assert, no scrubber timeout,
no Xid. Forge fires 327×/run (clients 0xc1d00001/0xc1e00007), payloads climb monotonically.

**+ cont.31 prereq patch** (`mode2_12_cont31_prereq_fixes.patch`) applied cleanly on top:
STALE-SYS sysmem re-back (71×/run, incl. the pool block VA 0x204400000..0x204417000) +
stale-USERD-overlay drop (41×/run). Both proven-firing.

**NO-REGRESSION: cup2 (single ctx, CE HtoD/DtoH round-trip) PASSES rc=0, byte-exact
(0xabcd1234).** All three fixes are safe for the working path.

**★ REMAINING BLOCK (the sole survivor).** cupctx2_min STILL rc=124. With dmesg fully clean,
CTX2's create finishes ALL kernel work, then libcuda **busy-spins in USERSPACE** (main thread
State=R, wchan=0, empty kernel stack — a pure completion-poll, NOT a kernel wait). Key trace
facts that RE-FRAME cont.29's "16-sema pool" theory:
- The parser/forge write ONLY kernel semas in the `0x120xxxxxxx/0x121xxxxxxx` range. **NOTHING
  writes the `0x2044xxxxx` pool region** the whole run.
- `M5.22 RANG` (host-channel doorbell forward) = **0 for the ENTIRE run, including CTX1** —
  yet **CTX1 SUCCEEDS**. So CTX1's cuCtxCreate completes via the kernel `0x121xxx` semas
  (parser+forge), NOT via host execution and NOT via any `0x2044` pool write.
- ⇒ CTX1 and CTX2 have IDENTICAL clean dmesg and IDENTICAL (absent) 0x2044 writes, yet CTX1
  passes and CTX2 hangs. **The difference must be a kernel-sema (0x121xxx) completion target
  that CTX2's forge/parser reaches for CTX1 but not CTX2** — i.e. one more channel/payload
  the emulator under-completes on the 2nd create. This is the precise next lead.

**PRECISE NEXT STEP (cont.33, for round 6):**
  1. **Install gdb on the guest** (NOT present — blocked this round's userspace backtrace).
     Catch the CTX2 spin frame: identify the EXACT semaphore VA + awaited value libcuda
     polls (cont.29 said the 16-slot pool @ ~0x2044xf000, but the trace shows nothing writes
     it AND CTX1 passes without it — so re-confirm what CTX2 actually polls now that dmesg is
     clean; the spin may have MOVED to a kernel-sema target the forge under-completes).
  2. Diff CTX1 vs CTX2 kernel-sema completion in the trace: for every `SEMW`/`FORGE` target,
     compare the MAX payload reached vs the channel's `lastSubmittedPayload`. Find the
     channel whose completion CTX2 leaves 1-behind (the collapse-fix routed CeUtils' sema to
     0x1000010 and it climbed to 0x39 then stopped in the FIX-1-only run — verify the forge
     now carries it to target, and hunt any OTHER channel that stops short on CTX2).
  3. If the awaited sema is genuinely a host-written user-CE completion (needs `RANG`),
     determine why CTX2's user channels aren't rung (token_valid? m2_usermode_qva? the
     0xc1d00001 exclusion mis-scoping a CTX2 channel?).

**KEEP (do not re-derive):** FIX 1 (content-validated own-VAS sema resolution — kills the
UVM MAX_JUMP collapse) and FIX 2 (kernel-CeUtils finishPayload forge default-on — kills the
scrubber timeout) are CORRECT and cup2-safe. Re-apply `mode2_12_cont33_fix.patch` (315 lines,
includes the cont.31 prereqs) and continue from the userspace completion-poll layer above.
**RULED OUT (cont.33, trace-proven):** cont.32's "refresh/zero UVM pool page" (wrong — it's
a VAS collapse, and completed_value persists so zeroing back-jumps too); a GSP re-boot on
2nd ctx (there is exactly ONE UNLOADING, at terminal SIGINT teardown — so the cont.32
handshake fix is NOT needed for this hang and was NOT applied this round).

## cont.34 (2026-07-05) — Fable-5 round 6: #12 RESOLVED. cupctx2_min rc=0 (both contexts), cup2 rc=0.

Started from round 5's validated patch (`mode2_12_cont33_fix.patch`: kernel side
clean — UVM MAX_JUMP + scrubber timeout GONE — but a residual libcuda userspace
busy-spin). Confirmed the round-5 baseline (clean dmesg, cupctx2_min rc=124), then
gdb'd the CTX2 spin and diffed CTX1-vs-CTX2 execution in the emulator trace. Three
distinct, sequential root causes — each fix advanced the host-GPU fault to the next:

**THE POLLED VALUE (gdb-confirmed).** CTX2's `cuCtxCreate_v2` spins in a userspace
wait-ALL on **16 per-channel completion semaphores** in a `/dev/nvidiactl` sysmem
pool page at guest CPU VA **0x20440ff00..0x20440fff0** (16 slots × 0x10, target=1,
stuck 0). The tracking-sema chain is `entry.tracker+0x9410` → `+0x20` memdesc →
`+0x10` = the sema CPU VA. Confirmed cont.29's "16-sema pool" — but the pool is
written by the HOST GPU's CE/GR `SET_SEMAPHORE`, not by the emulator's parser
(zero SEMW to 0x2044xxxxx all run), so it needs REAL host execution of CTX2's
channels, exactly like CTX1.

**FIX A — reuse-vs-mint the per-client cvas + compute-aperture va_seen flush.**
The guest RM client (0xc1d00003) PERSISTS across cuCtxDestroy→cuCtxCreate; only its
TSGs/channels are freed+recreated at the SAME compute VAs (gpfifo 0x2002xxxxx,
pushbuffers 0x2024xxxxx, pool 0x2044xxxxx) with FRESH guest GPAs. The old code
minted a fresh empty host VAS for CTX2 but the global `va_seen` dedup (truthful for
CTX1's VAS) made every re-back sweep back NOTHING → host GR channel FAULT_PDE'd on
its own GPFIFO (empty directory). Reusing CTX1's VAS fixed the PDE but thrashed the
STALE-SYS re-back (4000+ host-rmfree/run, unmap windows → FAULT_PTE on the pool).
**Winning combo: mint a FRESH VAS per context (no stale collisions) + on compute-
channel teardown FLUSH the client's compute-aperture (VA≥0x2_0000_0000) sysmem host
pins from `m2_mapped_va` (free the pin + drop the entry)** so CTX2's fresh PT walk
re-backs the ENTIRE working set (pushbuffers, pool, and — via the same dedup — the
vidmem GPFIFOs) CLEANLY into the fresh VAS with zero st=0x51 collisions. Scoped to
the freed client + compute aperture + only when a compute channel was actually freed
→ single-context paths (cup2/cup8/LLM: no mid-run compute-channel free) untouched.
This alone moved the fault PDE→PTE and got **8/16** pool semas completing.

**FIX B (the operative one) — schedule the 2nd context's GR TSG.** The 8 STUCK
semas belonged to the 8 rl=0 GR-family channels (chid 12-19): they RANG
(GP_PUT=1) but the host never consumed (gp_get stuck 0), while the 8 rl=1/rl=2 COPY
channels RAN (gp_get=1). Root: those 8 channels share the GR TSG (`m2_gr_tsg`), and
the exec_doorbell (M5.9) ring path SKIPS scheduling the GR TSG — it assumes M5.8
`doorbell_setup` already `GPFIFO_SCHEDULE`'d it. But doorbell_setup early-returns on
the sticky `m2_doorbell_ready` (set in CTX1), so CTX2's FRESH GR TSG (0x5c000048)
was **never scheduled** → its channels are off-runlist → host ignores the ring.
Fix: in exec_doorbell, `GPFIFO_SCHEDULE` the GR TSG exactly once whenever it differs
from the last-scheduled handle (`m2_gr_tsg_sched`, new field). The M5.33 st=0x57
noise the old skip avoided was RE-scheduling an already-scheduled TSG — the one-shot
guard prevents that. With this the GR channel executes (gp_get advances 0→4) and all
16 pool semas advance → `cuCtxCreate` returns.

**RESULT.** `cupctx2_min` **rc=0** — `[CTX1] OK`, `[CTX2] OK`, VERDICT PASS (2
contexts, create→destroy→create→destroy, no compute). No-regression: **cup2 rc=0**,
CE HtoD/DtoH byte-exact (0xabcd1234), RTX 3060 enumerated, compute=8.6. All fixes
are teardown-scoped or 2nd-context-scoped → the single-context working path is
untouched. Committed to `src/qemu/nvkvm_gpu_emul.c` on `consolidation` (patch:
`mode2_12_cont34_fix.patch`, includes the round-5 cont.33 changes so the tree is
complete).

**Fix sites** (all in `nvkvm_gpu_emul.c`): `nvkvm_m2_ctx_free_drop` (compute-
aperture sysmem-pin flush on compute-channel free; `freed_compute_chan` flag);
`nvkvm_m2_exec_doorbell` (GR-TSG one-shot `GPFIFO_SCHEDULE`); new struct field
`m2_gr_tsg_sched`; `nvkvm_m2_doorbell_setup` records the scheduled GR TSG; M5.25
ring-loop skip re-keyed to `m2_gr_tsg_sched`.

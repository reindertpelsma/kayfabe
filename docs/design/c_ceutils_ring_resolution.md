# How the C resolved the CeUtils ring VA `0x1_2006_4000` — evidence brief for the Rust port

Status: evidence dossier, 2026-08-08. Read-only extraction from the C research artifact
(`/workspace/nvidia-gpu-passthrough`, branch `consolidation`, HEAD `8baf4f2`). Every claim below
is either a citation into that repo (labelled `[src]` = read from code/doc, `[measured]` = the C
observed it on the bench and the cited doc names the run) or is labelled `[inferred]`. Where the
C's own record disagrees with itself, the disagreement is reported, not adjudicated.

Primary sources: `docs/design/mode2_2nd_context_hang.md` (the #12 dossier, cited below as
`#12:<line>`), `docs/design/mode2_address_table.md` (`AT:<line>`), `src/qemu/nvkvm_gpu_emul.c`
(`C:<line>`), `docs/BENCH_REBUILD_NOTES.md` (`BN:<line>`). All line numbers are at `8baf4f2`.

The Rust's blocker for context: the CeUtils channel's ring VA `0x1_2006_4000` refuses with
`NoVas(ChanId(1))` during `memmgrTestCeUtils`. The C hit the same channel at the same VA —
`gpfifo VA 0x120064000`, CeUtils scrub channel, client `0xc1e00007` (#12:34-35) — and spent
cont.1→cont.34 of the #12 dossier on it. `0x120064000` recurs across boots and even across two
concurrent processes (BN:221-223, real GA106s, 2026-07-25), so the address itself is a stable
landmark of the stock driver's kernel-CeUtils layout. `[measured]` (recurrence) / `[inferred]`
(that it is deterministic RM layout rather than coincidence).

---

## 1. The mechanism, in order: how `0x120064000` became a physical address

### 1.1 What supplied the PDB — snooped GSP RPCs, two transports `[src]`

The C's channel-PDB source is a **snoop of the guest's own GSP RPC stream**, not any read of
instance blocks (those read empty — see §3). Two capture sites, both in the fn=76
(GSP_RM_CONTROL) RPC decoder:

- **`0x90f10106` `VASPACE_COPY_SERVER_RESERVED_PDES`** (C:2704-2734): records
  `pdb = levels[0].physAddress` (params offset `cmd+160`), keyed by
  `{hvas = control hObject (cmd+84), client = hClient (cmd+80)}`, with `root_sys = false`
  ("FB-rooted (GSP-client)", C:2716). The struct comment is explicit about why this is the
  source: *"This is the channel PDB source (the GSP-managed instblk is empty in our FB)"*
  (C:365-370). `[src]`
- **`0x801813` `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`** (C:2736-2787): UVM's transport for
  handing RM/GSP the root of a **UVM-managed user VAS**. Captures `{physAddress, aperture
  flags[1:0], hVASpace}`; `root_sys = (flags & 3) != 0` (C:2753). It is **appended as an extra
  candidate, never overwriting** the RESERVED_PDES root — the two can differ for the same
  hVASpace: *"0x3114000 from RESERVED_PDES vs 0x3400000 from SET_PAGE_DIRECTORY"* (C:2754-2756).
  `[src]`

Both feed `chan_vas[]` (C:380) and a **sticky, never-freed client-keyed copy** `m2_cli_vas[]`
(C:382-393) used only for semaphore writes — added because the CeUtils channel's own VAS handle
is freed by the guest before the channel runs (#12:36-38, #12:472-476).

### 1.2 How the walk was seeded — the resolver ladder `[src]`

Two ladders. `nvkvm_chan_own_pdb_rs` (pick a root for the executing channel, C:5322-5423):

1. handle-keyed: `chan_vas[i].hvas == chan_hvaspace` (C:5326-5333);
2. the instance-block root `chan_pdb` (RAMIN+0x200), *if* the fake GSP ever wrote it
   (C:5334-5336) — for GSP-managed channels it never did (§3);
3. **M5.36 content-probe**: walk every captured root against the channel's own gpfifo VA and
   take the first that yields a valid leaf; pass 0 restricted to roots owned (directly or
   dup-edge-linked) by the executing channel's client, pass 1 blind (C:5337-5403).

`nvkvm_chan_translate` (translate an arbitrary VA in channel context, C:5436-5499): the
`GPU_PROMOTE_CTX` side-table `va_map[]` first (client-scoped, "authoritative and required for
GSP-managed VASes whose leaf PTEs never land in our FB", C:5438-5448), then `chan_pdb`, then the
hVASpace-matched root, then client-owned roots, then a **blind any-VAS pass** as last resort
(C:5492-5496).

⚠ Note for the port: steps 3 and the blind pass are *heuristics* (see §3 and §5.9) — they are
the parts the Rust's forward-population rule exists to replace, and the C itself paid for them
(the cont.33 semaphore collapse, §5.9).

### 1.3 The walk itself, and the FB-vs-sysmem decision `[src]`

`nvkvm_walk_pdb_root` (C:4921-5015) is a GMMU **VER2** walk from an explicit root:

- levels: PD3 `va[48:47]` → PD2 `va[46:38]` → PD1 `va[37:29]`, 8-byte PDEs (C:4931-4934); then
  PD0 as a 16-byte dual PDE at `va[28:21]` (C:4965-4969); then the small (4 KiB, `va[20:12]`)
  or big (64 KiB, `va[20:16]`) page table (C:4982-4996).
- leaf special cases: a **PD1-level 512 MiB leaf PTE** (C:4936-4956 — the #13 fix; GA10x
  `bPageTable` at that level, and the guest kernel-RM CeUtils identity-maps the whole FB heap at
  512 MiB pages, so without this case the walker faulted) and a PD0-level 2 MiB leaf
  (C:4971-4981).
- **the aperture fork is taken at EVERY level**, not once: the root aperture comes from the
  captured `root_sys` (C:4930); each PDE's bits `[2:1]` (1=VID, 2/3=SYS, C:4957-4960) select
  where the *next table* lives; the leaf PTE's bits `[2:1]` (0=VID, 2/3=SYS, C:5003-5013) set
  `*out_sys` for the final page. Reads/writes then route by `out_sys`: sysmem → DMA to guest
  RAM, vidmem → the emulated-FB backing (`nvkvm_phys_rd32/wr32`, C:5502-5517).

### 1.4 The decisive lines for `0x120064000` itself

The 2026-06-17 overnight trace (#12:337-345): *"The CeUtils gpfifo VA `0x120064000` DOES resolve
via PDB walk. Both captured roots of its VAS (`hVASpace=0xcaf00005`: `0x3114000` from
RESERVED_PDES and `0x3400000` from SET_PAGE_DIRECTORY) agree it maps to sysmem phys
`0x165664000`"*, and the earlier `picked_pdb=0` was *"a content-gate artifact"* — the resolver
only pinned a PDB when the GP entry read non-zero, so an idle ring demoted a resolvable channel
to the heuristic (#12:346-364). `[measured]` — but read §2 before trusting the SYS verdict:
the C later reattributed that exact walk.

---

## 2. ★★ SYSMEM or FB? The C's record is genuinely inconsistent — and the later entries refute the earlier reading

This is the section the Rust asked about (`:342-345` vs `:599`/`:640`), and the honest answer is:
**the C's own record contradicts itself across captures, the later entries explain the earlier
ones away as foreign-VAS aliases, and the ground truth (obtained only by in-guest
instrumentation) is that the aperture is a per-instance property that differed between the two
CeUtils channels of one run.** In sequence:

1. **2026-06-17** (#12:342-345): walk says **SYS `0x165664000`**, "the walk is correct".
2. **cont.7** (#12:597-606, 2026-06): *"the scrub channel's true root is not observable in our
   state. No captured PDB correctly roots its own buffer (`gpfifo 0x120064000` → real FB
   `0x31f0000` …): `0x2efba5000` (its freed explicit VAS) FAULTs; `0x2efa6c000` (sibling) maps
   to a sparse/wrong page; **`0x3114000` (UVM) resolves only the sysmem host-sema**"* — i.e. the
   item-1 SYS result came from walking a **foreign** (UVM) VAS, not the channel's own.
3. **cont.8** (#12:639-645, 2026-06-19): exhaustive check — the sibling's `0x2efa6c000` holds
   `0x120064000` only as a sparse identity reservation (`fb=0x64000 val=0`); *"The buffer's real
   FB (`0x31f0000`) is reachable only via the BAR1 VAS."*
4. **cont.12** (#12:866-870, 2026-06-21): `0xc1e00007`'s own-VAS walk resolves its finishPayload
   to a sysmem alias `0x14f26c004`, *"NOT where the guest reads it"* — the C deliberately
   reordered to BAR1-primary to protect the proven FB page `0x31f8004`.
5. **cont.25 kprobe ground truth** (#12:1360-1378, 2026-06-29): within ONE `cupctx2_min` run,
   CTX1's CeUtils channel had `bUseBar1=0`, finishPayload GPA `0x12867a004` (**sysmem**),
   completed; CTX2's had `bUseBar1=1`, finishPayload GPA `0x108824004` (**BAR1/vidmem**), hung.
   Why one instance is sysmem and the other vidmem was *"not yet pinned down"* (#12:1377-1378).
6. **2026-07-25, real GA106** (BN:216-223): the same gpfifo VA `0x120064000` is used by both of
   two concurrent processes, and all successful resolution went through two *shared* roots —
   `pdb=0x2efa6c000` (**FB**, 319 hits) and `pdb=0x3118000` (**SYS**, 36 hits), with the
   executing channels carrying `chan_pdb=own_pdb=0x3118000`.

Also on record: the **sibling** scrub channel (`0xc1e00008`, PDB `0x2efa6c000`) had its
finishPayload `0x42006c004` resolve to **sysmem `0x144a48004`** and work (#12:38-41), while the
`0xc1e00007` finishPayload was vidmem — same code path, opposite apertures.

**Why no stable answer exists** `[src]`: the aperture of a CeUtils channel buffer is hardcoded
per *use* in the guest driver — general CeUtils passes `_NO_BAR1_USE_TRUE` (`mem_mgr.c:4134`) →
`bUseBar1=FALSE` → sysmem; the memory scrubber passes `_VIRTUAL_MODE_TRUE` with no
`_NO_BAR1_USE` (`mem_scrub.c:154`) → `bUseBar1=TRUE` → vidmem (#12:543-551). And the physical
page is chosen per boot by the guest's CPU-side PMA — *"the guest's CPU-side PMA is the sole FB
allocator … the `0x31f0000` 'collision' is … our resolver guessing"* (#12:25-32). So any single
capture's aperture/phys answer for this VA is a fact about that boot and that CeUtils instance,
nothing more.

**Consequence for the Rust's `Pdb` doc** (which the parent task says currently reads "a per-GPU
FB address"): the C's own capture struct carries a `root_sys` flag precisely because roots exist
in both apertures (C:371-374 — `SET_PAGE_DIRECTORY` roots are "typically in SYSMEM"), and a
SYS-rooted PDB (`0x3118000`) was the executing channel's own root on real hardware on 2026-07-25
(BN:221-222). Both the root and every intermediate table can be in either aperture; the C's
walker forks on aperture at every level (§1.3). `[measured]` (SYS-rooted PDB exists) + `[src]`
(the per-level fork).

---

## 3. Where the PDB came from — and which parts were heuristics

**The C used the `0x90f10106` snoop as the primary source, exactly the transport the Rust has
measured 4× during boot.** It did **not** rely on `SET_PAGE_DIRECTORY` for kernel channels:
*"Kernel-internal VASes (CeUtils scrubber etc.) never take this path"* — `0x801813` is UVM's
transport for user VASes (C:375-379), which only appears once UVM registers a GPU (post-boot,
CUDA time). So the Rust's "zero `SET_PAGE_DIRECTORY` during boot" is **consistent with the C's
record**, not in tension with it. `[src]`

**Instance blocks were a dead end for channel PDBs, twice over** `[measured]`
(`mode2_2nd_context_hang.md:463-469`, cont.5 forge run): the C verified
its RAMIN offsets are right (`NV_RAMIN_PAGE_DIR_BASE_LO` = word 128 = byte `0x200`, #12:463-466)
and then found *"the instblk at `0x2efa6e000` is simply never populated — every
channel logs `M5.14 … PDB empty (GSP-managed)` because our fake GSP (which owns instblk
construction in GSP-client mode) never writes RAMIN+0x200"* (#12:466-469). In a faked-GSP world
the instance-block PDB is *your own output*, not an input — there is nothing to snoop unless you
wrote it.

⚠ **The heuristic the Rust's `bar2.rs` header cites IS a heuristic, say it loudly.**
C:3756-3771: a 4-byte FB write through the PRAMIN window at `(fa & 0xFFF) == 0x200` with
`(val & 3) == 0 && (val & 0xFFFFF000) != 0` is *declared* an instance-block bind, and the
containing page becomes `bar2_inst_block` ("the most-recent one before kbusVerifyBar2 is
BAR2's"). That is pattern-matching guest FB writes — reverse resolution by content, exactly what
the Rust's forward-population rule forbids. It survives in the C only for the narrow BAR2 case
(where the CPU really is handing the fake GSP an instblk it built, and the C has no
transport-level signal for it). `[src]`; classification as a thing to learn from and not port:
`[inferred]`, but the C's own #12 record shows what the same *class* of content-heuristic cost
elsewhere (§5).

**Other heuristics the C used on this channel and later refuted or confined:**
- `bar1_wpg` MRU (most-recently-BAR1-written FB page) as the ring/finishPayload resolver —
  produced a *different channel's* page (#12:45-47) and was formally retired for finishPayload
  (#12:524-525, cont.5, 2026-06).
- the content-gate ("only pin a PDB if the GP entry reads non-zero") — demoted an idle-but-
  resolvable channel to the heuristic cascade (#12:346-364, 2026-06-17).
- the M5.16 single-GP-entry ring predicate — matched pushbuffer pages, flip-flopped across
  doorbells, risked writing a foreign channel's page (#12:1430-1436, cont.26, 2026-06-29).
- the blind any-VAS probe — collapsed two clients' semaphores at the same VA onto one physical
  page (§5.9).

---

## 4. The finishPayload / completion aperture

**Offset** `[src]`: finishPayload VA = `gpfifo_va + 0x8004`, *independent of channelPbSize* —
derived in #12:1288-1292 from `channel_utils.c:242-250, 671-672` (`gpfifo_va = pbGpuVA +
channelPbSize`; `finishPayloadOffset = channelPbSize + GPFIFO_SIZE(0x8000) + 4`).

**Aperture** — per-instance, decided by the guest's `bUseBar1` (§2 item 5 and #12:543-551):
- The instrumented-driver counters, 2026-06-17 (#12:117-119, #12:303-305): destruct #1
  `finVa=0x42006c004 bUseBar1=0` (sysmem) completes; destruct #2 `finVa=0x12006c004 bUseBar1=1`
  (vidmem) is never written and times out. `[measured]` — this is the #12 root-cause line, and
  it is a **CPU-side ground truth** printed from inside the guest driver, not an emulator parse.
- kprobe3, 2026-06-29 (#12:1360-1364): read `bUseBar1` + `slow_virt_to_phys(pbCpuVA +
  finishPayloadOffset)` out of the live OBJCHANNEL. `[measured]`

**Reconciling `#12:69` (`finFB=0x31f8004`) with `AT:199-201` ("vidmem `0x12006c004`")**: they
name different things. `0x12006c004` is the finishPayload **GPU VA** of the `bUseBar1=1`
instance (AT:200 calls its aperture vidmem, matching the counters above). `0x31f8004` is a
candidate **FB physical** for that VA — first produced by the `bar1_wpg` heuristic (#12:45-47),
then declared a wrong contiguity extrapolation (cont.5, #12:496-503), then **re-corrected** in
cont.8 (#12:629-637, 2026-06-19): the instrumented guest's backdoor reported the finishPayload
region at FB `0x31f8000 = gpfifo_FB 0x31f0000 + 0x8000`, contiguous within the 64 KiB channel
buffer — the forge's location had been right and the failure was elsewhere. **The C reversed
itself twice on this exact point**; both reversals are in the dossier. Meanwhile `0x2efa6c000`
at #12:639-645 is the *sibling's PDB*, not a finishPayload address.

**How the answer was actually determined**: not by the emulator's resolver — cont.24
(#12:1293-1300, 2026-06-29) records the same VA resolving to **three different physical pages
across three attempts** (`FB 0x31f8004`, `SYS 0x149e6c004`, `SYS 0x102626004`), none of which
the guest read — but by guest-side instrumentation (NV_PRINTF counters, then `register_kprobe`
on `channelWaitForFinishPayload`). `[measured]`

**What finally shipped** (cont.33, 2026-07-05, #12:1809-1830; landed with cont.34, patch
`mode2_12_cont34_fix.patch`): (FIX 1) semaphore writes resolve under the channel's **own**
client's VAS via the sticky `m2_cli_vas` + an inline content-validated own-VAS probe, never the
stale global `chan_pdb`; (FIX 2) the kernel-CeUtils finishPayload forge
(`gpfifo_va + 0x8004`, forward-only) default-on for kernel CeUtils channels only. Guest dmesg
went fully clean and `cup2` stayed rc=0. `[measured]`

---

## 5. What the C got WRONG on the way — the refuted hypotheses, so the Rust does not re-walk them

The #12 dossier is a debugging narrative; these are its dead ends, each with the line where it
was planted and the line where it died:

1. **The wrong semaphore** — `va=0x121000010` (the UVM *tracking* sema) named as root cause,
   corrected same-day 2026-06-17: the destruct waits on the finishPayload, a different sema in a
   different place (#12:107-157).
2. **The `0x31f0000` "FB collision" as sharing/lifecycle** — an elaborate page-reuse /
   cross-client-aliasing theory (#12:208-241, #12:243-271) killed by the allocator correction
   (2026-06-19): the guest's CPU-side PMA is the *sole* FB allocator, so a phys overlap is
   *"structurally impossible … It is our resolver guessing"* (#12:25-32).
3. **Owning-client overlay release** — freeing a root-freed client's backings yanked a page a
   *different* client was still polling; bench-disproven and reverted, 2026-06-18 (#12:78-86).
4. **The "non-trapping memslot / coherence wall"** — (#12:174-183, #12:527-540) refuted in
   cont.9, 2026-06-20: BAR1 is pure MMIO in this device, *"forge-write and guest-read are
   therefore coherent by construction"*; the memslot claim was a false read of a poll-spin
   detector whose consecutive-read counter resets on any interleaved read (#12:710-715). The
   instrument, not the memory model, was wrong.
5. **"finishPayload is a separate, non-contiguous memdesc"** — cont.5 (#12:487-525) — refuted by
   cont.8's ground truth (contiguous at `+0x8004`, #12:629-637).
6. **"Guest polls SEC2 for a Booter-done signal"** — cont.9 L3 (#12:744-754) — wrong; the SEC2
   polls all completed and the real asserts were elsewhere (cont.10, #12:758-763).
7. **"The 2nd `cuCtxCreate` re-boots the GSP"** — the entire cont.30/31 frame — disproven in
   cont.32, 2026-07-04: the observed "re-boot" was a `gsp_reloaded` state-machine misfire
   dressing up *post-SIGKILL teardown* as a re-acquire; there is exactly one GSP boot per run
   (#12:1717-1724).
8. **The finishPayload as THE #12 hang at all** — the whole cont.22-27 saga: cont.28
   (2026-06-29) showed `channelWaitForFinishPayload` fires only at teardown; the 90 s hang was a
   libcuda *userspace* busy-poll (#12:1477-1479), finally resolved in cont.34 (2026-07-05) as a
   16-semaphore pool waiting on a **GR TSG that was never `GPFIFO_SCHEDULE`d** for the second
   context (#12:1908-1922). The ring-resolution work was real and necessary, but it was never
   the bug that made `cupctx2_min` hang.
9. **"Zero/refresh UVM's stale pool page"** — cont.32's fix direction — disproven in cont.33,
   2026-07-05: the backwards jump came from a **VAS collapse** (a stale foreign global
   `chan_pdb=0x3114000` resolving CeUtils' sema VA onto UVM's page); zeroing the page would
   itself be a backward jump (#12:1789-1808). This is the concrete cost of the blind-probe
   heuristic and the reason C:5346-5372 grew client-scoped passes.
10. **Three resolutions, three wrong pages** — cont.24's summary of the resolver's authority on
    this channel (#12:1293-1300): every emulator-side resolve of the finishPayload VA was a
    guess until guest instrumentation supplied the answer.

The meta-finding is itself evidence: on a GSP-managed channel whose VAS handle the guest freed,
**every content- or MRU-based reverse resolution the C tried produced a wrong page at least
once**, and the fixes that survived (`va_map` from PROMOTE_CTX, snooped `0x90f10106`/`0x801813`
roots, sticky client-keyed sema roots, per-channel pinning) are all forward-populated,
transport-observed facts — the address-table doctrine the C wrote down in AT:10-27 and
AT:268-305 after paying for the alternative.

---

## 6. The three-line summary for a Rust implementer

Given the guest has published `levels[0..n]` of a VAS via `0x90f10106` (and, later, UVM roots
via `0x801813` with an aperture flag):

1. **Record** `pdb = levels[0].physAddress` keyed by `{hClient of the control, hVASpace
   hObject}`, root aperture FB for `0x90f10106`, from the flags for `0x801813`; keep multiple
   candidates per hVASpace — they legitimately differ (C:2704-2787).
2. **Resolve the channel to a PDB by the ctxshare/instblk chain, never the client handle**
   (`hVASpace` if non-null → ctxshare/TSG → device-default; AT:268-305, from
   `kernel_channel.c:1030`) — for `hVASpace=0` GSP-managed channels the device-default VAS is
   *yours to mint and populate*, because you are the GSP that real hardware would have asked
   (AT:307-313).
3. **Walk VER2 with the aperture fork at every level** (PDE bits`[2:1]` 1=VID/2,3=SYS; PTE
   0=VID/2,3=SYS; 512 MiB PD1 and 2 MiB PD0 leaves — GA106 CeUtils identity-maps the whole FB
   heap at 512 MiB, so without that leaf case the walk faults, C:4936-4956); the **leaf aperture
   picks the byte source** — SYS → guest-RAM GPA, VID → your FB backing (C:5003-5013,
   C:5502-5517). A miss is a fault, never a fallback (AT:181-196).

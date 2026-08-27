# RESUME HERE — cold-start state, 2026-08-12 (supersedes the 08-11 handoff)

> ## ⊘⊘⊘ SUPERSEDED 2026-08-15 — **GO TO `../../../nvkvm-rs/docs/design/RESUME_HERE_2026_08_15.md`**
> (in the **kayfabe** repo, `/workspace/nvkvm-rs`, at master `a7de8964`).
> **Thirteen rungs landed on 2026-08-14/15** — 29 merges — and this file predates all of them.
> ⚠ **Three of its framings are now actively wrong**, so do not read forward from here:
> **(1)** four different things wear the word *"drain"* and `budget_hit=true` is the *disposal's*
> and is true on **every** boot; **(2)** *"the guest emits zero TLB invalidates on the compute
> path"* is **FALSE** — 377 per boot as BAR0 writes to `0xB830B0`, a transport no measured zero
> covered; **(3)** `n = 1` is not a grade — a single-boot `43` is wrong **1 time in 5** on these
> boxes.
> ★ The 08-15 doc is the **ORDERING**; the **RECORD** is `git log --merges --since=2026-08-14` in
> the kayfabe repo, where each merge carries its rung's findings and refutations in full.

> ### STATUS — 2026-08-12 / ⊘ **SUPERSEDED — see the block above.** Formerly *"LIVE — THIS IS THE HANDOFF DOC."*
> Supersedes `RESUME_HERE_2026_08_11.md`, which is marked SUPERSEDED-BY at its head. Everything
> needed to continue is here or linked from here. ⚠ **The night-run cron prompts still name the
> 08-11 doc**; they cannot be edited from a later session, so that doc's first line redirects here.

---

## 1. WHERE THE NORTH STAR IS

**`cuCtxCreate` still hangs. `CUP2_RC = 124`.** `CE-SUBMIT → RETIRED` has never printed.

> ### ★★★★★ w271 + ITS ANALYSIS, 2026-08-12 (**NEWEST — read THIS one first**) — **THE CE WALL
> IS CLOSED, THE WALL MOVED TO THE GR ENGINE, AND THE HIGH ADDRESS IS NOT A HOST POINTER.**
> `../../../nvkvm-rs/traces/boots/w271/RESULT.md`, branch `w271-the-extent-key`, boots at rev
> `5feac90`, analysis at **`d5d5c38`**. The w270 correction below is not superseded — it is
> **one rung further on**: its named defect (the pin's identity was the base, not the
> `(base, extent)` pair) is FIXED, and this is what came next.
>
> ★★★ **THE EXTENT FIX WORKED AND THE OLD FAULT IS GONE.** w270's `off` arm faulted at
> `CE2 @ 0x2_04420000`; w271's `pin` arm pins **exactly that address**
> (`OPERAND-PIN va=0x204420000 GREW requested=131072 described=131072`) and has **zero CE
> faults**. Same wall budget (254 s vs 256 s), doorbells **17 → 88**, token `0x0001000f`
> **1 → 69**. ⊘ `CUP2_RC = 124` on both arms still (tenth consecutive) — but the counters all
> moved, and the single remaining `Xid` is on a **different engine**.
>
> ★★★ **FIRST GR-ENGINE FAULT OF THE CAMPAIGN.** `ENGINE GRAPHICS HUBCLIENT_FE faulted @
> `0x75b2_aee00000`, `FAULT_PDE`. Every fault before this one was `engine=Ce`. `HUBCLIENT_FE`
> = front-end method fetch ⇒ **the GR engine is fetching methods**, which is the plane
> `cuCtxCreate` actually waits on.
>
> ### ⊘⊘⊘ AND THE ALARM THAT SHAPE INVITES IS WRONG — do not re-raise it
> `0x75b2_aee00000` is ~129 TB and looks **exactly like a host `mmap` return**. It is not one,
> and reading it as a VA-identity violation costs a lane. Three refutations:
> - The sibling `0x75b2b9000000` is written **by the guest, into the guest's own pushbuffer**,
>   as `NVC7C0_SET_SHADER_SHARED_MEMORY_WINDOW_A/B` (`ogkm-580 clc7c0.h:424`; `_A`'s field is
>   `16:0`, so `0x75b2` is legal and not a truncation).
> - It tracks the **guest's** per-boot ASLR in **both** arms: `off` libcuda@`0x76b5dc200000` /
>   window `0x76b5d1000000`; `pin` libcuda@`0x75b2c4e00000` / window `0x75b2b9000000` / fault
>   `0x75b2aee00000`. Same 32 GiB slot within an arm, different across arms.
> - ★★★★★ **Native, unvirtualised GA106 emits the same shape** — already committed here:
>   `traces/native_dataplane_ga106/` decodes `cup2`'s pushbuffer as
>   `OFFSET_OUT_UPPER=0x00007f4f` / `OFFSET_OUT=0x66200000` ⇒ GPU VA `0x7f4f_66200000`, while
>   the same log's mmap census puts that process's `/dev/nvidiactl` maps at `0x7f4f70ddf000`.
>   **On real hardware the GPU VA IS the process VA.** That is UVM unified addressing.
>
> ⇒ **A `0x7xxx_xxxxxxxx` GPU VA is the NORMAL CUDA regime, measured on bare metal.** The
> `0x2_xxxxxxxx` family every earlier rung lived in is the *other*, RM-managed family. ⊘ **Shape
> cannot discriminate origin**: guest CUDA and host CUDA draw from the same 47-bit space. What
> discriminates is **who wrote it** and **what it correlates with**.
>
> ★★ **MISS vs FAILED DESCENT = FAILED DESCENT, and both walkers agree.** Our `kind="Fault"` is
> `CeResolve::Fault(TranslateFault)` — *"MISS = FAULT, arriving from the guest's own page
> tables"*, a **distinct variant** from `NoPublication`; hardware independently says `FAULT_PDE`
> (a directory, not a leaf). **The address table is behaving** — the guest's own tables do not
> describe that VA. ⇒ The next rung is **UVM's fault-driven population**, not the table. ⊘ Do
> **not** add `0x75b2_…` to the address table: nothing says the guest asked us to map it, and
> populating a VA the guest's own tables leave invalid is the `cap2b` class pointed inward.
>
> ⊘ **Two counting corrections.** (1) The `Xid`'s `channel` field is
> `(runlistId << 24) | ChID` (`g_kernel_channel_nvoc.h:1493`) ⇒ `0x01000011` = runlist 1 ChID 17,
> `0x00000009` = runlist 0 ChID 9. So *"engine changed"* and *"channel changed"* are **ONE
> measurement reported twice**; the substitution is **four** independent facts, not five.
> (2) Channel `0x9` is **ours** (runlist 0, our isolate) — but **we never log host chids**, so
> *which* of our ten materialised host GR channels is unknown. ⚠ Our own `vchid=VChid(0x9)` is a
> **different number space**; conflating them names the wrong channel.
>
> ⚠ `grep 'CUP2_RC=[0-9]*'` still matches `GCC_CUP2_RC=0` — it yields a spurious `CUP2_RC=0`
> on both w271 arms. **Anchor it (`^CUP2_RC=`)**; anchored, both arms read `124`.
>
> ⊘ The w271 summary-line false negative (`placed_as_asked=false` with `memory=0x0` on grown
> runs) **was already fixed** by `11b75a7`, which is *not* an ancestor of the boots' build rev
> `5feac90` — which is exactly why the committed logs still show it. **Nothing to do.**

> ### ⊘⊘⊘ CORRECTION, 2026-08-12 — **THE WAIT WAS SATISFIED
> AND RE-ARMED. THE RELEASE WROTE `2`.** Everything below about *"the guest is blocked on a
> second CE release"* is SUPERSEDED: it was written, the guest consumed it, and it now wants a
> **third**. `../../../nvkvm-rs/traces/boots/w270/RESULT.md` (rev `1b64729`, 2 arms, real
> GA106, branch `w270-the-operand-pin`; pre-registration
> `docs/design/w270_the_operand_pin_prereg.md` at `905f289`, before any of the code existed).
>
> ★★★★★ **THE PIN'S FOURTH SOURCE — the CE launch's own OPERANDS — and the decode AGREES WITH
> HARDWARE TO THE BYTE.** Chan 8's `methods=11 launches=3` submission declares, in the guest's
> own `OFFSET_OUT_UPPER`/`_LOWER`, **`W@0x204420000+0x8000`** — the exact address the host GPU
> faults on, decoded by the chip's own codec out of a binary in which that address **does not
> appear at all** (a negative content check enforces it). ⇒ The first end-to-end validation the
> decode path has ever had against an independent authority: the host GPU's own MMU.
>
> **One variable, `KAYFABE_GUEST_OPERAND` `off`→`pin`, measured:**
> - `Xid` **`0x2_04420000` → `0x2_04428000`**; polled slot `0x20440ff70` **`1` → `2`**; the wait
>   **wants `2` → wants `3`**; chan-8 CE doorbells **2 → 3**; every counter strictly greater.
> - ⊘ **`CUP2_RC = 124` on both arms** — ninth consecutive. But it was pre-registered at
>   `p = .40` as **A10**, a *first-class* prediction that the pin would land, the fault would
>   clear, and the number would not move — so the null could not be reported as "expected"
>   without also reporting what was expected to change.
>
> ★★★★★ **AND THE REASON IS ONE NAMED DEFECT, one layer below this rung's code:**
> ```
> run va=0x204420000 len=32768 → PINNED         memory=0xcafe0055
> run va=0x204420000 len=65536 → ALREADY PINNED memory=0xcafe0055
> ```
> **`pin_guest_ram`'s idempotence key is the VA; the EXTENT is not part of it.** 32 KiB were
> never described to RM, and the row that should have said so said `ALREADY PINNED … 
> placed_as_asked=true`. ⇒ *`already` is true of the ADDRESS and false of the RANGE* — a green
> supply row holding a wall in place. ⊘ Latent since `w265` in the primitive **all four**
> sources share; only surfaceable once a source produced a **growing** run at a repeated base.
> **The next rung is that one predicate**, and the fault address is its falsifier.
>
> ⚠ **`grep -c Xid` reads `1` on BOTH arms.** *A count cannot see a substitution* — `w265`'s
> lesson, reproduced verbatim, and the only reason it was caught is that the grader scores
> engine / client / **distinct addresses** / access type.
>
> ⊘⊘ **Three instrument findings, all of one class — AN ABSENT ARTEFACT READING AS A
> FAVOURABLE MEASUREMENT — in a single rung**, two of them costing no GPU time:
> **(a)** the rung's own cap2b guard **refused the first boot** because the address was in two
> printed *sentences*; it was right to, because a `strings` check cannot tell a literal in a
> sentence from one in a decision, and allowlisting the prose would have certified nothing;
> **(b)** ★★★ **`grep 'CUP2_RC=[0-9]*'` MATCHES `GCC_CUP2_RC=0`** — the guest **compiler's**
> exit status — and so reported **`CUP2_RC=0`**, the campaign's headline success value, on an
> arm that was hanging. `tail -1` does not save it: on any run where the hook aborts early the
> `gcc` line is the *only* match. ⇒ **anchor to `(^|[^A-Z_])CUP2_RC=`**, and render an absent
> line as *"THE MEASUREMENT DID NOT HAPPEN"*, never as empty;
> **(c)** a **missing** `hostdmesg` file read as *"zero faults"* — `stat` prints empty and
> `grep` matches nothing, byte-identical to a clean capture. The real file, once written, held
> an `Xid`.

★★★ **But the wall is now decomposed into three legs, and two are built and hardware-witnessed.**

| leg | what it is | state |
|---|---|---|
| **A — the RING** | the host GR channel is **born** over the guest's GPFIFO | ✔ **BUILT + WITNESSED** (`w261`, `w262`) |
| **B — the CURSOR** | `GP_PUT` is a word the **guest** advances ⇒ the guest's USERD, handed to RM **at creation** | ◐ **IN FLIGHT**, branch `leg-b-userd-adoption-at-creation` |
| **C — the DOORBELL** | trap, translate guest token → host token, ring | ✔ **BUILT** `b734995`, default-off behind `KAYFABE_GR_ROUTE` |

> ### ⊘⊘ CORRECTION, same day — **THE STOOL HAS FIVE LEGS, NOT THREE.** My three-leg model was
> useful and **incomplete**, and w262b's measurement is what found the rest. Four and five are
> **UNBUILT**:
>
> **4 — the PUSHBUFFER PAGES.** The guest's GPFIFO entries name VAs *outside* the leaf we join.
> `[measured, w262b]` `gp[0]` names `0x200400000`, `0x200800000` … while leg A1 joins the
> `0x200200000` leaf. **Nothing joins them.** ⇒ The ring is reachable and *the work it points at
> is not*. ★ Tractable: it is the **same mechanism as A1** — the join needs more sources.
>
> > ### ⊘⊘ CORRECTION, 2026-08-12 — **IT IS THE PIN THAT NEEDS MORE SOURCES, NOT THE JOIN.**
> > Read this before acting on the sentence above; that sentence names the wrong plane and a
> > rung built on it **cannot fire**.
> >
> > `[measured, traces/boots/w263/run_w263_ring_qemu.log, all 8 channels, BOTH arms]` the
> > pushbuffer VAs resolve **`pb=S:0x3d45f000` … `S:0x3e25f000`** — and `CeResolve::tag`'s own
> > doc is the authority on that letter: *"`V` = this device's framebuffer, **`S` = guest
> > RAM**, `P` = peer"*. ⇒ **The pushbuffer pages are in GUEST RAM, not the framebuffer.**
> > (The `Vidmem` that later readings attach to these addresses is the **ring's** aperture,
> > `rng=V:0x1024000`, and the `FwdFault::PushbufferAperture{va:GpuVa(8592179200)}` beside it
> > decodes to `0x200224000` — the ring's VA, not a pushbuffer's.)
> >
> > `kayfabe_rt::ceutils::resolve_leaf_of` answers `(Site::GuestRam, **None**)` for a sysmem
> > resolution **by construction**, and says why in its own comment: *"it is not this source's
> > to join: **the guest-RAM pin owns that plane**."* A third join source would have been
> > handed eight guest-RAM addresses, printed eight refusals, and joined nothing.
> >
> > ★★★ **The mechanism was already built and had no source.**
> > `SharedDoorbell::pin_ring_guest_ram` is the complete chain — VA → address table → GPA →
> > aperture check → the hypervisor's stated layout → file offset → one `OS_DESCRIPTOR` per
> > contiguous run, mapped **FIXED at the guest's own VA**. It is asked about exactly **one**
> > address, the ring's, which is in Vidmem, so on `w263` it refused all eight `NOT IN GUEST
> > RAM` — **by name and correctly**. ⇒ *The pin has never pinned one byte on a live guest,
> > and not because it is broken.* The addresses in the aperture it serves are on the same log
> > line, eight of them, and nothing presents them.
> >
> > ⇒ Leg 4 keeps its shape — *the primitive works, the source list is short* — and changes
> > its verb: **give the PIN a second source.** Built on branch `leg-4-pushbuffer-pin`
> > (`KAYFABE_GUEST_PUSHBUF=pin`); pre-registration `docs/design/w264_pushbuffer_pin_prereg.md`
> > in the `nvkvm-rs` tree.
> >
> > ⚠ The *hardware* half of the w263 reading is untouched: the eight `Xid 31 FAULT_PDE
> > ACCESS_TYPE_VIRT_READ` are real, and `FAULT_PDE` is exactly what a pin at those VAs would
> > install a directory entry for. Only the named mechanism was wrong.
> >
> > ### ⊘⊘⊘ CORRECTION, 2026-08-12 — **LEG 4 IS DONE, AND THE WALL IS NOW ON THE COMPLETION
> > PLANE.** Read this before acting on the `w264` block below; that block's *"the next rung"*
> > has been RUN. `traces/boots/w265/RESULT.md` (rev `2f02621`, 2 arms, real GA106).
> >
> > ★★★★★ **NO CODE WAS WRITTEN. The populate source was ALREADY BUILT and `w264` ran with it
> > OFF** — `KAYFABE_PT_WITNESS_EXEC`. `w261`/`w262` armed it; `w263_run.sh` and `w264_run.sh`
> > silently dropped it, and all four `w264` arms say `EXEC-WITNESS DISARMED` in their own logs.
> > ⇒ The ninth consecutive lane whose brief's premise was already answered — and the sharpest
> > instance yet: `execution_plane_increments.md` §16.98.1 diagnosed **this exact class about
> > this exact flag two rungs earlier** (*"a correct default is not a handoff … must be named in
> > the CONSUMER's preconditions, not only in the producer's rationale"*) — and recorded it in
> > the **producer's** doc, which is the failure mode the sentence describes.
> >
> > **One variable, `off`→`on`, measured:**
> > - the table LEARNED the leaves — `pdb=0x201000` rows **5 → 13 348**, `wit` **0 → 37**,
> >   `wit_sample` `[]` → **`[0x201000,0x202000,0x203000,0x204000]`** = *exactly* the four
> >   page-table pages the descent calls `byEXEC#104…#107`;
> > - `PB-PIN … MISS` **8 → 0**, resolved-in-guest-RAM **0 → 8**, **`PINNED` 0 → 8** — the
> >   guest-RAM pin has placed bytes on a live guest **for the first time**;
> > - `NOT-IN-GUEST-RAM = 0` ⇒ the **`miss = fault` invariant HELD**; the fix adds a *writer* to
> >   the witness, never a lookup path, so residue still cannot bind.
> >
> > ★★★★★ **AND THE EIGHT `Xid` AT THE EIGHT PUSHBUFFER VAs ARE GONE:**
> > ```
> > off: ENGINE CE3_PBDMA0 HUBCLIENT_ESC @ 0x2_02c00000 (8 distinct VAs) ACCESS_TYPE_VIRT_READ
> > on:  ENGINE CE3        HUBCLIENT_CE1 @ 0x2_0440f000 (ONE address)    ACCESS_TYPE_VIRT_WRITE
> > ```
> > Front-end → engine; method-fetch client → **data** client; the pinned pages → a new page;
> > **READ → WRITE**. ⇒ **The PBDMA fetched the pushbuffer, parsed its methods, and the copy
> > engine began EXECUTING them.**
> > ★★★ **`0x2_0440f000` is the COMPLETION SEMAPHORE PAGE** — eight channels'
> > `SET_REPORT_SEMAPHORE` targets, `0x20440ff80 … 0x20440fff0` at 16-byte stride, all
> > `site=GuestRam`. **Leg 5 has arrived as a hardware fault at a named address, and the fix is
> > isomorphic to the one just landed, on ONE page.**
> > ⚠ `CUP2_RC = 124` on both arms, **pre-registered at zero movement** (fifth consecutive lane
> > to predict zero and measure zero — still right: no table fix retires a semaphore nothing
> > submits). `CE-SUBMIT → RETIRED` still `0`.
> > ⊘ **Costs, unpaid:** `unwitnessed` rose **6275 → 19 874** beside `bound` **6275 → 19 615**
> > (the gate opened *partway*); **255 `StraddlesLiveBinding` refusals** (0 on `off`).
> > ⊘ **Not attributable to the PIN** — the arm changed 13 343 bindings, so the `Xid` move
> > belongs to the **arm**.
> > ⊘⊘ **AND THE INSTRUMENT LESSON, which cost the most:** `grep -c Xid` read **8 on both
> > arms**. ★★★ **A COUNT CANNOT SEE A SUBSTITUTION** — five facts changed and a magnitude saw
> > none of them. When a fix is expected to **move** a wall rather than remove it, the identity
> > is the instrument. `w265_grade.sh` now carries `Xid` ENGINE/CLIENT/DISTINCT-ADDRS/ACCESS-TYPE
> > as scorecard rows.
> >
> > ### ★★★★★ MEASURED, `w264` (4 arms, real GA106, rev `a4c46bb`) — **AND LEG 4 IS NEITHER**
> > `traces/boots/w264/RESULT.md`. The pin was built, armed, and asked about **exactly the
> > eight addresses hardware faults on**. The **address table answered `Miss` on all eight**,
> > while the **descent on the same log line resolves each to guest RAM** (`pb=S:0x41539000
> > …`), and `NOT-IN-GUEST-RAM = 0` on every row.
> > ⇒ ★ **The two resolvers disagree about EXISTENCE, not about aperture.** Leg 4 is not *"join
> > the pages"* and not *"pin the pages"* — it is **the address table's POPULATE side never
> > learning the pushbuffer leaves**. The consumer is built and correct; the authority was
> > never told. ⊘ `miss = fault` means it cannot be papered over at the consumer.
> > **The next rung's address list:** `pdb 0x201000`, VAs `0x202400000 0x202600000 0x202800000
> > 0x202a00000 0x202c00000 0x202e00000 0x203000000 0x203200000`. Open question to split
> > first: *never learned* vs *learned and pruned before we asked* — a `Miss` does not
> > separate them; start at `PT-DECODE`'s `bound=6275 unwitnessed=6275`.
> > ⚠ `CUP2_RC = 124` on all four arms, **pre-registered at zero movement**, and `CE-SUBMIT →
> > RETIRED` still `0` — leg 5 is unbuilt, so this rung could not have moved it.
> > ★★ Second result: the four-arm ladder (one variable per step) **discharges `w263`'s own
> > qualification** — `PushbufferAperture 0→9` belongs to legs **A2+B** (`join`→`ring`), not to
> > the pushbuffer plane, and the FB join **alone** (`base`→`join`) moves no doorbell-level
> > number at all. ⊘ Still unseparated: **leg B vs leg A2** — `ring` and `pin` both carry B, so
> > this campaign cannot attribute the fetch, and doing so needs a boolean on
> > `plan_engine_object`'s public signature (leg B's arming is inherited by construction).
>
> **5 — the COMPLETION PATH.** `CE-SUBMIT → RETIRED` was **0 on both `w262` arms**, as it has been
> in ~127 logs.
>
> > ### ⊘⊘⊘ CORRECTION, 2026-08-12 (**NEWEST — read this one first**) — **LEG 5 IS CLOSED.
> > THE GR ENGINE FETCHED, THE GR WORK RAN, AND ALL EIGHT COMPLETIONS LANDED. `CUP2_RC` DID
> > NOT MOVE.** Everything below this block about *"the completion is never written"* is
> > SUPERSEDED. `../../../nvkvm-rs/traces/boots/w268/RESULT.md` (rev `70463ae`, 2 arms, real
> > GA106, branch `w268-the-cursor-and-the-arm`; pre-registration
> > `docs/design/w268_the_cursor_and_the_arm_prereg.md` at `87c0d0f`, before the instruments
> > existed).
> >
> > ★★★★★ **THE OWNER'S `GP_GET` QUESTION, ANSWERED BOTH WAYS BY ONE VARIABLE**
> > (`KAYFABE_GR_ROUTE`, `refuse` → `passthrough`; ⊘ the arm is **not** defaulted and stays
> > `refuse`):
> > - `refuse` (the shipping configuration): all eight `GrCompute` channels read
> >   **`GET=0 PUT=1`** and `GET` **never** becomes non-zero — over **167 seconds**
> >   (`t=+71006ms` → `t=+238095ms`, when the guest tears down and `PUT` returns to 0).
> >   ⇒ **The guest submitted and the host engine NEVER FETCHED.**
> >   ★★ The reading carries its own **known-positive**: the same reader, same arm, reads
> >   `GET=1 PUT=1` on a **copy-engine** channel. The zero is about those channels, not the
> >   instrument.
> > - `passthrough`: **`GET` caught `PUT` on all eight, 32 ms after the doorbell.**
> >
> > ★★★★★ **AND THE THING `cuCtxCreate` POLLS WAS WRITTEN — the first time in this campaign.**
> > All eight GR `SET_REPORT_SEMAPHORE` slots (`+0xf80…+0xff0`) carry `payload=1` and a
> > **distinct GPU timestamp**; `COMPLETION-WATCH → OBSERVED` **= 8** against the control's
> > `NOT-OBSERVED = 8`; the page fills in real time `24/1024 → 48/1024` over 430 ms.
> >
> > ⊘⊘⊘ **AND `CUP2_RC = 124` ON BOTH ARMS**, pre-registered at **zero** movement (eighth
> > consecutive), with `cup2`'s own output **byte-identical** between arms. ★★★ **This is the
> > most informative negative the campaign has produced, because it is the first one taken
> > BEHIND a satisfied completion.** *"The guest is waiting for a semaphore nobody writes"* is
> > **RETIRED as an explanation** — eight were written, at the guest's own declared addresses,
> > with the guest's own payloads, and it did not proceed. **What it waits on now is a NEW and
> > unmeasured question.**
> >
> > ★★★ **WHY THIS WAS REACHABLE AT ALL, and it is a doc-hygiene finding, not a code one:**
> > `gr_doorbell_passthrough.md` §0.3 kept the route disarmed on two reasons — *"the ring is
> > OURS"*, *"the cursor is OURS"* — measured **2026-08-11** and **both refuted by `w267`'s own
> > log** (all 16 `GR-BIRTH iso2` lines read `adopt=GUEST-RING userd=GUEST-USERD`, eight of
> > them `GrCompute`; legs A2 and B landed at `w261`/`w262`). Impeccably sourced, out of date,
> > duplicated as a comment in **two** further places. ⇒ *A ruling's DATE is part of the
> > citation.* All three copies corrected in place at `65fe5ca`.
> >
> > ⊘ Two further code readings that retire standing puzzles:
> > **(a)** the three pin passes sit **below** `try_ce_submission`, which terminates a GR
> > doorbell by `RefuseByRoute` — so the GR pushbuffer pages were never pinned, and **arming
> > the route IS giving the pins their source** (`PB-PIN`/`SEMA-PIN` distinct tokens **8→16**);
> > **(b)** `DOORBELL-REFUSED [PushbufferAperture]` is a **POST-HOC** refusal —
> > `rm.schedule` + `rm.ring_doorbell` already ran inside `verb_op`, **before** `forward_ring`.
> > That is why the hardware executed at `w266` while every doorbell read as refused.
> >
> > ★★★ **THE NEW WALL, and it is one address**: the `pass` arm's single `Xid 31` is
> > `ENGINE CE2 HUBCLIENT_CE0 … VIRT_WRITE @ 0x2_04420000` — a page that appears **nowhere else
> > in the boot**, arriving on the guest's **first substantive copy-engine work**
> > (`methods=11 launches=3` over two GPFIFO entries) once GR context init completed.
> > ⚠ And a prediction that FAILED: I expected the fetching GR engine to fault **reading** its
> > own unjoined pushbuffer leaf. It did not, and that is unexplained.
> > ⚠ New measured instrument limit: the observer thread's `stderr` interleaves with QEMU's own
> > timestamped writer and splices whole log rows — 6 of 8 `why=first` rows on one arm.
>
> > ### ⊘⊘⊘ CORRECTION, 2026-08-12 — **THE PAGE WAS READ. THE COPY ENGINE WROTE IT,
> > AND IT IS THE WRONG ENGINE.** Read this before the `w266` block below: that block's *"top
> > limit"* has been discharged, and its two worlds are no longer open.
> > `../../../nvkvm-rs/traces/boots/w267/RESULT.md` (rev `b129770`, 2 arms, real GA106, branch
> > `w267-read-the-page`; pre-registration `docs/design/w267_read_the_page_prereg.md` at
> > `efbcaba`, before either instrument existed).
> >
> > ★★★★★ **WORLD (a) IS MEASURED, WORLD (b) IS REFUTED.** `[measured, `on` arm, t=+71169ms]`
> > `nonzero=12/1024` — **four complete `RELEASE_FOUR_WORD_SEMAPHORE` reports**
> > `[payload=1, 0, ts_lo, ts_hi]` at `+0xf40 +0xf50 +0xf60 +0xf70`. ⊘ The `off` arm reads
> > `nonzero=0/1024` on **all twenty dumps over 174 s**. ★ **The timestamps are the proof of
> > authorship** (`0x18cb063c_…`, distinct per channel): nothing in this VMM has that clock, so
> > this is not *"the value we expected appeared"* — it is **a value we could not have
> > fabricated**.
> >
> > ★★★★★ **AND THE `on` ARM ALONE IS A PER-CHANNEL CONTROL.** An ordering race — exactly the one
> > `w266` §3.2 said *"did not materialise, **not** that it cannot"* — split the eight CE
> > channels 4/4, and **every** fact partitions on that line with no exceptions:
> > **4 pinned** (`SOURCE 8 declared`, targets `…ff70 …ff40`) → payload + timestamp, **no `Xid`**;
> > **4 unpinned** (`NO PAGE TO PIN`, targets `…ff30 …ff00`) → slot **zero**, `Xid 31 VIRT_WRITE`.
> > Same boot, same page, same engine class. ⇒ The prediction failing is what bought the evidence.
> >
> > ⊘⊘⊘ **THE PAGE HOLDS SIXTEEN SLOTS AND THEY BELONG TO TWO ENGINES.** This is the correction
> > that matters for planning:
> > ```
> > +0xf00 … +0xf70   eight CE channels' NVC7B5 SET_SEMAPHORE      ← WRITTEN
> > +0xf80 … +0xff0   eight GR channels' NVC7C0 SET_REPORT_SEMAPHORE ← ZERO on EVERY dump, BOTH arms
> > ```
> > The CE pushbuffer is 32 bytes and decodes (`ogkm-580: clc7b5.h:84-105`) as `LAUNCH_DMA 0x14` =
> > `DATA_TRANSFER_TYPE = NONE` + `RELEASE_FOUR_WORD_SEMAPHORE` — **a pure semaphore release that
> > moves no data**. ⇒ **The engine that stopped faulting is not the engine `cuCtxCreate` waits
> > on.** Leg 5's supply side is real and it was never on the path to `CUP2_RC`.
> > ⇒ ⊘ **Do NOT widen the completion watch to the CE slots.** An `OBSERVED` row there would mean
> > nothing and read as everything.
> >
> > ⊘ **And the brief's follow-through was already built**: *"derive the right offset from the
> > guest's `SET_REPORT_SEMAPHORE`"* is what `decode_report_semaphore` has done since `w226` —
> > eight declares at eight distinct VAs, eight verdicts, nothing hardcoded. The address the rung
> > needed was **already read and truncated at the print** (`push_headers` emitted `words[i+1]`
> > and stopped, so a three-argument run rendered as `=0x2`). ★ **For a multi-word operand a
> > one-argument dump is not a smaller dump, it is a wrong one** — the half it keeps carries the
> > least information.
> >
> > ⚠ **Deviations, and they are the next rung**: the pin is triggered by a **CE doorbell** but
> > sourced from a **GR declaration**, and nothing orders those (`NO PAGE TO PIN = 4`);
> > `SemaPageReader::close` never runs because QEMU dies without `detach_ram`, so there is **no
> > teardown dump** and the harness's own assertion caught it; first-pass `PT-DECODE bound` moved
> > `19615 → 19618` identically on both arms — **bounded, not explained**, so `w266`↔`w267` is not
> > byte-identical. `CUP2_RC = 124` both arms: **seventh** consecutive predicted zero, seventh
> > measured zero, with the size pre-registered as **zero** rather than *"small"*.
>
> > ### ⊘⊘⊘ CORRECTION, 2026-08-12 (LATER) — **THE PAGE IS PINNED AND THE HOST GPU STOPPED
> > FAULTING: 8 `Xid` → 0. THE WALL IS NO LONGER A FAULT — AND IT IS STILL A WALL.**
> > Read this before the block below, which says leg 5 is *unbuilt*. It is built and it has run:
> > `../../../nvkvm-rs/traces/boots/w266/RESULT.md` (rev `f09aba2`, 2 arms, real GA106,
> > branch `leg-5-completion-pin`).
> >
> > ★★★★★ **One variable — `KAYFABE_GUEST_SEMA` `off`→`pin` — one page, and three campaigns of
> > MMU faults end.** `w263`/`w264`/`w265_off` faulted **reading** the pushbuffer; `w265_on`
> > faulted **writing** the completion; `w266_on` **does not fault at all**.
> > ```
> > off: 8 × Xid 31  ENGINE CE3/CE2 HUBCLIENT_CE1/CE0 @ 0x2_0440f000  VIRT_WRITE
> > on:  (nothing)
> > ```
> > ⊘ **And the zero is MEASURED, not an empty artefact** — checked first, because a zero-byte
> > `hostdmesg` is exactly the shape that reads as benign. The watermark advanced **961 → 969**
> > (the `off` arm's own 8), the `host dmesg delta:` line is printed **only** by the branch that
> > successfully read `dmesg`, and the probe log carries `HOST_DMESG_XID=0` beside `off`'s `=8`.
> >
> > ★ Again **no new mechanism**: `pin_guest_ram` — the chain that placed eight pushbuffer runs
> > at `w265` — pointed at one page. What did not exist was a **reader** for the addresses
> > (`WatchList` had `declare`/`stats`/`live`/`sweep` and no accessor) and a **consumer**
> > (`back_census_framebuffer_leaves` handles `Site::Framebuffer` only and calls the `GuestRam`
> > rows *"this pass's standing negative controls"*). Eight declared targets → **one** page.
> >
> > ⊘⊘ **AND THE COMPLETION STILL DOES NOT LAND.** `COMPLETION-WATCH … NOT-OBSERVED samples=88
> > last_seen=0x00000000`, byte-identical on both arms; `CUP2_RC = 124` on both — the **sixth**
> > consecutive predicted zero and the sixth measured zero. The observer *is* watching the
> > pinned page (declares resolve `gpa 0x2197eff0`; the pin placed `gpa 0x2197e000 len=4096`).
> > ⇒ ★★★ **The page became writable and nothing observable was written to it — 0 faults AND 0
> > completions.** *"Writable"* and *"the guest's wait was satisfied"* are different facts, and
> > this is the campaign's cleanest demonstration of it.
> >
> > ⊘ **THE TOP LIMIT, and it is the next rung: nothing reads the page.** *"No fault"* is
> > consistent with (a) the write landing at a slot nobody watches — the **CE**'s own
> > `SET_SEMAPHORE` target is a different address in the same page than the **GR** channels'
> > `SET_REPORT_SEMAPHORE` targets the observer watches — and (b) the engine never attempting
> > it. The evidence leans hard on (a); leaning is not measuring. ⇒ **Dump the 4 KiB page**, and
> > decode the CE's full `SET_SEMAPHORE_A/B/PAYLOAD` operand (the pushbuffer dump prints only
> > the first of three arguments).
> >
> > ⊘⊘ **INSTRUMENT LESSON, and it cost the most here: ADDING A PRODUCER SILENTLY RE-SCOPES
> > EVERY CONSUMER THAT WAS IMPLICITLY SCOPED BY BEING THE ONLY ONE.** Three grader rows read
> > `16/16/20` against the control's `8/8/12` — *"leg 4 doubled"* — when leg 4 had not moved;
> > they were counting leg 5's identically-shaped lines. Fixing it exposed that one of them was
> > **already wrong before leg 5 existed** (its true value is 8, not 12; four came from
> > `GR-RING-JOIN`/`GR-FB-JOIN`, a different plane), so at `w265` it was summing **three**
> > producers and only a hand-done subtraction made it read right. ★ **A row that needs a
> > subtraction to be read is a row that will be misread.** ⚠ And the obvious repair `TABLE:`
> > does *not* work — `SEMA-TABLE:` contains it; the correct anchor is `[^-]TABLE:`.
> >
> > ⊘ **`CE-SUBMIT → RETIRED` is still `0`** and this rung does not submit. The `Xid` going to
> > zero is a **supply-side** result.
> >
> > ### ⊘⊘ CORRECTION, 2026-08-12 — **LEG 5 NOW HAS AN ADDRESS, AND HARDWARE NAMED IT.**
> > `w265`'s `on` arm faults `ENGINE CE3 HUBCLIENT_CE1 @ 0x2_0440f000 ACCESS_TYPE_VIRT_WRITE`,
> > ×8. That page holds **eight channels' `SET_REPORT_SEMAPHORE` targets**
> > (`0x20440ff80 … 0x20440fff0`, 16-byte stride, all `site=GuestRam`). ⇒ Leg 5 is still
> > **unbuilt**, but it is no longer *unlocated*: the copy engine is now **trying to complete**
> > and faulting on the write. ★ The primitive it needs is `pin_guest_ram` — the one that just
> > placed 8 pushbuffer runs — pointed at **one** page. ⊘ `CE-SUBMIT → RETIRED` is still `0`;
> > nothing here submits, and an attempted semaphore write is not a retirement.
>
> ⇒ **`w263` is therefore pre-registered at ZERO movement, and that is a prediction against the
> rung's own optimism** — necessary-not-sufficient, exactly the shape `w260` measured for the
> supply side. ⚠ **A zero here still does not indict the passthrough model**; it indicts my
> arithmetic about how many legs there were.

**Why nothing has moved yet, and why that is expected**: a channel born `RingSource::Ours(None)`
has its `gpFifoOffset` on **our** ring and its `GP_PUT` in **our** USERD, and `submit_entry`
refuses a handed-in ring **by name**. ⇒ `GP_PUT == GP_GET` forever until **all three** legs are
present. Every zero-movement result so far was **pre-registered as zero**.

⚠ **The standing debt is NOT discharged and must not be read as discharged.** A one-legged stool
falling over is not evidence against stools. ★ **The leg-B boot is the first with all three legs,
and therefore the first where a `CUP2_RC` change is genuinely possible.**

---

## 2. WHAT LANDED 2026-08-11 → 08-12

- **Leg C** (`b734995`) — the doorbell route. ⊘ There was **no server to build**:
  `SharedDevice::doorbell` was never copy-engine-specific; the missing wiring was **one arm in one
  `if`**. Token translation is a **plain field read**, not a map lookup.
- **Leg A1** — the FB join gets a **second source**, the channel's own ring, **and it runs before
  the birth**. The join's only prior driver is the **operand** census, and *a ring is not an operand
  of the methods it carries*, so the ring was never presented.
- **Leg A2** — the production birth path names the guest's ring; **crosses the IPC wire** (the
  adapter runs in the child, so a trait-only change would be dead on the one path a boot exercises).
  **Arming is inherited** from the supply side, so a disarmed build is `None` by construction.
- **`w261`** — 24 × `GR-RING-JOIN` at `fb_phys=0x1000000`, including the walling channel.
- **`w262`** — ★ `GR-BIRTH … adopt=GUEST-RING`, **16 armed / 0 disarmed**, on client `0xc1d0000c`.
  The disarmed zero is **measured**: 24 `adopt=DECLINED` lines prove the path was consulted.
- **Display scoped** (`docs/design/display_plane_scoping.md`) — ★ `NVA083_GRID_DISPLAYLESS` is a
  **first-class NVKMS HAL**, 885 lines, `.coreChannelDma = { }`, **zero EVO channels**.
  ⇒ Emulating the display engine is **dominated**: 1–3 days vs 3–6 months.
- **`CLAUDE.md` FIFTH LIMIT corrected** — the oracle table is **11 empty / 16 truncated / 29
  complete**, not "11 empty, 45 good". The trustworthy predicate is `dlen >= psize`.

---

## 3. ★★★ WHAT LEG B ACTUALLY IS — corrected twice, both times by a doc that already existed

⊘ **It is NOT a missing `hUserdMemory` arm, and it is NOT `mem_phys`.** Adjudicated in
`docs/design/userd_is_not_the_ring.md` (STATUS LIVE 2026-08-11) — **a day before two lanes
re-derived the same decision from a false premise.**

- RM **permits** guest-supplied USERD unambiguously: `OsDescMemory` is **explicitly special-cased**
  so client pages can back USERD. No aperture gate, no RM-allocation requirement. Hard constraints
  are the whole list: **512 B, 512 B-aligned, non-VPR, phys < 2^40**.
- ★ The GSP path **cuts for passthrough**: CPU-RM resolves handle → memdesc → physaddr **locally**
  and ships GSP a physical descriptor.
- ⊘ `mem_phys`-has-no-producer is **CONFIRMED but IRRELEVANT** — RM looks `hUserdMemory[0]` up in
  **the caller's own client**, so forwarding the guest's handle was never the mechanism.
- ★★★ The guest's USERD is **already decoded** (`ChannelUserdWire`, version-keyed, live).
  `kayfabe-rmrpc/src/lib.rs:1379` names its own gap: *"⊘ Read by no decision."*

⇒ **Leg B is a MISSING CONSUMER, in the same shape as leg A1**: join the USERD's leaf, hand RM an
`OS_DESCRIPTOR` over it **at creation**. ⚠ **Never at first doorbell — RM zeroes a caller-supplied
USERD**, so late adoption wipes the cursor that caused the doorbell.

---

## 4. ⊘ WHAT IS STILL UNMEASURED

1. **That the adopted ring is FETCHED.** Nothing reads `GP_GET`. *"Adopted"* ≠ *"fetched"*.
2. **The GR channel's own cursor vs its own birth** — nothing joins a BAR1 offset to a channel.
3. **Leg B's ordering claim** — an observation cannot separate *right* from *wrong and lucky*;
   needs fault injection. The favourable reading is an **indication, not a proof** (44 advances
   dropped, and the predicate had a false positive).
4. **`NOT-ASKED = 0`** on both arms has **no live known-positive** — it rests on a unit test.
5. The three `w260` unknowns (install→bind ordering, the release path, attempt-once) — **still
   need fault injection**, still unmeasurable by a green boot.

---

## 5. ⚠ TRAPS ADDED SINCE THE 08-11 DOC — all measured, several hit after being warned

- ★★★★★ **A recorder that buffers and dumps at teardown reports ORDER correctly and TIME not at
  all.** `nvkvm_bar1_record` stores 16 entries, prints once at teardown ⇒ **every** log says
  *"GP_PUT came 178 s after the alloc"* — a constant dressed as a measurement, and it is the
  **favourable** answer. ⇒ **Ask when the LINE was emitted, not when the event happened.**
  ⚠ A broken instrument that always agrees with you is worse than one that always disagrees.
- ★★★ **A gate's PROSE is not its ASSERTION.** `guest_ring_census.rs:168` reads *"exactly one
  caller"* and asserts a **definition** count; it stayed green through the rung that gave the verb
  its first production caller. A pre-registration **promised** to fix it and did not.
- ★★ **A `#[cfg(not(feature = ...))]` sibling with an empty body is a silent no-op.** Assert the
  feature from a positive signal the armed path emits.
- ★★ **A mutation that does not apply is a green indistinguishable from a test that catches
  nothing.** Assert `count == 1` before writing.
- ★★ **`git worktree list`, not mtime, decides whether a tree is live.** A depth-limited `find
  -newermt` scan reported the **active** 18 GB tree as idle.
- ★★ **`git push origin <branch>` reports `Everything up-to-date` truthfully about a branch you are
  not on.** Check `git branch --show-current` first and `git rev-parse HEAD origin/<branch>` after.

---

## 6. NEXT RUNGS, ORDERED

0. ⊘⊘ **DONE at `w267` — DO NOT RE-RUN THIS.** ~~READ THE SEMAPHORE PAGE~~ The dump exists and
   the answer is **(a)**: the copy engine wrote four `RELEASE_FOUR_WORD_SEMAPHORE` reports at
   `+0xf40 … +0xf70` with hardware timestamps, while the eight **GR** slots at `+0xf80 … +0xff0`
   stayed zero on every dump on both arms. See the LATEST correction in §1 and
   `../../../nvkvm-rs/traces/boots/w267/RESULT.md`.
   ⇒ ★ **The successor is ORDERING, not widening.** The pin is triggered by a **CE doorbell** and
   sourced from a **GR declaration**, and nothing orders those, so 4 of 8 channels raced ahead of
   their own pin and faulted (`NO PAGE TO PIN = 4`). ⚠ The fix must derive the page from
   something the **guest** wrote — pinning a remembered `0x2_0440f000` is the `cap2b` class.
   ⊘ **Do NOT widen the watch to the CE slots**: a different engine's semaphores, which the guest
   does not wait on. An `OBSERVED` row there would mean nothing and read as everything.
1. ◐ **Leg B** — in flight. The first three-legged boot.
2. **Read `GP_GET`** — the difference between *adopted* and *fetched*. Nothing does today.
3. **Fault injection** for §4's items 3 and 5 — a green boot cannot reach them **by construction**.
4. **Merge to `master`.** ⚠ Deliberately held: `master` is still `e758778`. Merging an
   **unwitnessed** capability is the mechanism that produced the eight built-and-unwired
   capabilities already on the orphan list. Legs A1/A2 are now witnessed and eligible.
5. **Display** — decide posture A (displayless, 1–3 days) vs B (virtual display). ★ The deciding
   experiment is *"does Vulkan/EGL survive `NVKMS_ALLOC_DEVICE → NO_HARDWARE_AVAILABLE`?"* — one
   boot, two strong sources disagree.
6. ⚠ **Confirm the app count with the owner**: both `realapp_matrix.md` and the owner's own
   `PRODUCT_POSITIONING.md` say **22**, and **no 30-app list exists in either tree**.

---

## 7. HONEST LEDGER

**Every lane dispatched in this window refuted something in its brief, and in each case the
refutation was worth more than the work it replaced.** The pattern is unchanged and one-directional:
**I assume things are missing when they are already built, or already answered.** Six instances of
re-deriving an answered question — the worst being a lane that reached a **correct conclusion from a
false premise**, which reads as corroboration rather than duplication.

⇒ **Weight my estimates of remaining work as upper bounds**, and before dispatching anything run
`git log --all --oneline --grep=<topic>` in **both** trees and read the doc's dated STATUS block.

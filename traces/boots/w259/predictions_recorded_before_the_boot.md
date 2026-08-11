# w259 — the GR passthrough route: REFUTATIONS FIRST, then the predictions

> ### STATUS — 2026-08-11 / **LIVE — PRE-REGISTRATION. Nothing here was booted. NO CODE WAS
> WRITTEN THIS RUNG.**
> Branch `hostgr-route-over-guest-ring`, off `origin/master` `d55187a`.
> Every claim below is labelled **measured** / **inferred** / **assumed**. Every "measured"
> row names the committed artefact it was read out of, and the revision that produced it.
>
> ⊘ **This file exists because the brief asked for the predictions to be committed BEFORE any
> boot. It is committed before the build as well, because three of the brief's premises turned
> out to be false and the corrections are worth more than the build would have been.**

---

## 0. ★★★★★ LEAD WITH THE REFUTATIONS — three, and the second one inverts the rung

### 0.1 ⊘⊘ REFUTED — *"no host GR channel exists today because the doorbell is refused before materialization"*

The brief's item 2 offers this as its **inferred** reading and asks for it to be tested. It is
**false**, and the host GR channels are not merely possible — they exist, eight per boot, in the
most recent committed evidence.

**[measured 2026-08-11, `traces/guest_boots/run_w256_ce36a5b_cel_unbounded_qemu.log`, rev
`ce36a5b`, `isolate_plane=real`]**

```
kayfabe: ENGINE-OBJECT class=0xc7c0 client=0xc1d0000c parent=0x5c000019 params=16B
   → FORWARDED engine=GrCompute host_object=0xcafe000a materialized_channel=true reused=false
```

— eight such lines (`0x5c000019/1f/23/27/2b/…`), each `materialized_channel=true`. The same
boot's promote-ctx census closes it from the other side:

```
{8x pdb=Y own=not-declared cs=ok(h0x5c000007=>c0xc1d0000c/0x5c000007) …
    p2/c0:vc7 GrCompute c0xc1d0000c/0x5c000019 …}
```

**Why the brief's reading was wrong, structurally:** the host channel is materialized on the
**engine-object alloc** path, not on the doorbell path. `plan_engine_object`
(`crates/kayfabe-fwd/src/lib.rs:3153`) emits `VerbPlan::EngineObject { channel: None, … }` when
`chan.host_channel` is `None`, and the isolate's arm for that verb
(`crates/kayfabe-isolate/src/lib.rs:2421-2455`) calls `rm.alloc_channel(vas, *engine, …)` before
allocating the object. `engine_type_for(GrCompute) = ENGINE_TYPE_GRAPHICS`
(`crates/kayfabe-isolate-host/src/rm.rs:1751-1766`). The doorbell refusal at `shim.rs:4527` is
**downstream in time** of that, not upstream. `gr_execution_boundary.md:44-52` states the same
fact in words: *"we already **allocate** host GR channels … We can already schedule a TSG. ⇒
**Scheduling is not what is missing.**"*

### 0.2 ★★★★★ ⇒ THE TOKEN QUESTION IS SETTLED, AND THE ANSWER IS THE BRIEF'S — for a different reason

Because the host GR channel exists, so does its host work-submit token: `alloc_channel_in`
issues `NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN` on every channel it builds
(`rm.rs:3915-3924`) and the pair is adopted into core as `Channel::host_channel` /
`Channel::host_token` (`crates/kayfabe-fwd/src/lib.rs:3291-3297`).

**Translation is already a map lookup, and it is already performed — on the CE path, in
production, today.** `VerbPlan::Doorbell`'s isolate arm calls `rm.ring_doorbell(chan.1)`
(`crates/kayfabe-isolate/src/lib.rs:2412`), where `chan: ChannelHandles = (HostHandle, u64)` and
`.1` is the **host** token; `HostRmBackend::ring_doorbell` (`rm.rs:3267`) narrows it to `u32` and
writes it to the usermode window. The guest's token never reaches hardware.

**The measurement the brief half-remembered has been found, read, and it does NOT say what the
brief feared.** It is not in this repo — it is in the C artifact this tree cites by the `C:`
convention (`kayfabe-abi/src/submit.rs:1064` and `rm.rs:3067` both point at it):
`C: /workspace/nvidia-gpu-passthrough/docs/design/mode2_doorbell_chid.md:381-399`,
`[measured 2026-06-05, GA106, cup2 m2exec]`:

```
Guest doorbell tokens written during cuCtxCreate: 0x4, 0x10008, 0x10001.
- 0x4      == host chan token (gpfifo 0x121010000) ✅
- 0x10008  == host chan token (gpfifo 0x1210d0000) ✅
- 0x10001  — NO host token equals it ❌

DECISION: doorbell pass-through is INCORRECT. … So we must NOT forward the guest's token
verbatim. Use GP_PUT-driven demux … ring ITS host_token (from the M5.12 table).
```

⇒ ★★★★★ **The refuted thing is forwarding the guest's token VERBATIM. The prescribed fix is
trap-and-translate against a per-channel host-token table — which is exactly what kayfabe already
does** (`ring_doorbell(chan.1)`, above). `C: mode2_forwarding_model.md:118-122` says so in as many
words: *"Verbatim ring is wrong … so the trap+translate is **mandatory**."*
⇒ **This rung is not the refuted design; it is the prescribed one.** No guest-driver patch is
implicated: the patch-requiring escape is the *other* one (§6/§12's legacy host-allocates-chid,
`mode2_doorbell_chid.md:236-238`, explicitly rejected for *"breaks the stock-driver thesis"*).
⚠ Labelled: the C measurement is **measured**; that kayfabe's live path is the prescribed shape is
**inferred from code**, at high confidence.

**★★ And the same table corroborates item 9 of §2 from a completely independent implementation.**
`mode2_doorbell_chid.md:388` ends its host-token table with **`GR 0x200200000 -> 0x0c`** — the
C's host **GR** channel, gpfifo at guest VA **`0x200200000`**. That is byte-for-byte the address
kayfabe reads for its own GR ring two months later on the same chip
(`run_w256_ce36a5b_cel_unbounded_qemu.log`: `RING-ROSTER key=0xc1d0000c:0x5c000019
ring=0x200200000 entries=1024`). ⇒ Two independent ports, two months apart, one address — and the C
built a **host GR channel over it**. ⊘ It still does not say which *plane* backs it, so item 9
stays open; but it removes any doubt that `0x200200000` is the GR GPFIFO.

**Corroborating shape, measured, w256:** the guest's GR tokens are `0x00000007 … 0x0000000e`
(runlist 0, chid 7..14 under the `NV_CTRL_VF_DOORBELL` encoding), while its CE tokens are
`0x00010002`, `0x00010003`, `0x00020005`, `0x00020006` — different runlists. Nothing in the host
token space needs to agree with either.

⊘ **One thing the C's alternative fix tells us for free:** GP_PUT-driven demux requires scanning
*every* forwarded channel's USERD `GP_PUT` — the same word G8 (§2 item 7) must write. The read and
the write are the same field, so building G8 also builds the C's fallback demux, without parsing
a ring.

### 0.3 ⊘⊘⊘ REFUTED — *"#233 says the guest ring cannot be exported on the bench's QEMU backend today"*

`docs/design/guest_ram_crossing.md` §1 is titled **"★★★ MEASURED — guest RAM crosses today with
NO new QEMU surface"** `[measured 2026-08-10, vh2, rev 954f926, transcript
docs/reference/bench_evidence/guest_ram_memfd_954f926.out]`. The mechanism is
`-object memory-backend-memfd,share=on` plus `/proc/self/fd` enumeration inside the shim
(`NVKVM_RAM_BACKEND=memfd`, then `KAYFABE_GUEST_RAM=memfd`). What #233's sketch got wrong is
recorded in the same file's §0 (R1–R5): guest RAM can never be a *window*, and the two VMM
backends differ in ownership, not shape.

⇒ **The export is not the blocker.** ⚠ What #233 *does* still constrain is arming: `w230c` is a
committed control in which `NVKVM_RAM_BACKEND=memfd` was set and `KAYFABE_GUEST_RAM=memfd` was
not, and **not one line of the crossing ran** while the boot looked completely healthy
(`guest_ring_adoption.md` §1b).

---

## 1. ⊘⊘⊘ AND THE RUNG WAS NOT BUILT — the reason, stated so it can be overruled

### 1.1 A LIVE doc in this tree already answers the brief's exact question, and the answer is NO

`docs/design/gr_execution_boundary.md:1-23` — **STATUS 2026-08-11, LIVE**:

> **Question, from the brief:** open `shim.rs`'s `Route::NotACopyEngineChannel` refusal so the
> real host GR engine runs `cuCtxCreate`'s pushbuffer and writes `0x2_0440fff0` itself.
> **Answer: NO — and the thing that must exist first has a name, a shape, and now a number.**

Its §4.1 orders the work and says outright *"none of these is 'open the route'"*:

| § 4.1 item | status **today**, measured |
|---|---|
| 1. fault containment (property 3) | ★ **DISCHARGED** — `traces/boots/w248/`, §16.100: bystander ran **2 675 519** verified iterations across an Xid 31 `FAULT_PDE`, 0 errors |
| 2. the FB crossing (property 1's majority — 3 of 5 operands) | ★ **BUILT** — w228 `fb_leaf_crossing.md`, `KAYFABE_FB_BACKING=on`; `traces/boots/w247/` reads `placed_as_asked=true ×24` |
| 3. **property 2 — CLOSURE** | ⊘ **the one that remains** |
| 4. open the route | ⊘ *"Only then"* |

### 1.2 ★★★★★ BUT PROPERTY 2 IS DISSOLVED **BY** THIS RUNG, NOT BEFORE IT — and that inverts §4.1

`gr_execution_boundary.md` §2.5 states the closure objection: `alloc_channel_at` places our own
ring inside the VA space the channel is given, so *"the guest could aim its own
`SET_REPORT_SEMAPHORE` at **our** completion semaphore."*

Two of the three objects it names have **already moved**, measured:

- the isolate's CE ring / pushbuffer / release word live in `ExecutorVas` only, and **R30 arm C
  was re-measured and REFUSED** — `NVRM: Xid 31 … CE0 faulted @ 0x1_20022000, FAULT_PDE
  ACCESS_TYPE_VIRT_READ` (`docs/design/executor_vas_separation.md` §2; re-confirmed at `b39f95f`,
  `guest_ring_adoption.md` §1);
- USERD is in **no GPU address space at all** — it is handed to RM as `hUserdMemory[0]` and only
  ever CPU-mapped (`rm.rs` G4 arm, `alloc_channel_in`).

⇒ **The entire residual of property 2 is one object: the materialized channel's own 64 KiB ring,
`raw_map_dma`'d into the guest's `Vas::host_vas`** — and `RingOwner::HandedIn` allocates and maps
**nothing** of ours into that space. `alloc_channel_over_guest_ring`'s own doc says so
(`rm.rs:3640-3660`).

⇒ ★★ **Promoting `alloc_channel_over_guest_ring` to the doorbell path removes the last non-guest
object from the guest's VA space as a side effect of the execution work.** Property 2 is not a
prerequisite of this rung; it is a **consequence** of it. `gr_execution_boundary.md` §4.1's
ordering is stale in exactly this respect and must be corrected in place.

⚠ **Provenance, stated because it matters:** this synthesis composes with an **UNCOMMITTED**
adjudication sitting in the contested shared checkout —
`/workspace/nvkvm-rs/docs/design/property_2_adjudicated_against_the_kernel_ce_vas_ruling.md`
(untracked, `??`, another lane's read-only work, verified at `b3ecda4`). Its §5 reaches the same
conclusion independently: *"What would actually dissolve the residual … **`w230`'s guest-ring
adoption** … Promoting it to the doorbell path removes the residual as a side effect of the work
the execution plane needs anyway."*
⊘ **A `??` file is about one checkout, not the repository.** That doc must be committed, and
folded into `gr_execution_boundary.md` §4.1 above the ordering it corrects, **before** any code
cites it. That fold is item 0 of the build order in §2.

### 1.3 ⊘ AND THE ROUTE ALONE IS A WASTED BENCH SLOT — arm C by construction

The missing verb is **G8, the cursor bridge**, and `guest_ring_adoption.md:184-186` names it:

> **The cursor bridge (G8).** Nothing writes the guest's `GP_PUT` into the host channel's USERD,
> so a channel built this way is accepted by RM, schedulable, and **fetches nothing**.

⇒ Opening `Route::NotACopyEngineChannel` today rings a host GR channel whose USERD `GP_PUT` is
`0`. Hardware compares `GP_PUT` against `GP_GET`, finds them equal, and fetches nothing —
**correctly and forever**. The falsifier would return **C**, and C would be **uninformative**,
because it was determined before the boot by a line of code rather than by the GPU.
⊘ **A run whose outcome is fixed in advance is not a measurement.** That is why nothing was
built: the deliverable the brief asked for would have consumed a serialized bench slot to
re-observe a `0`.

★ Note this is *not* the same as saying the rung is far away. G8 is small and the pieces are
in place — see §2.

---

## 2. ⇒ THE RUNG, ORDERED — what is BUILT vs what is MISSING

| # | piece | state | where |
|---|---|---|---|
| 0 | commit + fold the property-2 adjudication into `gr_execution_boundary.md` §4.1 | ⊘ **missing** (doc lives untracked in the shared checkout) | `docs/design/gr_execution_boundary.md` |
| 1 | host GR channel with a host token | ★ **BUILT + MEASURED**, 8/boot | `fwd/src/lib.rs:3153`, `isolate/src/lib.rs:2421` |
| 2 | guest→host token translation | ★ **BUILT + LIVE on CE** — `ring_doorbell(chan.1)` | `isolate/src/lib.rs:2412`, `rm.rs:3267` |
| 3 | channel born over a handed-in ring | ★ **BUILT**, measured on hardware (R31 arm A: token `0x4`, ring placed **as asked**, exactly one CPU map) | `rm.rs:3662` `alloc_channel_over_guest_ring` |
| 3b | **a caller for (3) on a guest path** | ⊘ **MISSING** — one caller, the R31 probe | `guest_ring_census.rs:168` asserts the count |
| 4 | RM accepts an unbound `gpFifoOffset` at alloc time ⇒ **no ordering change needed** | ★ **MEASURED**, R31 arm C accepted `0xB_0000_0000` | `guest_ring_adoption.md` §3.3 |
| 5 | guest-RAM export + pin (`OS_DESCRIPTOR` over guest pages, `GuestRamGrant`, `VerbPlan::PinGuestRam`) | ★ **BUILT + MEASURED** (R29: placed at `0x301400000` as asked, reads `0x9a114001`) | `rm.rs:2816-2840`, `isolate/src/lib.rs:2317` |
| 6 | emulated-FB crossing for the 3 FB operands | ★ **BUILT** behind `KAYFABE_FB_BACKING=on` | `fb_leaf_crossing.md` (w228) |
| 7 | **G8 — the cursor bridge** (guest `GP_PUT` → host USERD) | ⊘ **MISSING**, and it is the load-bearing gap | new verb; USERD is CPU-mapped on **both** ring provenances |
| 8 | the `HostGr` route itself | ⊘ **MISSING** — `DoorbellRoute::HostGr` has **2 mentions, 0 consumers** | `rt/src/device.rs:3817`, `:3853`; refusal at `shim.rs:4527` |
| 9 | ★ the ring's own backing must be host-addressable at the guest's VA | ⚠ **UNDETERMINED** — the GR ring is at guest VA `0x200200000` (`RING-ROSTER key=0xc1d0000c:0x5c000019 ring=0x200200000 entries=1024`) and **no committed line records which plane it lands in**. The semaphore is `GuestRam{gpa:0x2e92fff0}` and 3 of 5 operands are `Framebuffer` — the ring is *neither of those five operands* | must be measured before (3b) can be written |

★ **(9) is the one genuinely open engineering question and it is cheap to close**: it decides
whether the ring is pinned with (5) or with (6), i.e. which of two already-built primitives (3b)
calls. It can be answered by one line added to the existing print-only `GR-PUSHBUFFER` dump, or
off a boot that already runs `GR-ADDRESS-CENSUS`. ⊘ Do **not** assume guest RAM because the CE
ring was guest RAM (`run_w226a`: `ring=0x420064000 gpa=0x23092000`) — that is a different channel
family.

### 2.1 ★★ THE OPACITY CONSTRAINT — how the route must be shaped (owner ruling, 2026-08-11)

> *"in prod passthrough isn't parsed"*

The route's serve decision must read **zero** guest ring bytes. Concretely, when it is built:

- the serve path is `facts.route() == HostGr` → `None` (decline to the CE executor) → the
  existing forwarding fall-through → `exec_doorbell` → `plan_doorbell` → `VerbPlan::Doorbell` →
  `rm.schedule` + `rm.ring_doorbell(host_token)`. **Not one step of that reads the ring.**
- ⚠ **One entanglement exists today and it is on that path**: `plan_doorbell` runs the **#14
  ring-gate** over `working_set` (`fwd/src/lib.rs:2716-2726`, `VerbPlan::gated_doorbell`), which
  refuses `UngatedVa` for any VA in the working set that the address table misses. The working
  set is produced by the **caller** — so on the GR arm it must be **empty**, and the gate must
  therefore be vacuous rather than bypassed. ⊘ Handing the GR arm a working set derived from a
  ring read would be exactly the shape the owner ruled out.
- `dump_gr_pushbuffer_once` (`shim.rs:3912`, bounded + print-only) and `declare_gr_completion`
  keep their current shape: they observe, they decide nothing.
- **Required test, offline:** *a GR channel whose ring cannot be parsed still gets its doorbell
  forwarded* — construct an unresolvable `ring_va`, assert `DOORBELL … forwarded`, watch it RED
  first. ⊘ If it cannot be written, the entanglement is the finding.

### 2.2 ✔ THE FALSIFIER DOES NOT PARSE ANYTHING — confirmed, with one correction

The corrected signals are `CUP2_RC` (guest), host **Xid**, host **`GP_GET` vs `GP_PUT`**.

⊘ **Correction to the coordinator's phrasing:** `GP_GET`/`GP_PUT` are *not* "the host channel's
own progress counters on **our** allocation" once the ring is handed in — under
`RingOwner::HandedIn` the **GPFIFO is the guest's**. They remain readable without parsing
anything for a different and stronger reason: **`GP_GET`/`GP_PUT` are words in USERD, and USERD
is ours on both ring provenances** — `hUserdMemory[0]` is always `alloc_device_local` and always
CPU-mapped, while `ChannelRings::ring` is `None` and refused `RING_NOT_OURS` on the handed-in arm
(`rm.rs`, G4). ⇒ **The falsifier holds. No decision grep below reads a guest ring byte.**

---

## 3. THE PREDICTIONS — for the boot, when items 0/3b/7/8/9 are built

Control = the committed `w256_ce36a5b_cel_unbounded` shape (route refused).
Arm = same binary + the GR route, `KAYFABE_FB_BACKING=on`, `NVKVM_RAM_BACKEND=memfd`,
`KAYFABE_GUEST_RAM=memfd`, `ce_executor=host`.

1. **`DOORBELL-REFUSED … [Route::NotACopyEngineChannel]` drops from 8 to 0** on the arm and stays
   **8** on the control. The `by engine:` histogram keeps `GrCompute=8`.
2. `ENGINE-OBJECT … engine=GrCompute … materialized_channel=true` stays at **8** on both — this
   rung does not change where the channel is born (item 4 above).
3. ★ `COMPLETION-WATCH … NOT-OBSERVED samples≈88` — **DIAGNOSTIC ONLY.** ⊘ It is *not* the
   success signal and must not be read as one; under passthrough we neither write nor wait, and a
   watcher reporting the right value is a different read of one address from the guest's.
4. ★★★ **Primary: `CUP2_RC` 124 → 0.** Nothing else decides A.
5. `GR-ADDRESS-CENSUS operands=5 bound=4 unbound=1 mme_dwords=39` unchanged — it counts binding.
6. **The control must print zero of every new line.** A new instrument that fires on the control
   is a neutrality failure and is reported before any arm result.

⇒ If (1) is false the route did not arm and **nothing else in the log may be read**.
⇒ If (4) is true and (3) still says `NOT-OBSERVED`, **report that first** — it would mean the
guest saw a value our watcher did not, which is the ordering hazard the tree already names.

### 3.1 The three-value falsifier (owner's corrected form)

| value | PRIMARY (guest) | corroborating (host) | reading |
|---|---|---|---|
| **A** | `CUP2_RC` **0** | host channel `GP_GET` caught `GP_PUT` | the rung landed |
| **B** | `CUP2_RC` **124** | **host Xid** present | the GPU tried and faulted — addressing; the Xid names the address |
| **C** | `CUP2_RC` **124** | no Xid, host GR channel exists, **`GP_GET` never advanced** | the GPU never fetched — doorbell/token |
| ⊘ **non-answer** | — | `run_<tag>_dmesg.log` zero bytes **or** `grep -c NVRM` = 0 **or** the rev stamp ≠ the branch SHA | **the measurement did not happen.** NOT the B row |

---

## 4. THE EXACT RUN — bundle, tree, command, decision greps

**Transfer** (coordinator does this; names requested):

| what | name |
|---|---|
| bundle | `/workspace/hostgr_w259.bundle` (`git bundle create hostgr_w259.bundle origin/master..hostgr-route-over-guest-ring` plus `master`) |
| fresh tree on `vh` | `/workspace/kayfabe_w259` (clone from the bundle; ⊘ never `git pull`, never an existing tree) |

**Build + boot, on `vh` as root:**

```sh
export PATH=/root/.cargo/bin:$PATH          # cargo is NOT on the non-interactive PATH
cd /workspace/kayfabe_w259
REV=$(git rev-parse HEAD); echo "REV=$REV"  # ★ carry this into every claim
unset CARGO_TARGET_DIR                      # build_qom_shim.sh copies from $REPO/target/release
cargo build --release -p kayfabe-qemu-raw --features host-isolates
./scripts/bench/build_qom_shim.sh
strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -o 'kayfabe-rev:[0-9a-f]\{40\}'
#   ⇒ MUST equal $REV. If it does not, STOP: the bench is serving an older binary.

# arm
KAYFABE_ISOLATES=1 KAYFABE_FB_BACKING=on \
NVKVM_RAM_BACKEND=memfd KAYFABE_GUEST_RAM=memfd \
POST_CAPTURE_HOOK=scripts/bench/guest_cuinit_wall.sh \
  ./scripts/bench/host_xid_watch.sh ./scripts/bench/boot_capture.sh w259_${REV:0:7}_grroute
# control: identical, WITHOUT the GR route selected
```

**Decision greps — in this order. ⊘ None of them reads a guest ring byte.**

```sh
B=/workspace/bench; T=w259_${REV:0:7}_grroute

# ⊘ GATE 0 — did the measurement happen at all?
test -s $B/run_${T}_dmesg.log            || echo "NON-ANSWER: dmesg is zero bytes"
grep -c NVRM  $B/run_${T}_dmesg.log      # must be > 0
strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -c "kayfabe-rev:$REV"  # must be 1

# 1 — did the route arm?
grep -c 'Route::NotACopyEngineChannel'   $B/run_${T}_qemu.log     # arm: 0   control: 8
grep -o 'by engine: [^;]*'               $B/run_${T}_qemu.log | tail -1   # GrCompute=8 both

# 4 — PRIMARY
grep -o 'CUP2_RC=[0-9]*'                 $B/run_${T}_probe.log    # A iff 0

# B vs C — host side only
grep -ci 'Xid'                           $B/run_${T}_hostdmesg.log
grep -o 'GP_GET [0-9]* GP_PUT [0-9]*'    $B/run_${T}_qemu.log | tail -5

# 3 — DIAGNOSTIC ONLY, never the success signal
grep -c 'COMPLETION-WATCH .* NOT-OBSERVED' $B/run_${T}_qemu.log
```

---

## 5. ⊘ WHAT THIS FILE DOES NOT ESTABLISH

- ⊘ **Nothing was booted.** No claim here rests on a boot I ran.
- ⊘ **No code was written.** `DoorbellRoute::HostGr` still has zero consumers at this branch's
  HEAD, and `alloc_channel_over_guest_ring` still has one caller.
- ⊘ **§0.2's disposition of the token is INFERRED** from live code. The C measurement it reads
  against *is* measured, and was located and quoted — but nobody re-ran it against kayfabe's
  host-token table, and nobody has measured a **GR** doorbell being translated on either port.
- ⊘ **Item 9 (the ring's plane) is UNDETERMINED** and is the first thing the next rung must
  measure. Guessing it picks the wrong primitive.
- ⚠ **Two adjacent lanes touch these objects** — `carry-the-guests-engine-and-close-the-ring-gate`
  (`11cced9`, version-keyed `ChannelEngineWire` decode) and `channel-kind-two-axes` (the declared
  passthrough/emulated × passthrough/managed split, `kf-chankind` at `d55187a`). The two-axis lane
  is building the *declaration* this rung would consume; landing item 8 without it would spell the
  same distinction twice.

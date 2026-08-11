# §5.12 — ONE MEMORY for a framebuffer leaf (`w231`)

**STATUS: LIVE, 2026-08-11 — FORWARD-PORTED, and TWO THINGS CHANGED.** This doc was written on
branch `fb-join` (`2fe5f39`), 80 commits behind `origin/master`. It is now on `master`'s
shapes, and its own text is out of date in exactly two places, each corrected in place below:
- ★ **the bind now declares `kayfabe_mmu::BackingBytes::JoinsGuestWindow`** and is adjudicated
  by ruling 3/4 at `Binding::real_gpu_memory`. That machinery did not exist when this was
  written. See `gpga_region_kind.md` §8.1's `CORRECTED 2026-08-11` block for the decision and
  why the aperture is **not** corrected to sysmem.
- ⊘⊘ **the order in §"The order" below is WRONG as of the port.** It was join(+bind) → adopt
  → install; it is now join → adopt+map → install → **bind**. See the correction at that
  section.

⊘ **The hardware result quoted here is CITED, not re-run, and it is not a measurement of this
code**: `[measured 2026-08-11, vh2, GA106, 580.159.04, 8eb8dcd]` ran the old ordering over a
`BackingBytes` variant that did not exist. **Nothing in this tree has booted.**

**Rung:** `fb-join` / `257016e`, based on `2718c22` (`R30`, the CPU-view probe).
**Question, from the brief:** close *"two memories"*. `w228` backed three framebuffer
operands with real card memory at the guest's own addresses — **blank, and with no view the
guest shares**. Under execution the engine reads the *real* object while the guest reads the
*emulator's fabricated* one: anything the guest wrote appears as zeros, anything the engine
writes is invisible. ⚠ **Silent in both directions** — no fault, no error, no status.

**Answer: the leaf is now one memory, and the ordering is safe by construction rather than by
discipline.**

Companions: `fb_cpu_view.md` (`R30`, which measured the chain on real hardware and is not
re-proved here), `fb_leaf_crossing.md` §5.9 (`w228`, the chain this **replaces**),
`guest_ram_crossing.md` §5.8 (the first crossing, whose shape this is with the `memfd`'s owner
inverted), `isolate_vmm_fd_crossing.md` §12 (decision (b), which is what refuses the
alternative).

---

## 0. ★★★★★ WHAT I REFUTED FIRST — INCLUDING MY OWN BRIEF

### 0.1 ⊘ REFUTED, from the brief: *"§5.11"*

The brief and this rung's own first draft numbered this section **§5.11**. `w230` (`ad4ed3c`,
*"the guest's ring is adopted on a real GA106"*) had already taken it, in a tree live on the
other bench at the time. ⊘ A trivial collision, recorded because the *mechanism* is not
trivial: two agents numbering sections from the same base commit will collide every time, and
nothing in the tree makes them notice. This is **§5.12**.

### 0.2 ⊘ REFUTED, MINE: *"an `Option<ExportDirectory>` field can be `#[cfg]`-gated"*

The composition root's route from a backing token to a descriptor exists only in a build with
`host-isolates`, so the obvious shape is a conditional field. **Rust does not allow it**:
attributes are not permitted on tuple-struct fields or in tuple patterns, so a conditional
field becomes a conditional *arity* — two shapes of one type, and every destructuring written
twice. ⇒ The condition moved into what the shape **contains** (`FbExportDir` is `()` without
the feature), which is where it belonged: an archive with no isolate plane has no directory to
carry, and `()` says exactly that. `shim.rs`'s `FbExportDir`.

### 0.3 ⊘ REFUTED, MINE, and caught by the tests rather than by reading

`LoopbackRm::join_fb_leaf` checked its **argument** (`known(vas)`) before its **composition**
(is there a shared join table?). A backend with no table then answered `BadHandle`, naming the
caller for a fault that is entirely ours. ⊘ Ordering fixed to match `HostRmBackend`: the pool
gate first, by name, for every argument.

### 0.4 ⊘ NOT REFUTED — the three things the brief settled, checked and left alone

- **`Request::ExportBacking` / `ExportSource::HostDeviceMemory` cannot be used.** Confirmed by
  reading and unchanged: `rm.rs`'s `export_backing` destructures the source and returns
  `NotExportableAsMemory` before any host call. Nothing in this rung touches it, and nothing
  here copies device pages into a `memfd`.
- **The join is a MAPPING, never a connection.** `FbStore`'s own docs still nominate a
  delegating "connection to an isolate, every access a round trip", and
  `tests/tests/unranked_locks.rs:56-59` still forbids it. ⇒ `FbStore::install_join` replaces
  the store's **pages**, never its lookup, and its doc-comment carries the argument so the
  next reader of `fbwin.rs` meets both halves in one place.
- **The leaf becomes host SYSTEM memory.** Named on `kayfabe_isolate::FbLeafJoined`, on
  `FbLeafBacking::Joined`, in the shim's own boot line, and in §4.1 below.

### 0.5 ⊘ WHAT THIS RUNG DOES NOT DO, stated before any green line

- **`cup2` does not pass and no doorbell is routed.** `Route::NotACopyEngineChannel` refuses
  every `GrCompute` doorbell exactly as at `w228`. **The guest did not move.**
- **Nothing executes.** A joined leaf is memory both parties can reach; it is not an engine
  being pointed at it.
- **`SET_SHADER_SHARED_MEMORY_WINDOW` is still `Unresolved`**, and no sweep enumerates the
  leaves an operand does *not* name. `gr_execution_boundary.md`'s other reasons stand.

---

## 1. ★★★ THE CHAIN, AND IT IS `PinGuestRam`'s WITH THE OWNER INVERTED

```
   isolate                                        VMM
   ───────                                        ───
1  mint a sealed memfd            ExportSource::Fabricated
2  mmap it here                   → the GPU-side view, held for the isolate's life
3  NV01_MEMORY_SYSTEM_OS_DESCRIPTOR over that mapping
4  map_gpu_va(vas, ., len, at)    DMA_OFFSET_FIXED — placement CHECKED, twice
5  the descriptor rides Reply::JoinedBacking ───▶  adopt into ExportRegistry
                                                  mmap it here → the guest-side view
6                                                  ESTABLISH: copy SparseFb's own bytes in
7                                                  INSTALL: the range goes live
```

★ Guest RAM crosses VMM→isolate at spawn on a fixed fd number; this crosses isolate→VMM on the
reply. **No device fd anywhere** — decision (b) is honoured rather than circumvented.

### 1.1 ⊘ Why it is ONE verb and not `export_backing` + `map_gpu_va`

Because **the VMM cannot name the isolate's backing.** `ExportedBacking::token` is the
*adapter's* index, minted when the adapter adopts the descriptor; the child's index into its
own table deliberately does not travel (`export.rs`'s module docs — *"a value the peer supplies
must never name a slot in our registry"*). A VMM holding an exported backing therefore has no
way to say *"describe **that** one to RM at **this** VA"*. The two halves have to be one verb,
issued by the party that owns the `memfd`.

⇒ `Request::JoinFbLeaf` is the **second** request whose reply may carry a descriptor. That is a
protocol-policy change from a set of one to a set of two, and it is written down as one: every
other reply is still read with an `max_fds` allowance of **zero**, so the kernel — not a `case`
— is what stops a child attaching a descriptor to an `Alloc`.

### 1.2 ★★★ The establishment copy, and why it is what makes the ordering safe

⊘⊘ **CORRECTED 2026-08-11 (the forward-port) — THE ESTABLISHMENT COPY WAS NOT THE WHOLE OF
THE ORDERING, AND THE HALF IT MISSED IS THE FAILURE PATH.** Read this before §1.2 and before
the *"The order"* list in §1.

Everything below about the copy is correct and is unchanged in the code. What it does **not**
cover is the **bind**. As written, this rung's order was:

> **1.** join (`back_fb_leaf`, *which also bound the row*) → **2.** adopt + `mmap` →
> **3.** establish + install

So between step 1 and step 3 the address table said the range was host-backed while the
guest's window still pointed at the emulator's own page. ⚠ And the real problem is not the
window's width: **if step 2 or step 3 REFUSED, the row stayed** — permanently declaring a join
that never happened. This rung's own code logs that state as `⊘ Two memories, named` and
leaves the row in place. Under the port the row's declaration is
`BackingBytes::JoinsGuestWindow`, so it would be `w228`'s two memories under the one word that
says they are one — strictly worse than `w228`, which at least declared the shadow.

★ **The order is now:**

> **1.** join (`back_fb_leaf`, which binds **nothing** and adopts only the host facts) →
> **2.** adopt + `mmap` → **3.** establish + install → **4. bind**
> (`SharedDevice::adopt_joined_fb_leaf`)

and every path that does not reach step 4 calls `release_unadopted_fb_leaf` instead of
binding. ⊘ The cost, stated: between 1 and 4 the isolate holds a fixed map at the guest's VA
that core state does not name, so a re-ask in that gap re-plans as a **first** join and RM
answers the second fixed map at an occupied address `0x51` — which cannot be told apart from
exhaustion. The release is what closes it.

★ A consequence rather than an incidental: **the adopt is what makes a replay a replay.**
Idempotence is read off the row's host backing, so a join whose install never happened
correctly re-asks as a first join instead of replaying onto a window that points elsewhere.

⊘ **The bench measurement in §3.2 ran the OLD order.** It is not a measurement of this code.

Bytes the guest wrote **before** the backing existed are already in `SparseFb`. At install, the
store reads them **from its own pages** — never through the range it is installing, which by
then answers for them — and copies them in **before** the range goes live.

★★★ This is not a detail. The owner's objection was *"mapping after execution seems racy to
me"*, and it is correct: once the engine has written the real object and the guest has written
the fabricated one, **there is no correct merge** — a merge is a choice about which writes to
lose. With the copy at install, after it there is ONE memory and there is never a merge. The
safety is by construction.

★ It follows that establish-and-install must be **atomic against guest access**, and it is:
`RegPlane::join_fb` holds the plane lock across both, so no framebuffer read or write can land
in the window where the bytes exist in two places. ⊘ And nothing under that lock blocks — the
isolate round trip has already happened, and a `memcpy` into an `mmap` blocks on nothing.

★ The local pages are **released** at install, after the copy. A store that kept them would
have the two-memories defect one layer down: its own stale page and the joined backing. Their
`origin` rows go with them, or a residency census would name a page it can no longer see.

### 1.3 ⚠ REPLACES, does not extend

`FbLeafBacking::Vidmem` — `w228`'s chain — is still expressible and has **zero production
callers**. A leaf served by both would have two host objects at one guest VA. The arming
variable is renamed rather than extended: `KAYFABE_FB_BACKING` is **gone**, and `KAYFABE_FB_JOIN`
refuses its `on` spelling by name so a stale bench script fails loudly instead of disarming
silently.

---

## 2. ★★★★★ THE POOL, and why every test here runs FOUR workers

**An isolate is a pool.** `child.rs` runs one `HostRmBackend` per worker thread, and the VMM's
requests are served by whichever worker's socket is idle — a join and the read of it need not
land on the same one. A join table on the backend would therefore be correct on every
one-worker test, on every `cargo test` in this workspace and in Clippy, **and wrong at the first
boot with a pool of two.**

⇒ `crates/kayfabe-isolate-host/src/fbjoin.rs` is a per-**isolate** object, built once in
`child.rs` and cloned into every worker. ⊘ There is deliberately **no constructor path that
gives a backend a private table**: `with_fb_joins` installs a shared one, or the backend has
none and **refuses the verb by name** (`FB_JOIN_NO_TABLE`). The failure mode is unrepresentable
rather than guarded.

★ `crates/kayfabe-isolate-host/tests/fb_join_crossing.rs` joins on one slot and reads on
another, over a pool of four. A `pool_size(1)` there would make the whole file vacuous, which
is why the constant carries that paragraph.

---

## 3. MEASURED

### 3.1 Off the bench — a real spawned isolate, a real socket, no GPU

`crates/kayfabe-isolate-host/tests/fb_join_crossing.rs`, 5 tests. This pays a bound
`export_backing.rs` stated about itself in as many words: *"⊘ What it does not prove: that the
**isolate** can see the write. Nothing in the port reads the isolate's own view of a backing."*
`fb_join_peek` is that port, so the two halves of a backing are compared **from opposite
processes** rather than from two duplicates in one.

- both directions agree over one fabricated backing, across two processes and three pool slots;
- ★★ **the negative control fires**, and its VMM-side read returns **direction 1's own
  pattern** rather than zeros — so both views are live and hold different bytes, which zeros
  alone could not have shown. `fb_cpu_view.md` §3.2's signature, reproduced;
- an unjoined address answers `Ok(false)` — a MISS, never a page of zeros;
- a join's descriptor is checked by the **kernel** (`DescriptorKind::RegularFile`, and `/proc`
  independently says `/memfd:`), exactly as an export's is;
- a backend with no shared table refuses **by name**, and the instrument refuses too rather
  than answering `Ok(false)` — which would say the isolate looked and found nothing.

`crates/kayfabe-device/tests/fb_join.rs`, 9 tests, over the store's own half: the establishment
copy is **non-vacuous** (asserted on `nonzero > 0`, not on "it ran"), an untouched leaf copies
nothing **and says so**, a failed copy installs **no** join at all, the local page is released,
an access straddling the edge is not half-served, and `device_reset` forgets joined ranges —
the cross-life leak `fb_cpu_view.md` §4.3 names.

### 3.2 ★★★★★ On the bench — real GA106, `w231a_257016e_join`

`[measured 2026-08-11, bench `vh2`, RTX 3060 GA106, host driver 580.159.04,
`KAYFABE_ISOLATES=real KAYFABE_CE_EXECUTOR=local KAYFABE_GUEST_RAM=memfd KAYFABE_FB_JOIN=shared`]`
**QEMU binary stamped `257016eb`**, asserted on the artifact before the boot — and asserted
*negatively* as well: `strings` over the built binary finds `GR-FB-JOIN` and `KAYFABE_FB_JOIN`
and **zero** occurrences of `GR-FB-BACKING`. The replacement is in the artifact, not only in
the tree.

```
kayfabe: FB-JOIN arm=shared exports_directory=true ⇒ leaves are JOINED — one backing, two mappings, ONE memory

kayfabe: GR-ADDRESS-CENSUS proc=2 chan=0 class=0xc7c0 operands=5 bound=4 unbound=1 mme_dwords=39
GR-FB-JOIN SET_VALID_SPAN_OVERFLOW_AREA leaf va=0x200000000   len=0x200000 fb_phys=0x400000 → JOINED (shared) memory=0xcafe005e host_va=0x200000000   placed_as_asked=true
GR-FB-JOIN SET_TEX_SAMPLER_POOL          leaf va=0x10002000000 len=0x200000 fb_phys=0x800000 → JOINED (shared) memory=0xcafe005f host_va=0x10002000000 placed_as_asked=true
GR-FB-JOIN SET_TEX_HEADER_POOL           leaf va=0x10000000000 len=0x200000 fb_phys=0x600000 → JOINED (shared) memory=0xcafe0060 host_va=0x10000000000 placed_as_asked=true

GR-FB-JOIN ★ DIRECTION 1 (guest view → isolate view) fb_phys=0x400000 AGREES over 1024 words
GR-FB-JOIN ★ DIRECTION 2 (isolate view → guest view) fb_phys=0x400000 AGREES over 1024 words

GR-ADDRESS-CENSUS (RE-STATED AFTER JOINING) proc=2 chan=0 joined_leaves=3 live_views=3
```

★★★★★ **Both directions, on a real GA106, over a leaf the census actually named** — not a
synthetic buffer. A per-word pattern written through this device's own framebuffer window (the
path a guest `PRAMIN` store takes) is what the isolate's mapping holds, and what the isolate
writes is what that window reads back. `w228`'s three leaves are the same three leaves, at the
same three VAs and the same three framebuffer addresses, and `placed_as_asked=true` on all
three.

★ **The replay arm fires seven times and is silent about views**: `chan=1` … `chan=7` print
`ALREADY JOINED (no second object, no second descriptor, no second establishment copy)` and
then `joined_leaves=3 live_views=0`. ⊘ The probe correctly says *"NO PROBE: no leaf reached a
live view this doorbell — that is the absence of a measurement, NOT a measurement of absence."*

### 3.3 ⊘⊘ REFUTED, AND IT IS MY OWN BRIEF: the establishment copy was **VACUOUS** on hardware

The brief says, and I built to it: *"Bytes the guest wrote **before** the backing existed
already live in `SparseFb`."* **Not for these leaves.** All three report

```
established=0 bytes over 0 page(s), of which 0 NON-ZERO
⊘ the establishment copy was VACUOUS for this leaf: no page of it was resident, so nothing
  the guest had written came across. That is CORRECT (an unwritten leaf is zeros either way)
  and it is NOT evidence that the copy works
```

⇒ **Bar 2 is NOT met on hardware, and I am not going to call the substitute equivalent.** The
copy is proved by `crates/kayfabe-device/tests/fb_join.rs` (asserting `nonzero > 0`, and that a
failed copy installs no join at all) and exercised end to end in the loopback crossing; on the
real GA106 it moved **nothing**, because the guest has never written those framebuffer pages
through any window this device serves.

★ That the instrument **said so** is the point. A report that printed only `established=N` with
no `pages`/`nonzero` terms would have shown `0` and read as a number rather than as *"this
measurement is vacuous"*. The vacuity arm was written before the boot, for exactly this.

⚠ And it is a **finding about the leaves**, not only about the instrument: `w227c` resolved
these three operands through the guest's own page tables, so the guest's page tables *bind*
them — and yet nothing has ever put a byte in them through `PRAMIN`, `BAR1` or `BAR2`. Whether
that is because the guest writes them by an engine we do not execute, or because it has not
written them yet at the instant of the first GR doorbell, this rung does not say and must not
guess.

### 3.4 ★★★ THE DOORBELL CENSUS DID NOT MOVE, and that is this rung's own control

```
nvkvm: doorbells: 191 arrived, 183 served, 8 REFUSED by name
nvkvm:   of the served: 183 local (CPU CE, end witnessed), 0 forwarded (host channel rung)
nvkvm:   by engine: GrCompute=8 GrGraphics=0 Ce=183 NvEnc=0 NvDec=0 Other=0 unrouted=0
nvkvm:   first doorbell refusal [Route::NotACopyEngineChannel] …
```

⊘ **Byte-identical to `w218`, `w227c` and `w228a`.** The address plane changed and the
submission plane did not, which is what a rung touching only the address plane must produce.
**The guest did not move**: `cup2` still spins inside `cuCtxCreate`, `forwarded = 0`, and every
`GrCompute` doorbell is still refused by name. If those numbers had moved, something here would
be doing more than it says.

### 3.5 ★★★★★ THE NEGATIVE CONTROL — `w231b_257016e_control`, watched to fail

**Same binary, same tree, same stamp, `KAYFABE_FB_JOIN=private`.** One property changes: the
VMM maps the isolate's backing `MAP_PRIVATE|MAP_ANONYMOUS` instead of `MAP_SHARED` —
`kayfabe_linux_raw::Backing::PrivateAnonymous`'s arm of the `mmap` argument computation
(`crates/kayfabe-linux-raw/src/mapping_unsafe.rs:344-347`). The isolate chain, the join table,
the establishment copy and both probes either side of it are the **same code**. ⊘ Not "a second
memfd", which would be a tautology.

```
kayfabe: FB-JOIN arm=private exports_directory=true ⇒ leaves are the NEGATIVE CONTROL …
GR-FB-JOIN ⊘ DIRECTION 1 (guest view → isolate view) DISAGREES at word 0 (got 0x00000000, want 0x5a1a5a5b) of 1024
GR-FB-JOIN ⊘ DIRECTION 2 (isolate view → guest view) DISAGREES at word 0 (got 0x5a1a5a5b, want 0xa5e5a5a4) of 1024
GR-FB-JOIN   ★★ AND THE VALUE READ BACK IS DIRECTION 1'S OWN PATTERN, not zeros …
```

★★ **Read the second line — it is the strongest single signal in this rung.** The control's
guest-side read did not return zeros. It returned `0x5a1a5a5b`: the **direction-1 pattern**,
still sitting in the private pages this run wrote it into, because direction 2's write went to
the shared `memfd` and never reached them. A control that merely returned zeros would be
consistent with a mapping that was never written at all; this one demonstrates both views are
live, hold different bytes, and are read by the same loop.

⊘ And `fb_cpu_view.md` §3.2 measured the *identical shape* on this bench three hours earlier,
against `R30`'s standalone ladder rung. The same control, now over a real framebuffer leaf
inside a real boot.

### 3.6 ★★★ THE ARMING CONTROL — `w231c_257016e_arming`, and the three-arm table

**Same binary, same tree, same stamp, `KAYFABE_FB_JOIN=off`.** Nothing is materialized.

| | armed `shared` (`w231a`) | negative control `private` (`w231b`) | arming control `off` (`w231c`) |
|---|---|---|---|
| `GR-FB-JOIN` lines | 36 | 37 (the extra is the control's own ★★ line) | **0** |
| `JOINED (shared)` | **3** | 0 | 0 |
| `JOINED (private)` | 0 | **3** | 0 |
| `★ DIRECTION … AGREES` | **2** | **0** | 0 |
| `⊘ DIRECTION … DISAGREES` | **0** | **2** | 0 |
| `GR-ADDRESS-CENSUS` blocks | 8 | 8 | **8** |
| `HostBackedFb` rows | 24 | 24 | **0** |
| `Framebuffer { … }` rows | 24 | 24 | **24** |
| doorbells | `191 / 183 / 8` | `191 / 183 / 8` | `191 / 183 / 8` |

★ **The `off` arm is silent, not merely quiet** — it prints no line the armed run does not, so
its log is a *subset* of the armed one rather than a different log. ⊘ That was a deliberate
choice at the arming check; a "join disabled" line would have made the control incomparable.
And the census still runs 8 times with all 24 rows reading `Framebuffer { … }`, which is what
says the *subject* was exercised and only the *treatment* was withheld.

★★ Read the `a` vs `b` columns together: **everything except the two direction lines is
identical**, including the join itself, the placement, the re-statement and the doorbell
census. The negative control changes the VMM's *mapping*, not the join — which is why `JOINED`
still prints three times and `HostBackedFb` still appears 24 times in both.

⚠ **This is the trap that costs boots, closed.** A boot can run with the plane off and still
produce a full `dmesg`, a full serial log, a full census and `RC=0` with not one line of the
changed code having run. The `FB-JOIN arm=…` line is printed on **every** arm, from the
composition root's own single reading, so a reader can tell the three apart from the boot's own
on-disk evidence rather than from whichever shell exported the variables.

★ **Which line did I expect this to execute?** The `Backing::PrivateAnonymous` arm named above,
and nothing else — every other line in the join path is shared with the armed run. That is the
question `w228`'s first control failed (it asked `back_fb_leaf` about an address it never
walks, and would have printed `✅ REFUSED` **while allocating a real object**) and `w229`'s
failed (its probe was handed the address its own ring had just been freed from).

---

## 4. THE COSTS, NAMED

### 4.1 ⚠ The leaf is host SYSMEM, not card memory

The engine reaches it over PCIe instead of out of local framebuffer. That is a **performance**
divergence from the C artifact (`C: nvkvm_gpu_emul.c:8454-8459` double-maps a *vidmem* object)
and from `w228`, not a correctness one, and it is the identical trade the first crossing
already makes for the guest's ring.

⊘ **It is not optional.** The C can double-map vidmem because it is monolithic: QEMU holds
`/dev/nvidia` itself, so the CPU half never crosses a process boundary. Here it would have to,
and `fb_cpu_view.md` §0.1's three cited driver facts are why it cannot. Card memory is exactly
the memory that cannot carry a guest-reachable CPU view.

### 4.1b ⚠ THE PROBE WRITES DEVICE MEMORY, and that is a cost rather than a detail

`probe_joined_leaves` writes 4 KiB of pattern into the **first** joined leaf, through
`RegPlane::fb_poke` — deliberately, because that is the path a guest `PRAMIN` store takes and a
check that wrote through some other door would be measuring a door the guest does not use. It
then leaves the isolate's poke pattern there.

⊘ On this boot that destroyed nothing: the establishment report says the leaf had **no resident
page at all** (§3.3), so there were no guest bytes to lose. ⚠ **That will not stay true.** The
first boot in which a leaf carries guest content is a boot in which this instrument corrupts
4 KiB of it, and nothing in the code notices. It is armed only by `KAYFABE_FB_JOIN` and only on
the first live leaf per doorbell, but it is not observationally neutral and must not be left
armed once the join is load-bearing — the same warning the C artifact's `m2rec` carries.

### 4.2 ⊘ What §4 of `fb_cpu_view.md` left open and this rung still leaves open

- **Extent.** One `SharedRam` per leaf is the safe unit, not the efficient one. The C's
  run-coalescing and 2 MiB re-cut needs a leaf enumeration this port does not have.
- **Lifetime.** The FB re-back gap `fb_leaf_crossing.md` §3.1 records is inherited unchanged: a
  leaf the guest unbinds and re-creates over a different frame keeps its host object.
- **Reclaim.** A `memfd` minted for a join that then refused is held by `ChildExports` until the
  isolate dies. Reclaiming exports is a table-lifetime question and is not this verb's to
  answer — the same asymmetry `describe_guest_ram` records for the mapping it does not free.

---

## 5. What the next rung inherits

1. **A framebuffer leaf is ONE memory, measured on a real GA106** (§3.2). The engine's object
   and the guest's window are the same pages, in both directions, at the guest's own VA.
2. ⊘ **The establishment copy is unmeasured on hardware** (§3.3) and proved only off it. The
   first thing that writes one of these leaves through an emulated window will make it
   non-vacuous, and the instrument already prints the number that will say so.
3. ⊘ **Not the doorbell gate.** `Route::NotACopyEngineChannel`, 8 of 8, unchanged.
   `gr_execution_boundary.md` §3 stands: the VA space is the only sound containment surface for
   a stream carrying 39 dwords of guest-authored MME microcode, and this rung makes one more
   piece of that surface real without opening the gate.
4. ★ **`SET_SHADER_SHARED_MEMORY_WINDOW` is still `Unresolved`** — one of the four bound
   operands is still backed by nothing, and no sweep enumerates the leaves an operand does not
   name.
5. ⚠ **The extent, the lifetime and the reclaim** listed in §4.2 are all still open.

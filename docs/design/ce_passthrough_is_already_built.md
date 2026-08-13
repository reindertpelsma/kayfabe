# CE passthrough is ALREADY BUILT — and the blocker is the RAW CLIENT'S OWN MEMORY LAYOUT

**STATUS: LIVE — 2026-08-13 (w284).** Supersedes the premise of the `w284` brief, which asked
for *"one core-reachable verb that lowers to `HandedIn` for CE"*. That verb exists, is
engine-blind, and has already fired for CE on real GA106 — **88 times**.

⚠ Everything below is read out of committed boot logs and out of the code at
`8a47073`. No claim here is from a doc comment; where a doc comment says otherwise it is
named as stale in §5.

---

## 0. ★★★★★ THE HEADLINE — the brief's premise is refuted by our own committed trace

The brief's reasoning rested on `crates/kayfabe-isolate-host/src/rm.rs:582`:

> *"[`RmBackend::alloc_channel`] — the only channel verb the core can reach — lowers
> **unconditionally** to `RingSource::Ours(None)`, so every host channel is
> [`RingOwner::Ours`] … The [`RingSource::Guest`] arm has exactly **one** caller in the
> workspace and it is `bin/rmladder.rs`'s R31 diagnostic probe, never the core."*

**That sentence was true at `361fca8` and is false at HEAD.** `alloc_channel_over_guest_ring`
now has **four** call sites, and one of them is `rm.rs:4018` — inside `alloc_channel` itself,
the core-reachable verb. The brief flagged this risk itself (*"VERIFY THE LOWERING IN CODE
BEFORE BUILDING… If a CE route to `HandedIn` already exists, say so and skip ahead"*). It
does. This is the skip-ahead.

### The route, measured firing on a CE channel

`traces/boots/w283/run_w283d_client_qemu.log.gz`, the raw CE client's own channel:

```text
kayfabe-isolate: GR-BIRTH iso2/gpu0 #1 engine=Ce vas=0xcafe0005
  adopt=GUEST-RING memory=0xcafe0006 ring_va=0x120020000 gp_fifo_va=0x120021000 entries=64
  userd_memory=NONE userd_offset=NONE
  userd=DECLINED ⊘ the ring's leaf was consulted — and the guest's resolved USERD was
        UNREADABLE, in guest RAM, undeclared, or outside that leaf
  joined=YES ⇒ the address table held a JoinsGuestWindow binding at this channel's declared
        gpFifoOffset → alloc_channel_over_guest_ring
  [births=1 guest_ring=1 guest_userd=0 declined=0 not_asked=0 refused=0]
```

⇒ **A host CE channel was born over the guest's own ring.** `engine=Ce`. `joined=YES`.
`alloc_channel_over_guest_ring`. Leg A2 is live for CE and was live before this rung began.

And the doorbell is forwarded for it, in the same boot:

```text
kayfabe: DOORBELL-XLATE proc=2 chan=0 vchid=VChid(0x3) engine=Ce
         guest_token=0x00000003 host_token=0x6 schedule=true
kayfabe-isolate: DOORBELL-VERB engine=Ce host_token=0x6 scheduled=true → calling ring_doorbell
kayfabe-isolate: DOORBELL-STORE #1 host_token=0x00000006 runlist=0 chid=6 → storing 32 bits
                 at USERMODE_NOTIFY_CHANNEL_PENDING=0x90 in the mapped usermode window
kayfabe-isolate: DOORBELL-STORE #1 host_token=0x00000006 ★★★ WROTE — the store executed
```

⇒ **Map + create-over + forward-the-doorbell — all three of the brief's "pieces" — are built,
wired and firing for CE.** The rung's stated gap does not exist.

### It is not a one-off: 88 CE births took the route

Cross-tabulated over `traces/boots/w263..w269/*.log`:

| birth | count |
|---|---|
| `engine=Ce adopt=GUEST-RING` | **88** |
| `engine=GrCompute adopt=GUEST-RING` | 88 |
| `engine=Ce adopt=DECLINED` | 108 |
| `engine=GrGraphics adopt=DECLINED` | 28 |

And on the **guest driver's own** channels leg B fires too — `traces/boots/w267/run_w267_on_qemu.log`:

```text
GR-BIRTH iso2/gpu0 #9 engine=Ce … adopt=GUEST-RING memory=0xcafe0006
  ring_va=0x200200000 gp_fifo_va=0x200218000 entries=1024
  userd_memory=0xcafe0006 userd_offset=0x1a000 userd=GUEST-USERD … joined=YES
  [births=9 guest_ring=9 guest_userd=9 …]
```

⇒ `userd_memory` **is** `ring memory`, at offset `0x1a000`: ring and USERD in **one** joined
leaf. That is the shape leg B was built for, and on the guest driver's channels it holds.

---

## 1. ★★★★★ WHY THE RAW CE CLIENT STILL FAILS CRITERION 1 — one number, measured

`w283d`'s own descent line, same boot, same channel:

```text
RING-PROJ … engine=Ce … userd=h0xcafe000b/off0x0/phys=fb:0x50000/0x200 fbuserd@0x50088
            GET=0 PUT=1 … ring=0x120021000
            walk: … L4@0x5100[…]=LEAF@0x120020000->0x40000/Vidmem/sz0x10000 walkend=LEAF
```

Put the two numbers side by side:

| what | value |
|---|---|
| the ring's joined leaf | fb `0x40000`, length `0x10000` ⇒ covers fb `0x40000 .. 0x50000` **exclusive** |
| the client's USERD | fb `0x50000`, size `0x200` |

`adopted_guest_userd` (`crates/kayfabe-fwd/src/lib.rs:3993`) is a **containment test** against
the ring's binding:

```rust
let offset = base.checked_sub(binding.phys())?;          // 0x50000 - 0x40000 = 0x10000
if offset.checked_add(USERD_SLOT_BYTES)? > len {         // 0x10000 + 0x200 > 0x10000
    return None;                                          // ⇒ DECLINES
}
```

⇒ **The USERD misses the ring's leaf by exactly one byte of extent.** It is the *first byte
past the end*. The test is correct; the USERD simply lives in a different object.

★★★ **And that is a property of OUR OWN INSTRUMENT, not of the architecture.** The raw CE
client is `kayfabe-rm-ladder --ce-client`, and its channel is built by
`HostRmBackend::alloc_channel_in` (`rm.rs:4876`), which allocates USERD as a **second,
separate 64 KiB device-local object**:

```rust
let (userd, userd_owner, userd_offset) = match ring {
    RingSource::Ours(_) | RingSource::Guest(GuestRing { userd: None, .. }) => {
        match self.conn.alloc_device_local(RING_OBJECT_BYTES) {   // ← its OWN object
            Ok(h) => (h, UserdOwner::Ours, 0),
            …
```

Two `alloc_device_local(0x10000)` calls ⇒ fb `0x40000` and fb `0x50000` ⇒ two leaves. The
guest **driver** does the opposite (one object, USERD at `+0x1a000`), which is why leg B
fires there and declines here.

⇒ **The raw CE client is unrepresentative of the workload it stands in for, in exactly the
dimension that decides criterion 1.**

---

## 2. ⊘⊘ THE SECOND GAP — passthrough is still an ADDITION, not a REPLACEMENT

`w283`'s RESULT ruled that adoption and `ce_copy` are *"two designs, not two dispositions"*.
**The tree has not yet implemented that ruling.** Measured at HEAD, both run on one doorbell:

`crates/kayfabe-rt/src/device.rs:2262`:

```rust
if let Some(vmm) = vmm && ring_content_is_forwardable(out.engine) {
    self.forward_ring(vmm, out.proc, out.chan)?;      // ← reads the guest's ring,
}                                                     //   decodes, → ce_copy
```
```rust
pub fn ring_content_is_forwardable(engine: EngineKind) -> bool {   // device.rs:4596
    matches!(route_of_engine(engine), DoorbellRoute::CpuCe)         // i.e. engine == Ce
}
```

⊘ **There is no env flag on this.** The predicate is `engine == Ce`, full stop. So on the
`w283d` configuration a single CE doorbell does **both**:

1. `VerbPlan::Doorbell` → `ring_doorbell(host_token=0x6)` — the adopted, guest-ring channel;
2. then, on the same call, read-ring → decode → `VerbPlan::CeSplit` → `ce_copy` on a
   **different** host channel (`host_token=0x7`), which is the one that produced
   `CE-SUBMIT … by=HostCe … → RETIRED` and made criteria 2 and 3 green.

⇒ **Criterion 2's green is the composing path's, and criterion 1's red is the passthrough
path's — in the same boot, on the same doorbell, from two different host channels.** Reading
the four criteria as one verdict on one mechanism is a category error, and the scorecard
does not currently say which channel each row came from.

---

## 3. ⊘ WHAT THIS RUNG DID NOT BUILD, AND WHY — the ruling the owner needs to make

Criterion 1 needs the host channel to carry the guest's USERD. Three options; **all three
need a ruling, and none is a "work around it".**

### Option A — make the raw client's USERD live inside its ring object
Change `alloc_channel_in`'s `Ours` arm to place USERD at an offset within `ring_obj`
(`RING_OBJECT_BYTES = 0x10000`; `0x0`–`0x1000` pushbuffer, `0x1000` GPFIFO, `0x2000`
semaphore — `0x3000` onward is free).

- ★ **Uses the already-live production path**, changes no architecture, and makes the client
  *match the guest driver's own layout* — i.e. makes the instrument representative.
- ⚠ **Blast radius: it is shared production code.** `HostRmBackend` builds the host isolate's
  own channels too. And `userd` would then **alias** `ring_obj`, which touches four sites
  that today assume two distinct handles:
  `rm.rs:4152` (teardown frees `parts.ring` *and* `parts.userd` ⇒ **double free**),
  `rm.rs:5112`/`:5126` (two independent `map_cpu`s of one object),
  `userd_store_u32` / `read_gp_cursors` (must add the offset), and the `userd_offset_0` guard.
- ⊘ Not started. A double-free in a teardown path surfaces anywhere but at the call, and this
  rung will not put one into a boot un-reviewed.

### Option B — join the USERD's own framebuffer leaf
- ⊘ **Structurally blocked today.** The join is `resolve_leaf_of(ce, va)` — it walks a **VA**
  to a leaf. A USERD has **no GPU VA**; it is named to RM as `hUserdMemory` + `userdOffset`.
  There is nothing for the walk to start from.
- ⊘ Finding the leaf from `fb:0x50000` would be a **reverse lookup by physical address**,
  which `kayfabe_mmu`'s `gpga.rs` forbids by name (*"`fn owner_of(addr)` … and there never
  will be"*), and which `adopted_guest_userd`'s own doc calls out as the licence it does not
  take (*"THIS IS A CONTAINMENT TEST, NOT A RESOLUTION"*).
- ⇒ Needs a **new verb**: join a framebuffer object by phys, recorded forward against the
  channel that named it. That is an architecture change and an owner ruling.

### Option C — widen the ring's join to cover the adjacent leaf
- ⊘ **Refused on sight.** It would bind VA `0x120030000` → fb `0x50000` in the host VAS when
  the guest's own page tables say no such mapping exists. That is `ShadowsGuestMemory` /
  `#255`'s forbidden state wearing a convenience's clothes.

**Recommendation: Option A**, with the aliasing handled by a distinct `UserdOwner` arm rather
than by making two handles compare equal — and with the native arm of `--ce-client` as the
known-positive that RM accepts a USERD at an offset (it must, since the guest driver does it).

⇒ **Then, and only then, suppress `ce_copy` for the adopted channel** (§2), because
suppressing it first strictly regresses criteria 2 and 3 with nothing to replace them.

---

## 4. What a green run still could not prove

- Nothing here bounds `cup2`. The raw client builds **its own** `FERMI_VASPACE_A`; the GR
  channel `cuCtxCreate` walls on belongs to the guest driver's client with its own PDB.
- `#255` (`shim.rs:8591`) is computed from `CeChannelFacts` **inside the CE decode path**. On
  a true passthrough arm that path does not run, so #255 would go silent — and silence there
  is *"the instrument did not run"*, not *"the condition is absent"*. Re-arming it against the
  passthrough VAS needs its own input, not just its own call site.
- The doorbell's `working_set` is `&[]` at `shim.rs:4707`, so `plan_doorbell`'s `#14` ring
  gate is **vacuous** on every production doorbell. It is a type-level guarantee with nothing
  to check.

---

## 5. Stale doc comments that misdirected this rung — corrected in place

| site | says | actually |
|---|---|---|
| `rm.rs:582` | `alloc_channel` *"lowers unconditionally to `RingSource::Ours(None)`"*; the `Guest` arm has *"exactly one caller … the R31 diagnostic probe"* | four call sites; `rm.rs:4018` is inside `alloc_channel` and is the production lowering |
| `shim.rs:12664` (`GR_ROUTE_ENV`) | *"the host GR channel's ring **and** its `GP_PUT` are both ours … the armed arm buys the **transport**, not execution"* | refuted at `shim.rs:5745` by the w267 birth lines; the correction was never propagated into the flag's own doc, which is the doc a reader hits first |
| `shim.rs:10540` (`GUEST-RING` banner) | *"⊘ Supply side only: the host channel still declares OUR ring and OUR USERD"* | false since leg A2; the boot's own `GR-BIRTH` line two screens later contradicts the banner it printed |
| `fwd/lib.rs:6674` (`parse_pushbuffer`) | *"Only runs where the core is already the mediator … A userspace ring never carries a fact the core must extract"* | `kayfabe_fwd::parse_pushbuffer` has **zero** production callers; `SharedDevice::parse_pushbuffer` runs per-doorbell on every forwarded `Ce` channel including user procs |

⚠ This is the fourth rung in a row redirected by a doc comment that stopped being true and did
not say so. Three of the four above are **in the file the reader is told to read first**.

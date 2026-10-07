# w282 / RESULT — THE HOST COPY ENGINE MOVED THE GUEST'S BYTES, AND THE GUEST READ THEM BACK

**STATUS: LIVE — 2026-08-13.** Two runs, six boots, real GA106 / `580.159.04`.

| run | revision | arms | gates |
|---|---|---|---|
| `w282` | `5f0a6e3fcd79dc3ad54505ecee7b924306944941` | `client` / `clientoff` / `cup2` | stamp PASS, tree clean, `cap2b` guard 0, `ENOSPC_LLVM=0`, `EXIT rc=0` |
| **`w282b`** | **`f07c75a1d7af51f69068e176b780ededdb7029ac`** | `client` / `clientoff` / `cup2` | same, all PASS |

`GUEST_MD5` equal to the native md5 on every arm, `total=53 failed=0`, six carried arms PASS.
Every number below was read from an artefact opened in this session.

---

## ★★★★★ LEAD — CRITERION 2 IS MET. THE BYTES MOVED, ON REAL HARDWARE, AND THE GUEST SAW IT.

`w282b_client`, verbatim, from the guest's own client and from the isolate:

```text
CE-SUBMIT dst=0x120010000 len=4096 by=HostCe gp_get=1 gp_put=1
          sem=0x00000001 want=0x00000001 → RETIRED
HOST_DMESG_XID=0        faulted @ = []

R33 arm 1 COPY = dst[0]    0x3f0011cc -> 0xc0ffee33  (want 0xc0ffee33)   ★ MOVED
                 dst[last] 0xc0fff232               (want 0xc0fff232)   ★ MOVED
                 semaphore 0x00000000               (want 0x00000001)   ⊘
                 GP_GET 0 GP_PUT 1                                      ⊘
```

**The destination is read back out of the guest, at both ends, and both ends carry the values the
guest asked for.** ⊘ Not inferred, not a count, not our own read of our own store — the guest's
own process read its own buffer after the copy.

★ **The known-positive ran on bare metal from the SAME binary, minutes before each pair**
(`xid_w282_native.log`, `xid_w282b_native.log`), and passes all three:

```text
★ R33 arm 1 COPY = 4096 bytes moved: dst[0] 0x3f0011cc -> 0xc0ffee33, dst[last] 0xc0fff232,
    engine semaphore 0x00000001 (declared 0x00000001), GP_GET 1 caught GP_PUT 1
    — read back through an INDEPENDENT mapping (its own device node, its own mmap)
```

⇒ a guest-side failure is never ambiguous between *"we built it wrong"* and *"the guest path is
broken"*. **The one criterion the guest now meets, the native arm meets the same way.**

### The three-fact falsifier, as pre-registered, on IDENTITY

| pre-registered | measured |
|---|---|
| `CE-SUBMIT` must still read **`by=HostCe`** | ★ **`by=HostCe` = 1, `by=Ours` = 0** |
| `#255` must go **FIRED → QUIET** | ★ control **`FIRED`** naming both VAs; armed **`QUIET`** naming both host VAs |
| the Xid gone **or moved** | ★ **`HOST_DMESG_XID=0`, `faulted @ = []`** |

⊘ **All three, together.** `w281b`'s falsifier fired on a count while the thing counted was
substituted underneath it; this one cannot, because every row names an address or a key.

---

## ★★★ WHAT ACTUALLY FIXED IT — and it took TWO iterations, the second found by the first boot

### Iteration 1 (`w282`, `5f0a6e3`) — leg 7: **the join was never CALLED on a CE doorbell**

⊘⊘ **The premise was already built.** `join_one_fb_leaf` — the four-step join — has been in the
tree since `w260`, and its own doc says `what` names *"an operand's name, or the channel's own
ring"*. `Regs::back_census_framebuffer_leaves` already drives it off an **operand census**.
★ What it had never covered is a **CE doorbell**: that caller hangs off
`declare_gr_completion`, which `SharedDoorbell::ring` calls on the two **GR** dispositions and on
**no CE path at all**. A CE operand instead reached `pin_operand_guest_ram`, which refuses a
framebuffer binding **by name** with the sentence

> *"An operand that binds in the framebuffer is a real and served case — that memory is ours
> already and needs no descriptor"*

— true of the CPU executor, and **measured false of a host engine** at `w281`.
⇒ Leg 7 (`KAYFABE_OPERAND_JOIN`) presents them. It adds **no primitive, no verb, no authority**.

Result, `w282_client`: both leaves joined, `placed_as_asked=true`, `#255` QUIET, `by=HostCe`
held — **and `Xid 31 … FAULT_PTE @ 0x1_20010000`, the same VA, UNMOVED.**

### ★★★★★ Iteration 2 (`w282b`, `f07c75a`) — **the join placed the object in ONE address space and the engine runs in ANOTHER**

The first boot's own evidence named it: the table said host-backed at the guest's own VA and the
engine still could not reach it. `ce_copy_outcome` states the invariant in its own comment
(`crates/kayfabe-isolate-host/src/rm.rs:5333`):

> *"W229 — the CE channel is built in the isolate's **OWN** address space, never in `vas`. `vas`
> still names the space the OPERANDS live in, and **`map_dma_both` has placed them at the same
> addresses in both**, which is why `src`/`dst` below need no translation."*

**`join_fb_leaf` did not go through `map_dma_both`.** It called `raw_map_dma` on the
guest-facing range **alone**. The object existed, at the right VA, **in the wrong space**.

⊘ **The ring's leaf never exposed this**, and that is why it survived five boots: *we* decode the
ring in the VMM and hand `ce_copy` explicit `src`/`dst`, so nothing an engine dereferences ever
came out of a joined object. **An operand is the first joined object a real engine walks for
itself.** ⇒ `w229`'s finding one plane over, and `w229`'s fix: place it twice, at one address,
all-or-nothing. **One line.**

---

## THE DIFFERENTIAL — ONE VARIABLE, and the control reproduces `w281b` exactly

| | `w282b_clientoff` (`assert`) | **`w282b_client`** (`join`) |
|---|---|---|
| `OPERAND-JOIN-TABLE` | `2 asked, 0 MISS, 0 guest RAM, 0 already, **2 CANDIDATE(S)**` `[va=0x120000000:Vidmem@0x10000/FakeFramebuffer va=0x120010000:Vidmem@0x20000/FakeFramebuffer]` | **identical** |
| leaves | `JOINED 0 leaf/leaves … over 2 distinct` | `JOINED 2 leaf/leaves, 0 REFUSED` — `fb_phys=0x10000` → `host_va=0x120000000`, `fb_phys=0x20000` → `host_va=0x120010000`, `placed_as_asked=true`, establishment copy `4092` / `3072` non-zero bytes |
| `#255` | ★ **FIRED**, naming both VAs | ★ **QUIET**, naming both host VAs |
| `CE-SUBMIT` | `by=Ours src=Address(4831838208)` → **REFUSED BEFORE SUBMISSION** `Other(19270)` | `by=HostCe` → **RETIRED**, `sem=0x1 want=0x1` |
| host Xid | 0 | **0** |
| `dst[0]` in the guest | `0x3f0011cc` (unchanged) | ★ **`0xc0ffee33`** |

⊘ The two leaf `fb_phys` values are **not** the ring's (`0x40000`) — `w281b`'s single adopted
leaf. Three distinct leaves are now joined and they are told apart by address.

---

## ⊘⊘ TWO OF THREE CRITERIA ARE NOT MET, AND THE REASON IS ONE THING WITH NO BUILT MECHANISM

- **(1) `GP_GET` catches `GP_PUT`** — ⊘ **NO.** `GP_GET 0 GP_PUT 1` in the guest's own USERD.
- **(3) the semaphore carries the declared payload** — ⊘ **NO.** `0x00000000`.

⚠ **THE TRAP, NAMED BEFORE ANYONE READS IT WRONG.** `CE-SUBMIT` says `gp_get=1 gp_put=1` and
`sem=0x00000001`. **Those are the HOST channel we forwarded onto, not the guest's.** The guest's
own cursor and the guest's own semaphore page are untouched. Same class as `w281`'s trap and as
`a_count_cannot_see_a_substitution`.

**Why:** we forward a *decoded* `ce_copy` verb (`src`/`dst`/`len`); the guest's own
`SET_REPORT_SEMAPHORE` method is not carried, and `ce_copy` releases to **its own** host
semaphore (`sem_va = parts.ring_va + SEMAPHORE_OFFSET`). And the guest's `GP_GET` is advanced by
hardware only on the guest's own channel, which never ran. **Measured: there is no writer for
either, anywhere in the tree** — `completion_watch`'s module doc says so in its own words:
*"This module has **no writer**."*

### ⇒ THIS IS THE RULING QUESTION, AND I AM STOPPING HERE RATHER THAN IMPROVISING — see §NEXT

---

## `cup2` — THE OWNER'S QUESTION, ANSWERED, AND LEG 7 IS **NOT ON ITS PATH**

`^CUP2_RC=` **ANCHORED** (`GCC_RC=0`, so the workload built and ran): **`CUP2_RC=TIMEOUT`** on
**both** `w282_cup2` and `w282b_cup2` — the hook's own deadline, ⊘ **not** `124` and ⊘ **not** a
pass. ★ Reproducible across two revisions, including the one that made the raw client's copy
retire on hardware.

★ And leg 7 ran on it — **68** `OPERAND-JOIN-TABLE` lines — which is how we can say *why* it does
not help, rather than assuming:

```text
64 ×  OPERAND-JOIN-TABLE: 33 page(s) asked, 0 MISS, 32 in guest RAM (leg 6's population,
      untouched here), 1 ALREADY JOINED, 0 CANDIDATE(S) in the emulated framebuffer
```

⇒ **Every one of `cup2`'s CE operand pages is in GUEST RAM**, already served by leg 6's pin
(`ALREADY PINNED (idempotent replay; fully covered) … placed_as_asked=true`), and the single
framebuffer page it names (`va=0x2000a8000`) is **already joined** by the GR census path.
**Zero candidates. Leg 7 has nothing to do on `cup2`.**

`cup2`'s walls are elsewhere and both are visible:
- its 68 `CE-SUBMIT`s are **`src=Constant(0)`** — *fills/scrubs*, not copies — and all route
  `by=Ours` and are refused before submission;
- its host Xid is on the **graphics** plane: `Xid 31 ENGINE GRAPHICS HUBCLIENT_FE faulted @
  0x7077_28e00000, FAULT_PDE`.

⊘ So *"the operand join will move `cup2`"* is **refuted, with the reason measured**, and this is
the arm of my pre-registration that fired (H9 was pre-registered as *not* foregone).

---

## ★★★ THE OWNER'S FOUR RULINGS — where each stands

### 3. PER-VAS RESOLUTION — verified, **and now asserted so it cannot regress**

**The `promote.rs:968` worry is answered and it is benign.** That comment's *"non-per-VAS arm"*
is a **physical-descriptor** share for exactly three GPU-wide `buffer_id`s
(`FECS_EVENT`/`PRIV_ACCESS_MAP`/`UNRESTRICTED_PRIV_ACCESS_MAP`, `PhysHalfScope::PerGpu`). Its
payload is `GlobalPhysHalf { phys, len, aperture }` — **there is no VA field and there can never
be one**, the `va` always comes from the *local* half in *this* VAS, and the scope predicate is
re-tested at the point of use. ⇒ **operand resolution cannot reach it.**

Traced end to end, operand resolution is per-VAS **three independent times**: the VAs come from
this channel's own ring read through this channel's own root; each resolves through
`SharedDevice::resolve` keyed by this channel's `Pdb`; the leaf is walked from the same root and
bound into the same `Pdb`. Every `.table` access outside `kayfabe-mmu` reaches it via
`vases.get(&(gpu, pdb))`. There is no `owner_of(addr)` and `kayfabe-mmu/src/gpga.rs` says there
never will be.

⊘ **But it held by which INSTANCE the caller happened to hold, not by anything checked** —
`AddressTable::resolve(pdb, va)` used `pdb` **only to label the fault**. ⇒ **Fixed:**
`AddressTable` now carries its owner `Pdb` (`owned_by`, set at `Vas::new`) and **both entrances**
refuse a foreign one by name (`AddressFault::TableIdentity`, its own `FaultTag`, adjudicated by
the compiler's exhaustive match). ★ A foreign **VA** still gets `Miss` — *not found*, never
*denied*, which is the owner's own words.

### 4. CROSS-PROCESS ISOLATION — not weakened

A join lives in one `(ProcId, GpuId, Pdb)`'s table and is mapped in that isolate's host VASes. A
second guest process is a different `ProcId` ⇒ a different isolate ⇒ different host VASes. The
missing release (below) is a **resource** leak, not a **boundary** leak.

### 2. CLEANUP — designed, **not wired**, and the design is `docs/design/operand_join_lifetime.md`

### ⇒ THE `ogkm` ANSWER — the owner's specific ask

- **Normal free:** `NV_ESC_RM_FREE` = `_IOWR('F', 0x29, NVOS00_PARAMETERS)`
  (`nv_escape.h:33`, `escape.c:503`), `NV_ESC_RM_UNMAP_MEMORY` `0x4F`,
  `NV_ESC_RM_UNMAP_MEMORY_DMA` `0x58`, UVM `UVM_FREE` **34** / `UVM_UNMAP_EXTERNAL` **66**.
  ⚠ `UVM_UNMAP_EXTERNAL_ALLOCATION` / `UVM_MEM_UNMAP` **do not exist** in 580.159.04.
- ⊘ **The userspace free is BEST-EFFORT.** On `SIGKILL`/OOM userspace issues **nothing**.
- ★★★ **The guest KERNEL's teardown is GUARANTEED.** Linux always calls `.release`;
  `RmFreeUnusedClients` says so in its own comment (`osapi.c:545`); the single funnel
  `clientFreeResource_IMPL` (`rs_client.c:785`) **manufactures** the unmaps userspace never
  issued — CPU unmaps, DMA unmaps, real PTE writes, a real TLB invalidate. UVM fires *earlier*,
  on `mm` teardown (`uvm_va_space_mm.c:328`), including a page-directory write to hardware.
- ★★ **What we see as the faked GSP:** `NV_VGPU_MSG_FUNCTION_FREE` = **10**, plus PTE clears.
  ⊘ `DMA_FILL_PTE_MEM` is a **compiled-out no-op** (`rpc_vgpu.h:42`) and `UNMAP_MEMORY_DMA` is
  `_STUB` on **every** GSP-era chip (`g_rpc_private.h:634`) — the unmap is a **PTE store and an
  MMU invalidate, not a message**. `UNLOADING_GUEST_DRIVER` (47) is **unload-only**.
- ⇒ **Event-driven unjoin IS viable — but the event is the guest KERNEL's teardown, never a
  userspace free — and three deferral queues can delay it without bound.** ⇒ **T1** (the binding
  is dropped) precise, **T2** (the VAS dies) the backstop that covers `SIGKILL`, **T3** (device
  reset) already built. ⊘ **No refcount**: overlap is refused, not shared, so a leaf has exactly
  one join and one owner.

---

## ⊘⊘ AND THE FIRST RUN'S CONTROL FOUND A DEFECT IN MY OWN INSTRUMENT

`w282_clientoff` printed **zero** `#255` lines, because the first draft put the assertion inside
the armed path. ⇒ on the control, `QUIET` and *"the instrument never ran"* were **the same
observation**, and the guaranteed known-positive the brief handed me was **unreachable**.
★ Fixed with a **third arm**: `assert` classifies and states `#255` and **joins nothing**, so the
control's expected reading is `FIRED` — a **positive** observation rather than an absence. It
then fired: `FIRED = 1`, naming both VAs. ⚠ Caught by a control, not by reading.

---

## OPTION (c) — WHY I DID NOT CHOOSE IT

Steering the operands to sysmem is **not available as a device-side lever**: we are not the
allocator. `NV01_MEMORY_SYSTEM` / `NV01_MEMORY_LOCAL_USER` reach us **zero** times across every
committed boot (`gpga_region_kind.md` §0.1) — the guest's stock RM allocates out of its own heap
over the framebuffer we advertise. The only device-wide lever is `fb_length` → `PDB_PROP_GPU_ZERO_FB`,
which is a different (and unmeasured) product. For the raw client it would mean changing **our own
test binary** — and `alloc_sysmem` asks for `MAPPING_NO_MAP`, which would break criterion 2's
CPU read-back. ⇒ it dodges the question for one workload and answers it for none. **The join is
the fix; it is also a defect fix, exactly as the owner ruled.**

---

## ⊘ NOTHING WAS RELAXED

`ce_copy(Ours)` is untouched — `by=Ours` still refuses by name on the control, and the armed arm
never takes that branch. No passthrough was widened, no isolation boundary moved, no completion
was forged, and nothing here needs root.

---

## ★★★ THE NEXT ONE FACT — and it needs a RULING, not a patch

**Criteria 1 and 3 are one thing: nobody writes the guest's completion.** The tree's standing
refusal is `completion_watch`'s, and its wording is **conditional**:

> *"the payload is a literal immediate in the guest's own bytes, so writing it here **without
> running the work** is precisely the credit-shortcut the C artifact named and refused."*

**We now run the work**, and `ce_copy` returns only after `await_semaphore` observes the *host
engine's own* release. So the precondition that refusal names is, for the first time,
**falsifiable per submission**. Two options, and I did not choose between them:

1. ★ **Let hardware write it.** Carry the guest's declared release (`sem_releases` is already
   parsed: `Vec<(GpuVa, u64)>`) into the forwarded push as a second `SET_SEMAPHORE_A/B/PAYLOAD`,
   so the **real engine** writes the guest's semaphore at the guest's VA. ⊘ No forgery by
   construction. ⚠ Needs the guest's semaphore page reachable in the executor VAS — which
   `w282b`'s one-line fix now gives it, since the semaphore lives in the joined ring leaf.
2. **Write it ourselves on a witnessed retire**, and advance the guest's `GP_GET`. ⚠ `GP_GET`
   has **no** hardware route on our own host channel, so criterion 1 needs either this or a host
   channel born over the guest's USERD (`AdoptedGuestUserd`, built for GR, measured at `w267`).

⚠ Option 1 cannot reach criterion 1. Option 2 reaches both and is a CPU write of a completion.
**That is the owner's call.**

## ⊘⊘ WHAT THESE RUNS CANNOT PROVE

- **It is not a pass.** One of three criteria. It is the *hardest* one and it is a **stage**.
- **The completion plane still has no oracle** — `sem=0x00000001` is the isolate's read of the
  **host** channel's semaphore, not an independent witness of the guest's.
- **The join is not released.** Every joined leaf lives for the life of the `Vas`. §5 of
  `operand_join_lifetime.md` states exactly what leaks.
- **`cup2` is untouched by this rung**, measured rather than assumed, and `CUP2_RC=TIMEOUT`.
- One workload, one chip (GA106), one driver (`580.159.04`), one boot per arm.

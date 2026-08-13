# LEG B — the guest's USERD address WAS on the wire the whole time

> ### STATUS — 2026-08-12 / **LIVE — SOURCE-MEASURED, NOT YET HARDWARE-MEASURED.**
> Supersedes the *blocker* halves of three documents; see §0. Read `§1` before citing any of
> them. ⚠ **Everything here is read off `ogkm-580.159.04` and off this port's own decoders.
> No boot has yet shown a non-`UNREADABLE` `phys=` token.** The pre-registration
> (`leg_b_userd_at_creation_prereg.md`) is what closes that, and until it does, §3's
> `[NOT MEASURED]` markers are load-bearing.

---

## 0. ⊘⊘ WHAT THIS REFUTES — three documents, one shared premise, and the premise is false

| document | its claim | verdict |
|---|---|---|
| `nvidia-gpu-passthrough/docs/design/userd_is_not_the_ring.md` §3 (2026-08-11) | *"USERD never goes through a VA … A VA-keyed crossing cannot back a physically-named object"* → USERD needs *"a **different** crossing"* | **the diagnosis is right and the conclusion is wrong.** USERD is indeed named physically. That is not an obstacle; it is the answer, because the guest **sends us the physical address**. |
| `docs/design/leg_b_userd_adoption_blocker.md` §1 (2026-08-12) | *"the blocker is an ADDRESS WITH NO PRODUCER"*; `AllocFacts::mem_phys` has no producer, and *"there is no page-table walk that can find it"* | **the producer exists and is not `mem_phys`.** No page-table walk is needed. |
| `traces/boots/w262/RESULT.md` §5 (2026-08-12) | *"USERD is named by handle+offset … never by a VA — so the page-table walk that gave the ring its second join source **cannot** be pointed at USERD. That, and not `mem_phys`, is what a leg-B rung has to solve."* | **nothing had to be pointed at USERD.** |

★★★ **All three looked for a VA.** They were right that there is none, right that the guest's
handle is meaningless to us, and right to refuse a `UserdSource::Guest(handle)` arm. The shared
step none of them took is: *ask what the **guest's own kernel** does with the handle before it
talks to us.*

### 0.1 The answer, from the driver's own source

`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2747-2757`, inside
`_kchannelSendChannelAllocRpc`:

```c
// Fill the userd memory descriptor
if (pKernelChannel->pUserdSubDeviceMemDesc[subdevInst])
{
    pRpcParams->userdMem.base =
        memdescGetPhysAddr(pKernelChannel->pUserdSubDeviceMemDesc[subdevInst], AT_GPU, 0);
    pRpcParams->userdMem.size = pKernelChannel->pUserdSubDeviceMemDesc[subdevInst]->Size;
    pRpcParams->userdMem.addressSpace =
        memdescGetAddressSpace(pKernelChannel->pUserdSubDeviceMemDesc[subdevInst]);
    ...
}
```

CPU-RM resolves `hUserdMemory[0]` **locally, before the RPC** — it must, because GSP has no
client handle namespace to look one up in (`kernel_channel_gv100.c:184-187` does the lookup in
`RES_GET_CLIENT_HANDLE(pKernelChannel)`). ⇒ The physical address arrives at our fake GSP in the
**same `NV_CHANNEL_ALLOC_PARAMS` buffer** this port already decodes twice
(`ChannelUserdWire` @ +32/+64, `ChannelEngineWire` @ +128).

★ And `userdOffset[0]` is **already folded in**: `pUserdSubDeviceMemDesc` is a *sub*-memdesc
created at that offset — `memdescCreateSubMem(&pUserdSubMemDesc, pUserdMemDescForSubDev, pGpu,
userdOffset, userdSize)`, `ogkm-580: kernel_channel_gv100.c:234-237` — and the RPC reads
`memdescGetPhysAddr(..., 0)` of *that*. So `userdMem.base` is the address of **this channel's own
512-byte slot**. Nothing may be added to it.

### 0.2 ⊘ What survives from all three documents, unchanged

- **Do not build a `UserdSource::Guest(handle)` arm.** All three refused it; all three were
  right. The guest's handle is still meaningless to host RM. What replaced it is not a handle
  forward — it is a **host object over the same bytes**, addressed by offset.
- `AllocFacts::mem_phys` still has no producer. It is still irrelevant.
- `userd_is_not_the_ring.md` §1's driver survey — *RM permits a client-supplied USERD, no
  aperture gate, `OsDescMemory` explicitly special-cased, 512 B / 512 B-aligned / non-VPR /
  physical < 2^40* — is unaffected and is what makes §2 legal at all.
- `userd_is_not_the_ring.md` §4's cost — **a passthrough USERD cannot be CPU-mapped, so we lose
  our `GP_GET` read** — is unaffected and is now the live consequence. See §4.

---

## 1. ⚠ THE OFFSET, AND WHY IT IS NOT A FRESH COUNT

`NV_MEMORY_DESC_PARAMS` is 24 bytes (`alloc_channel.h:37-42`). The tail of
`NV_CHANNEL_ALLOC_PARAMS` (`:296-342`) runs:

```
+128 engineType  +132 cid  +136 subDeviceId  +140 hObjectEccError
+144 instanceMem  +168 userdMem  +192 ramfcMem  +216 mthdbufMem
+240 hPhysChannelGroup  +244 internalFlags  +248 errorNotifierMem
```

⊘ **The last two numbers were already pinned in this tree and are already exercised on the
bench**: `ChannelNotifierWire::V580 { internal_flags: 244, error_notifier_mem: 248 }`. So
`userd_mem = 168` is `internal_flags - 76`, not a fourth hand count, and
`userd_mem_is_where_the_notifier_wire_says_it_is` asserts exactly that in both directions
(against the notifier wire below it, against the engine wire above it) for 580 **and** 610.

★ This matters because a wrong offset here is **not** a loud failure. It would read four
plausible words out of `ramfcMem` or `instanceMem` — real addresses, in the right apertures,
of the wrong objects — and the containment check in §2 would happily pass on some of them.

---

## 2. WHAT LEG B ACTUALLY IS — a CONTAINMENT test, not a resolution

The guest's `userdMem.base` is an offset in the **emulated** framebuffer. It means nothing to
host RM. It becomes usable exactly when it lies inside a framebuffer leaf we have already
joined, because the join gives us a host `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` whose byte *i* **is**
emulated-framebuffer byte `phys + i` (`HostRmBackend::join_fb_leaf`, `table.install(phys, len,
at, region)`).

```
adopted_guest_userd(chan):
    fb        = chan.facts.userd.resolved   as UserdMem::Framebuffer { base, .. }   else None
    (start,len,binding) = table.binding_at(chan.facts.gp_fifo_ring.va)              else None
    binding.host().bytes() == JoinsGuestWindow                                      else None
    binding.aperture()     == Vidmem                                                else None
    off = fb - binding.phys()                        # containment, not a search
    off + 512 <= len                                                                else None
    ⇒ hUserdMemory[0] = binding.host().memory(),  userdOffset[0] = off
```

### 2.1 ★★★ This is NOT reverse resolution, and the distinction is the whole licence

`kayfabe-mmu`'s `gpga.rs` forbids `fn owner_of(addr)` *"and there never will be"*, and
`leg_b_userd_adoption_blocker.md` §2.2 refused the BAR1 route on exactly that ground.

⊘ **Nothing here asks who owns an address.** The chain is entirely forward: the channel names its
ring VA → the ring VA names a binding → the binding is a joined object with a known framebuffer
base and length. The only question asked of the guest's number is *"is it inside the object I am
already holding"*, which is a **containment test on a named object**, not a search over
addresses. Had the ring's leaf not been joined, leg B declines — it does not go looking.

### 2.2 ⚠ Why the RING's binding, when the object is USERD's

Because the arming must be **inherited**, exactly as leg A2's is
(`a_second_source_of_truth_beside_a_complete_value`). The address table holds a
`JoinsGuestWindow` binding at a ring VA **only** when `adopt_joined_fb_leaf` ran, and that runs
only when the shell armed the supply side. Keying leg B off the same binding makes a disarmed
build `None` **by construction** and makes it impossible for leg B to fire on a channel whose
ring leg A did not adopt — which is the state that would hand RM a guest cursor for a ring RM is
not reading.

⊘ **This is a scope limit and it is deliberate.** A channel whose USERD is in a *different* leaf
from its ring declines, even though the bytes may well be reachable. `[NOT MEASURED]` how often
that happens; on `w262b` the sixteen walling channels' rings and USERDs share one 2 MB leaf
(`LEAF@0x200200000->0x1000000/Vidmem/sz0x200000`, ring VAs `0x200200000 + n*0x3000`), so the
expectation is *never on this workload* — an expectation, not a measurement.

---

## 3. WHAT IS UNMEASURED, STATED BEFORE THE BOOT

- ⊘ `[NOT MEASURED]` **that the params reach +188 at all.** The decode is additive and answers
  `phys=UNREADABLE` if they do not. The prior is strong but indirect: the *error notifier* decode
  needs **268** bytes and is in production, and `ChannelEngineWire` reads +128. Neither has been
  shown to return `Some` on this bench in a committed log.
- ⊘ `[NOT MEASURED]` **that the guest's walling channels declare `ADDR_FBMEM`.** They CPU-map
  their USERD through BAR1, which is the framebuffer path, so `ADDR_FBMEM` is expected —
  `ADDR_SYSMEM` is legal and is refused **by name**, not by absence.
- ⊘ `[NOT MEASURED]` **that host RM accepts a non-zero `userdOffset[0]`.** `rm.rs`'s own comment
  records `userd_offset_0 = 0x2000` being bitten on RTX 3090 / 580.159.04 on 2026-07-30, with
  every ioctl returning 0 and `GP_GET 0 GP_PUT 1`. ★ Read what that measured: the offset was
  moved while our *store* stayed at +0, so RM read a slot nobody wrote. It is a **consistency**
  measurement, not evidence that RM mishandles the field — and the C's own M5.47 fix went the
  other way (`zero userdOffset[0] on forwarded channels — HOST NOW FETCHES`). Leg B moves the
  reader and the writer together, which that boot did not. ⚠ It remains the single most likely
  place for this rung to produce a silent `GP_PUT == GP_GET`.
- ⊘ `[NOT MEASURED]` **the ordering, for the group that matters.** `w262b` §4.2 established that
  all sixteen `GUEST-RING` births precede the first advance on every `0x3000`-stride page — and
  §4.3 established that the instrument has a false-positive class (a ring page whose 18th entry
  pair lands at `+0x8c`). Adoption at **creation** is what makes the ordering question moot, and
  it is why this rung does it there. See §4.
- ⊘ `[NOT MEASURED]` that any of this moves `CUP2_RC`.

---

## 4. ★★★ THE COST, INHERITED FROM `userd_is_not_the_ring.md` §4 AND NOW LIVE

RM **zeroes** a caller-supplied USERD (`[measured 2026-08-11, R32/w233, real GA106]`), which is
why adoption is at channel **creation** and may never move to the first doorbell: adopting late
wipes the cursor that caused the doorbell, returns `NV_OK`, and produces silence.

And a guest-backed `OS_DESCRIPTOR` **cannot be CPU-mapped** (`[measured, R31 arm B]`,
`NV_ERR_NOT_SUPPORTED`; driver's own `memMap_IMPL: CPU mapping not supported for addressSpace:
0x1`). So on an adopted arm:

- `submit_entry`'s `GP_PUT` store must not happen — and must be **refused by name**, not
  skipped. That is the point of leg B: the guest's own store through BAR1 lands in those bytes.
- ⊘ **`GP_GET` is no longer readable by us.** `userd_cursors` works through a CPU mapping that
  does not exist on this arm. The replacement is R32's **J2** (GPU-write → CPU-read through a
  described memfd), which `[measured 2026-08-11, f58473f]` **HOLDS** — 65536/65536 bytes read
  back through the other mapping, with the negative control firing. ⇒ The mechanism exists;
  ⊘ **it is not wired here.** Reading `GP_GET` off the joined leaf is the next rung, and until
  it exists a green boot cannot distinguish *"the ring was adopted"* from *"the ring was
  fetched"* — `admitted_and_served_are_different_gates`.

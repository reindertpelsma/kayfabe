# w290 — THE PARKED-HALF LEAD IS REFUTED, AND THE HOST VAS IS EMPTY

Boot `w290cup2` @ `c2b2a22`, stamp gate PASS. **⊘ RELAXED ARM, labelled:**
`KAYFABE_PT_SWEEP=on` + `KAYFABE_OPERAND_JOIN=join`, byte for byte the `w289cup2` arming, so
the fault reproduces by identity. A relaxed green is never the milestone; this arm is not
green anyway.

```
CUP2_RC anchored   = [CUP2_RC=1]      baseline 1. UNCHANGED.
unanchored         = [CUP2_RC=0 CUP2_RC=1]     ← would have reported 0 on a failing arm
Xid 31 ENGINE GRAPHICS HUBCLIENT_FE faulted @ 0x75c0_eee00000 FAULT_PDE ACCESS_TYPE_VIRT_WRITE
```

Same fault as `w289cup2` by identity; the address differs (`0x7ff6_a6e00000` → `0x75c0_eee00000`)
because under UVM unified addressing the GPU VA **is** the process VA and the process is ASLR'd.
That is **predicted**, not suspicious.

---

## ⊘⊘ LEAD WITH THE CONTRADICTION — the brief's premise is measured differently on the boot it is about

The brief carried **`joined=0 parked=5`** with **`bid=0x3 = {phys=0, va=2, complete=0}`**. Those
numbers are from **`w289g`, the raw-CE-client boot**. cup2's own boot — the one that produced the
`0x7ff6_a6e00000` fault the brief quotes — reads, and `w290` reproduces exactly:

```
CUMULATIVE bound=8 joined=4 joined_global=1 already=7 globals_added=1
last ACCEPTED: parked=0 half_already=9 half_unusable=0 orphans(awaiting_va=0,awaiting_phys=9)
TALLY: {bid=0x3 phys=0 va=10 complete=0}
```

⇒ **`joined=0` is FALSE here — the join works on this boot (4 joins, 1 GPU-scoped).** And the park
is **NINE**, not five. Both halves of the premise differ on the boot in question.

---

## 1. THE NINE PARKED HALVES, BY IDENTITY

```
[proc=2 gpu=0 pdb=0x201000 parked=9
  {bid=0x0 AwaitingPhysical va=0x203fb0000}   MAIN
  {bid=0x1 AwaitingPhysical va=0x20409d000}   PM
  {bid=0x2 AwaitingPhysical va=0x2040a6000}   PATCH
  {bid=0x3 AwaitingPhysical va=0x20409a000}   BUFFER_BUNDLE_CB
  {bid=0x4 AwaitingPhysical va=0x203f10000}   PAGEPOOL
  {bid=0x5 AwaitingPhysical va=0x203400000}   ATTRIBUTE_CB
  {bid=0x6 AwaitingPhysical va=0x203f30000}   RTV_CB_GLOBAL
  {bid=0x9 AwaitingPhysical va=0x203e80000}   FECS_EVENT
  {bid=0xb AwaitingPhysical va=0x203e00000}   UNRESTRICTED_PRIV_ACCESS_MAP
  bound=4=[0x203e90000,0x203fb0000,0x20409d000,0x2040a6000]]
```

★ **The instrument's own known-positive fired on this boot**: `bound=4=[…]` is non-empty on all
88 emissions beside 264 `bound=0=[]` rows, so *"everything is parked"* is a reading this
enumerator **can fail to produce**. Offline control:
`tests/tests/promote_ctx.rs::the_parked_half_census_names_the_id_the_half_and_the_address`.

⊘ Note `bid=0x0/0x1/0x2` appear in **both** lists at the **same VA**: the first GR channel's
promotion joined and bound them, and the seven later channels of the same TSG re-declared the VA
half against a physical already consumed, so it re-parked (`half_already=9`). A park is not
necessarily an unbound VA — which is exactly why the row prints the address and not a count.

## 2. ★★★★★ **PARKED-HALF-COVERS-FAULT = NO** — and it is not close

Every parked VA is in `0x2034_00000 … 0x2040_a6000`, UVM's low channel-resource window. The fault
is at `0x75c0_eee00000` — **~500 GB away**, in a different region entirely. No parked half, and no
promote binding, has anything to do with this fault.

⇒ **THE LEAD IS RETIRED.** `joined=0 parked=5 ⇒ FAULT_PDE` was a shape match, not a mechanism.

## 3. WHAT DOES OWN THAT VA — three pictures of one address space

```
GUEST-DESCRIBES [proc=2 pdb=0x201000 … 0x75c0ee000000+0xc00000, 0x75c0eee00000+0x400000]  OWNS-FAULT = YES
TABLE-DESCRIBES [proc=2 pdb=0x201000 rows=16425 … 0x75c0eee00000+0x400000]                OWNS-FAULT = YES
HOST-PUBLISHED  [proc=2 pdb=0x201000 host_rows=4 of 16425 runs=3
                 0x200000000+0x400000, 0x10000000000+0x200000, 0x10002000000+0x200000]    OWNS-FAULT = NO
```

★★★★★ **`host_rows=4 of 16425` — 0.024 %.** The other two address spaces are worse:
`[proc=0 pdb=0x200000 host_rows=0 of 533]`, `[proc=0 pdb=0x2efa9c000 host_rows=0 of 6254]`.
**The host VAS the GPU actually walks is empty.** The three published runs are the CE operands the
relaxed `OPERAND_JOIN=join` arm put there, and nothing else has ever been published.

★★ **And that is why it is `FAULT_PDE` and not `FAULT_PTE`.** The nearest published address to
`0x75c0_eee00000` is `0x100_02000000` — over a terabyte away, a different entry at every level of
the descent. There is no page **directory** over the faulting region at all, so hardware misses
above the leaf. A leaf-level story could never have produced this fault code.

## 4. ⊘ COMPLETING THE JOINS WOULD NOT HAVE HELPED — refuted at the source, not by a boot

`apply_promote_ctx` binds a joined range with `Binding::declared_by_guest`, and its own comment
says why that is the truthful call (`kayfabe-core/src/promote.rs:1180-1195`):

> "no `HostBacking` reaches this site … the *gap* is that the host object this range was allocated
> from is not carried to the bind."

⇒ A completed join produces a table row with `host: None` — **it puts nothing in the host VAS**.
Brief step 4 ("if a join would complete the descent, complete it") is void: no promote join can
complete a host-side descent.

## 5. ⊘ AND RM WILL NEVER SEND THE MISSING PHYSICALS — `PhysHalfScope::Never` CONFIRMED

`kgrctxPrepareInitializeCtxBuffer_IMPL` reaches its entry-emitting tail (`gpuPhysAddr`/`size`/
`bNonmapped=1`, `kernel_graphics_context.c:1845-1852`) only if the switch assigned a `pMemDesc`.
BUFFER_BUNDLE_CB / PAGEPOOL / ATTRIBUTE_CB / RTV_CB_GLOBAL / GFXP_POOL share **one** arm ending
`// No initialization from kernel RM; return NV_OK` (`:1748-1758`), and GLOBAL_PRIV_ACCESS_MAP has
its own at `:1803-1805`. `*pbAddEntry` stays `NV_FALSE` (`:1704`).

**UVM maps those buffers itself**: `uvm_user_channel_map_resources` → `uvm_va_range_map_rm_allocation`
(`uvm_map_external.c:375`) pulls PTE **bits** from `nvUvmInterfaceGetChannelResourcePtes` and writes
them into its own page tables with `uvm_pte_batch_single_write_ptes` (`:214-253`), and only then
tells RM the VA (`uvm_user_channel.c:438-439, :676, :686`; RM disclaims it at
`kernel_graphics_context.c:1883-1885`). That is precisely why the parked ids appear in
`GUEST-DESCRIBES` and why **no physical half is ever coming for them**.

---

## ⇒ THE SUCCESSOR, NAMED BY MEASUREMENT

The wall is **publication**, not population. Our table is right (`TABLE-DESCRIBES` owns the fault);
the host VAS is empty (`HOST-PUBLISHED` does not). The verb already exists and already works — it
is what the relaxed operand arm used: `SharedDevice::back_fb_leaf` via
`join_one_fb_leaf` (`shim.rs:10233`), which mints a host object and maps it at `host_va == leaf.va`.

⚠ **Cost is the design question, and it is not small.** cup2's VAS is `16425` rows / **7 runs /
~101 MB**. A wholesale mirror is 16425 `map_dma`s at 4 KiB granularity, or ~1540 at 64 KiB. That is
a rung of its own and it needs the owner's four-places check (this is PDB-PTE, so we may be in the
path) — it is **not** started here.

⊘ **Do not re-derive:** the parked halves (retired above), 64 KiB rounding, `GET_PTE_INFO`,
copy-and-swap promotion, ioctl divergence.

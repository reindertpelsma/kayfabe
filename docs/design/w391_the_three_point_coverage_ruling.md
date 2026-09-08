# w391 — THE COVERAGE RULING: THREE POINTS, AND THE TRAP SURFACE THAT GOES AWAY

**STATUS: OWNER RULING, 2026-09-08.** Supersedes the framing in
`w390_the_invalidate_blockage_point.md` §2/§2b — see §0. Design ruling + implementation
directive; the doorbell design in §3 is NOT yet built.

---

## §0 — ⊘⊘ WHAT THIS CORRECTS IN w390

w390 concluded *"the TLB invalidate is a valid barrier and an INVALID notification — it
publishes 25 of 74 rows because the guest fires no invalidate at all for the rest."* **Two of
that sentence's three claims are now retracted, both by the owner, both correctly.**

1. ⊘ **"The guest fires no invalidate for the rest" — WRONG SUBSYSTEM, not a hole.** The 49
   unattributed rows are not silent RM maps. `nvidia-uvm` has its **own** page-table writer
   (CE pushbuffer, `uvm_pte_batch.c`) and its **own** invalidate transport (pushbuffer
   `MEM_OP`, `uvm_ampere_host.c:255-265`) and **never** touches BAR0 `0x00B8_30B0`. w390's
   census counted BAR0 register writes only, so every UVM mapping was structurally invisible
   to it. ⇒ **a census over transports is only as complete as its list**, for the second time
   in this campaign.
2. ⊘ **"DEFER means the invalidate never happens" — WRONG CONTRACT.** `gvaspaceInvalidateTlb`
   passes `pRootMem`, the **root** memdesc, so it is a **WHOLE-VAS** invalidate, not a
   per-VA one. One un-deferred operation therefore flushes every deferred map before it. That
   is why the flag is `DEFER` and not `SUPPRESS`, and why RM needs no replay machinery: the
   caller's terminating operation IS the replay. `[measured, bare metal]` a raw client that
   DEFERs and then uses the address without invalidating **crashes** — the batch must be
   terminated or the guest breaks itself.
3. ⊘ **And my "malicious guest" framing was wrong.** An unpublished mapping means our host VAS
   lacks the row, so the **host engine faults on the guest's own work**. The guest gains
   nothing; it is self-harm. Unmap is the same, and stronger: on unmap a stale entry provably
   exists, so the guest must invalidate or its own release does not take effect.
   ⇒ The genuinely security-relevant boundary is **object lifetime (FREE) and our retire path**,
   which we mediate directly. It is NOT the TLB, and does not belong in this coverage set.

---

## §1 — ★★★★★ THE RULE

> **Our obligation is co-extensive with page-table state the GPU walks.** Every transition
> into that state must pass one of our interception points. Guest-local CPU-side bookkeeping
> in ogkm is **out of scope by construction** — if it produces nothing the GPU can walk, it can
> produce no fault, and a real GPU could not observe it either.

That last clause is the one that retires a whole class of would-be "coverage gaps".

## §2 — THE THREE POINTS

| # | point | covers | why it is a real gate |
|---|---|---|---|
| 1 | **TLB invalidate** (BAR0 `0x00B8_30B0`) | RM `dmaUpdateVASpace` — `MAP_MEMORY_DMA` and friends | the guest **spin-polls** `TRIGGER` (`kgmmuCheckPendingInvalidates_TU102`, `kern_gmmu_tu102.c:59-84`). `[measured w390c2]` we held it 25.2 ms, `over_budget=0`, `reentrant=0` |
| 2 | **`MEM_OP` on EMULATED channels** | UVM's page-table writes | UVM's channels are **Emulated** by construction: `project.rs:311` classes a channel Emulated when its anchor is `SYSTEM_ANCHOR`, and `SYSTEM_ANCHOR` = `RESERVED_CLIENT`, which `RmGraph::apply` **refuses as guest input**. We own the forward, so unlike a doorbell this is a genuine barrier |
| 3 | **RPC phys→VA** | GSP-mediated maps | ◐ **only when it actually yields PTEs the GPU walks.** Under split-VAS (`bSplitVasManagementServerClientRm` defaults `NV_TRUE`, `gpu_registry.c:171-183`) the guest maps locally and `NV_RM_RPC_MAP_MEMORY_DMA` fn 14 never fires — then there is nothing to cover, and that is not a gap |

⚠ **Point 2 is the one with an unverified premise.** CLAUDE.md records `MEM_OP` /
`MMU_TLB_INVALIDATE` pushbuffer methods measuring **ZERO** on the Mode-2 compute path. Before
this set can be called closed, that zero needs a **known-positive**: either we do not decode
`MEM_OP`, or that workload did not exercise UVM, or UVM invalidates a third way. A zero on a
point we are about to depend on is exactly the shape `a_census_zero_needs_a_known_positive`
names.

## §3 — WHAT THE RULING SCRAPS (owner, 2026-09-08)

- **No BAR2 traps.** BAR2 behaves like BAR1: passthrough mappings, or to our fake FB buffers.
- **No BAR1 traps.**
- **No traps in DMA / guest DRAM.**
- **No traps in GPGA address space, ever.** ★ **GPGA is STORAGE, not a control surface** —
  fake VRAM or real-VRAM backed, passthrough `mmap`ed. ⊘ This is a structural requirement, not
  a policy: a `memory_region_init_io` region **cannot** be non-trapping, so the plane must be
  registered as RAM (`_ram_ptr` or a real mapping). "Every BAR is `_init_io` in our shim and
  `realize` asserts that" is therefore the thing that has to change.
- **BAR0 keeps only a small trapped set**, mostly **writes** — the named registers and the
  doorbell tokens. **Read traps are almost never needed.**

## §4 — THE DOORBELL FAST PATH (directive; not yet built)

The whole vmexit→re-enter must contain **no blocking syscall**, minimal locks, **no BQL held**,
no allocation, and no logging in prod.

```
vmexit (BAR0 doorbell write)
  │
  ├─ read token
  ├─ O(1) lookup in the DOORBELL TABLE  →  8-byte word: {emulated bit, target}
  │
  ├─ PASSTHROUGH → ring the translated host doorbell. Nothing else. Return.
  ├─ EMULATED    → set "new work on channel X", wake the sleeping worker. Return.
  └─ NOT FOUND   → return. Silent. (log only under DEBUG)
```

**The table.** Intentionally small, O(1), one 8-byte word per doorbell. Prefer **atomics over a
mutex**; safety of that is to be checked, not assumed. Ordering rule: **deregister the doorbell
from the table BEFORE removing the channel from the channel list.** Growth must not race with
either doorbell invocation or table modification — one option is to make the table its own
`mmap` region so growth is appending pages, with capacity in the first word so oversized tokens
are rejected by a bounds check.

⊘ The owner sanctions `unsafe` Rust or assembly on this path if it buys speed on the
**passthrough** case.

★ **BQL:** assert-in-debug that it is NOT held (and, where the assertion needs an owner, that it
is this vCPU thread), and compile the assertion out in prod.

## §5 — WHAT IS NOT YET MEASURED

- Point 2's `MEM_OP` known-positive (§2 warning).
- **The unprimed DEFER case**: on a genuinely fresh VA there is no stale entry, so a DEFER'd map
  may be usable with no invalidate at all. Every arm we have run *primes* the VA with a
  deliberate fault first. ⚠ This is a **compatibility** question, not a security one — if it
  works bare-metal and faults under us, that is a behavioural difference worth knowing.
  ⊘ Owner's position: even if it succeeds it may violate NVIDIA's own protocol, and the coverage
  holds as long as CUDA always treats a DEFER as a non-finalized mapping.

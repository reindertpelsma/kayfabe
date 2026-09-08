# w392 — ⊘ POINT 2's TRANSPORT IS NOT UNDRAINED, IT IS **UNREACHED**

**STATUS: LIVE, 2026-09-08.** Measured on a real GA106 (RTX 3060, open 580.159.04, KVM
template box `50260029`), source revision `aa4ffb4d`, stamp gate passed.
Supersedes nothing; extends `w391_the_three_point_coverage_ruling.md` §2.

---

## §0 — THE HEADLINE

The owner's three-point coverage ruling makes point 2 — **`MEM_OP`/`MMU_TLB_INVALIDATE` on
the guest's emulated channels** — a precondition for scrapping the doorbell's VAS
publication. Before this rung, its status read as *"the decoder exists, the sink is never
drained; wire the drain."*

**That was wrong, and the correction changes the work.** Measured:

| fact | value | where |
|---|---|---|
| `MMU_TLB_INVALIDATE` decodes reaching `apply_pushbuffer` | **0** | `MEMOP-CENSUS seen=0` |
| pushbuffer parses that ran at all | **0** | `grep -c FWD-RING` |
| doorbells in the same boot | **480** (`GrCompute=125 Ce=355`) | by-engine census |
| `CUP3_VAL` | **43** | compute passes; no regression |

⇒ **`parse_pushbuffer` never executed.** The census could not have read anything else. The
transport is not merely undrained — **no ring is parsed at all on this workload**, so there
is no arm onto which a drain could be wired.

---

## §1 — WHY THE ZERO IS TRUSTWORTHY THIS TIME

`CLAUDE.md` already records the C measuring this transport at **ZERO** on the Mode-2
compute path. A second zero would normally be worth nothing: *"a census ZERO needs a
KNOWN-POSITIVE"*, and without one it cannot separate *"the guest never invalidates"* from
*"the counter was never wired"*.

So the census ships with its own known-positive (`tests/tests/memop_census.rs`):
a real `PushMethod::TlbInvalidate` driven through the real `parse_pushbuffer`, asserting
(1) the decode arm runs, (2) the PDB is **read out of the guest's method words** and not
echoed from the channel's own VAS, and (3) the census moves. Plus a **negative control**:
a CE-only ring must leave the census flat.

★ The negative control caught a defect **in the test, on its first run**: the census is a
process-global `AtomicU64` and libtest is multi-threaded, so an exact before/after delta
straddled a sibling test's invalidate — a *true* report of a *false* premise. The fix was
to make the delta attributable (a file-local lock), **not** to weaken the assertion to
`>=`, which would have deleted the only check that catches a counter wired to every method.

⇒ The bench zero is a fact about the **guest and the transport**, not about the instrument.

---

## §2 — THE MECHANISM, NAMED EXACTLY

`kayfabe_rt::device::ring_content_is_forwardable` (`device.rs:6394`) is a **conjunction**:

```rust
matches!(route_of_engine(engine), DoorbellRoute::CpuCe)
    && matches!(kind, GuestChannelKind::Emulated)
```

and it is the sole gate above the sole call to `forward_ring`, which holds the sole call to
`parse_pushbuffer` on the live path (`device.rs:2907` → `:3144`).

`route_of_engine` maps `Ce → CpuCe`, `GrCompute|GrGraphics → HostGr`. So the parse runs
**only for a channel that is both CE-class and Emulated**.

### ⊘ And the boot could not say which half failed — the joint fact was never recorded

The log carried the two **marginals** on different lines — `engine=Ce` ×355,
`kind=emulated` ×202 — and the **joint nowhere**. `grep 'kind=' | grep 'engine='` returns
**zero lines**. Two readings fit and they have different fixes:

- **the CE doorbells were all Passthrough** (user procs) ⇒ the emulated channels that ring
  are GR, and UVM's `MEMOPS` pool never rang at all;
- **the emulated doorbells were all GR** ⇒ same conclusion by the other route;
- or a mix.

⇒ **This is why w392 adds one line at the gate** printing the pair *and naming the failing
conjunct* (`⊘ SKIPPED: ENGINE conjunct` / `⊘ SKIPPED: KIND conjunct`). A skip must arrive
as a reason, not as an absence. ⚠ Not yet read back from a boot at the time of writing.

---

## §3 — WHAT IS SETTLED, AND FROM WHERE

**`MEM_OP`/`MMU_TLB_INVALIDATE` IS a kernel channel and DOES land `Emulated` — by a chain
that is total, not heuristic.** The owner asked; this is the answer, every link cited in
ogkm 580.159.04 and in our own source:

| # | fact | citation |
|---|---|---|
| 1 | GA106's invalidate is `MEM_OP_D OPERATION=MMU_TLB_INVALIDATE`, class C56F | `kernel-open/nvidia-uvm/uvm_ampere_host.c:255` |
| 2 | every one rides `UVM_CHANNEL_TYPE_MEMOPS` on `gpu->channel_manager` | `uvm_mmu.c:62`, `:722` |
| 3 | that manager comes from `nvGpuOpsCreateSession` → `rmapiGetInterface(RMAPI_EXTERNAL_KERNEL)` | `src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:777` |
| 4 | ⇒ `privLevel >= RS_PRIV_LEVEL_KERNEL` ⇒ RM stamps `processID = KERNEL_PID` **in the RPC encoder, i.e. on our own wire** | `src/nvidia/src/kernel/vgpu/rpc.c:3382`, `inc/kernel/vgpu/rpc.h:69` |
| 5 | we decode `KERNEL_PID` → `ClientKind::Kernel` | `kayfabe-abi/src/guest_os.rs:286` |
| 6 | every declared Kernel client folds into the ONE system component at `SYSTEM_ANCHOR` | `kayfabe-core/src/project.rs:1026`, `:1170` |
| 7 | `channel_kind() → Emulated` ⇒ `trap_contract() → ScheduleAndReturn` | `kayfabe-core/src/project.rs:312` |

★ Link 4 is the load-bearing one and it is better than expected: the sentinel is stamped by
the **RPC encoder**, which is our transport. We do not have to infer privilege — RM tells
us, in-band.

### ⊘ The owner's worst case is a non-case, for two reasons

> *"lets say in the worst case it isn't [a kernel channel], and it does a CE copy to the
> address — it's interceptible only I think, its flakey as out of scope of what a CE copy
> should do."*

Correct, and there is a stronger reason. Under the passthrough data plane the host GPU
walks the **host** page tables, which host RM owns. A guest scribbling its own PTE pages
publishes **nothing to hardware**; it only *tells us its intent*. Missing one faults the
guest's own work. It is a liveness bug for that guest, never a breach.

---

## §4 — THE BLIND SPOT, STATED

`Ga10xPushbuffer::tlb_invalidate` (`kayfabe-chips/src/ga10x.rs:1457`) returns `None` for the
`TLB_INVALIDATE_PDB_ALL` form — *"`PDB_ALL` names no page directory; there is no `pdb` to
report"* — so a whole-GPU invalidate falls through to `PushMethod::Opaque` and is **counted
nowhere**.

That is right for a *publish* (there is no VAS to publish) and wrong for a *census*, so it
is written down rather than left to be rediscovered. `seen()` counts **PDB-targeted**
invalidates only. UVM's own emitter uses `TLB_INVALIDATE_PDB, ONE` (`uvm_ampere_host.c:258`),
so the path this census exists to measure is inside what it can see.

---

## §5 — WHAT THIS COSTS THE RULING

Nothing in the ruling changes. What changes is the **order of work**:

1. ~~wire the drain on `PushbufferOutcome::invalidates`~~ — premature; the arm never runs.
2. **read the `RING-GATE` line off a boot** and learn which conjunct fails. One boot.
3. depending on the answer: either UVM's `MEMOPS` channels never ring (→ the raw UVM-ioctl
   client the owner asked for becomes the known-positive that forces one), or they ring and
   are misclassified (→ a classification fix, much smaller).
4. only then wire the drain, onto an arm proved live.
5. only then scrap the doorbell's VAS publication.

⚠ **Step 5 stays last.** w390 measured that with publication off (`assert` arm) compute is
dead: `NO_KERNEL_LINE`, 16 × Xid 31. Scrapping publication before point 2 carries the
mapping loses compute, which is the owner's own sequencing concern.

---

## §5b — ★★★★★ THE UVM RAW CLIENT PASSES ON BARE METAL (2026-09-08, host of box 50260029)

`--uvm-invalidate`, real GA106, open 580.159.04, source `7311e312`:

```
ok W392C uuid          = GPU-b448b62a-2cac-58c2-46ff-4ecd1e67fc4b
ok W392C INITIALIZE    = rmStatus 0x0
ok W392C MM_INITIALIZE = rmStatus 0x0
ok W392C REGISTER_GPU  = rmStatus 0x0
ok W392C REGISTER_VAS  = rmStatus 0x0
W392C_OUTCOME=(P) THE CLIENT WORKS
```

★ **This refutes a sentence in our own source.** `blockage_coverage`'s doc says the
emulated-doorbell point *"needs a guest-KERNEL channel (UVM's), which a raw client cannot
allocate."* `REGISTER_GPU` returned `0x0` for a client that allocated **no channel at
all** — a raw client does not *allocate* the kernel channel, it makes **nvidia-uvm**
allocate one. The claim turned *"not built"* into *"cannot be done"*, and the one coverage
point with no raw-client arm was the one recorded as unreachable.

⊘ **The gate caught THREE defects in the client**, and the `NV_STATUS` *name* misdirected
two of the fixes: `0x5d` is `NV_ERR_PAGE_TABLE_NOT_AVAIL`, but `uvm.h:368` documents its
actual meaning for this ioctl as *"the UVM file descriptor [must] be associated with a
single process"*. The missing call was `UVM_MM_INITIALIZE`, not anything about page tables.
⇒ when a status code sends you at a mechanism twice and misses, read the header's **prose**
for that call, not the shared error-code table.

## §6 — RESIDUE

- ⊘ The `RING-GATE` instrumentation is written and compiled but **its output has not been
  read from a boot**. Every §2 conclusion about *which* conjunct fails is therefore open.
- ⊘ Whether UVM's `MEMOPS` pool allocates channels at all on the cup3 workload is
  **unmeasured**. cup3 is one CUDA program; the LLM workload may differ.
- ⊘ The raw UVM-ioctl client (owner-requested) is **not yet written**. Its job is to force
  a UVM page-tree grow deterministically, so the known-positive does not depend on what a
  CUDA program happens to do.
- ⊘ **The client checks STATUSES, not CONTENT** — owner, same session: *"also needs to test
  for corruption of old mappings, and test that the contents is right."* A status-only
  client would have scored the w392 corrupted LLM run (16 tokens, garbage text, every
  `rmStatus` clean) as a pass. The extension is: write a pattern, churn other mappings,
  read the original back and compare. **Not built yet.**
- ◐ **The scale discriminator is CROSS-BOOT, not same-boot.** `MINMM_SUM=64` (w392b) and the
  16 garbage tokens (w392llm2) come from different boots, because w392b was cut before its
  LLM arm finished. The inference — *the defect scales with allocation count/size, not with
  the arithmetic path* — is sound but weaker than a single boot carrying both.

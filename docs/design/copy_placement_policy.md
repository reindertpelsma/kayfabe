# Copy placement and aperture policy — the owner's ruling, 2026-08-27

**STATUS: LIVE, 2026-08-27.** Owner ruling, given in chat during the vidmem-in-GPA lane
(task #271). Supersedes nothing; it *names* a policy the tree had been following only by
accident, and in one place was about to break.

Companions: `fb_cpu_view.md` (rescoped the same day — vidmem HAS a CPU view),
`mode2_uvm_residency.md` (the UVM exception, DECIDED 2026-06-04),
`mode2_address_table.md` (the aperture the guest declares).

---

## 1. ★★★★★ THE APERTURE IS WHAT THE GUEST SETTLED — we never fabricate one

A leaf declared `GMMU_APERTURE_VIDEO` is backed by **vidmem**. A leaf declared sysmem is
backed by **sysmem**. We do not substitute one for the other for our own convenience.

⊘ **This is a conformance rule, not a performance tuning knob.** The count of leaves where the
guest's declaration and our backing disagree must read **zero**; a nonzero reading is a defect
with a name, not headroom to be traded off. ⚠ An earlier framing in this lane treated that
same number as *"the perf opportunity"* — that reading is **retired**, because it invites
exactly the fabricated aperture this rule forbids.

Why it matters beyond tidiness: the guest's page tables carry the aperture bits, so a backing
that disagrees with them is a guest whose own PTEs describe memory that is not there. `#12`
(the 2nd-context hang) was an **aperture mismatch** and cost a campaign week.

### 1.1 ⊘ THE SOLE EXCEPTION: UVM

Managed allocations are the one case where the guest's declaration is not the answer, because
under UVM **residency migrates** and no single aperture is true for the lifetime of the
mapping. Our policy is already decided and unchanged (`kayfabe-fwd/src/lib.rs:450`,
`mode2_uvm_residency.md`, DECIDED 2026-06-04): a guest managed VA is backed by a host
`cudaMallocManaged` allocation and **host UVM owns residency**.

★ Same shape as the shipped sibling: in `nvkvm-pv`, `/dev/nvidia-uvm` is the one device that
never receives a device mapping and takes the degenerate anonymous-window path instead
(`src/qemu/nvkvm_isolate_handlers.c:4059-4070`).

---

## 2. ★★★★★ WHICH MECHANISM MOVES BYTES

| move | mechanism | why |
|---|---|---|
| bulk **DtoH** | **copy engine** | a CPU read from a BAR mapping is uncached and crawls |
| bulk **HtoD** | **copy engine** | WC writes are better than WC reads and still far below CE |
| bulk **DtoD** | **copy engine** | ⚠ the worst case for a memcpy: read over BAR *and* write back over BAR — the PCIe penalty paid **twice** for a copy that never needed to leave the card |
| **HtoH** | memcpy | clearly fastest; no engine involved |
| small pointers / integers / structs | memcpy | a channel submission plus a completion wait to move 4 bytes costs far more than an uncached read |

★★★ **This is what NVIDIA itself does**, and it is why `cudaMemcpy` at size is an engine
operation rather than a loop.

### 2.1 ★★ THE TWO BUCKETS ARE SEPARATED STRUCTURALLY, SO NO THRESHOLD IS NEEDED

An MMIO exit carries **one register-sized access**. So the trapped framebuffer path
(`SparseFb::read` / `SparseFb::write_tagged` → `MappedFb`, `kayfabe-device/src/fbwin.rs:983`,
`:1027`) is **4 or 8 bytes by construction** and can never be bulk. Its memcpy is correct with
no size threshold to measure and no way for bulk to arrive there by accident.

⇒ The rule does not need a tuned crossover. It needs the **bulk sites** to use the engine, and
there are only two of them.

### 2.2 ★★★★★ THE OBLIGATION THIS CREATES — the establishment copy

`FbStore::install_join` (`kayfabe-device/src/fbwin.rs:1145-1160`) copies the store's resident
pages into the joined backing with `region.write(at, src)`. **A leaf is at least one
`FB_LEAF_GRANULE`, so this is bulk HtoD.**

Today it is an ordinary memcpy into a `memfd` and that is correct — HtoH.
⚠ **The moment the leaf becomes vidmem it turns into a bulk CPU write across the BAR**, which
this policy forbids. It must convert to `ce_copy` (`kayfabe-isolate-host/src/rm.rs:4873`, with
the two-fact outcome instrument at `:6761`) **in the same change that flips the backing** —
not as a follow-up.

⊘ Do not read *"WC is write-combining, so writes are fine"* as a licence to skip this. WC
writes are **less bad** than WC reads, not good: they still cross PCIe at single-digit GB/s
against the CE writing device-local at full framebuffer bandwidth. That reasoning was made and
corrected inside this lane.

### 2.3 Already correct, recorded so nobody re-derives it

The guest's own bulk `cudaMemcpy` in either direction is issued as methods in **its own
pushbuffer**, which we forward to the host engine. That path is already CE and needs no change.

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

---

## 3. ★★★★★ MEASURED 2026-09-06 — ON THE FB-LEAF PATH THE MISMATCH IS 100%, BY CONSTRUCTION

⊘ **And the aperture separation the policy asks for is ALREADY ENFORCED there.** This was
checked rather than assumed, because the question *"are we putting sysmem-declared allocations
in GPGA?"* deserved a measurement and the answer is **no**.

`kayfabe-rt/src/device.rs:3692` — the FB-leaf **candidate filter**, quoted:

```rust
} else if b.aperture() != kayfabe_arch::Aperture::Vidmem {
    c.not_vidmem += 1;
}
```

⇒ A range the guest declared **sysmem never becomes an FB leaf at all**. It is excluded and
counted into `not_vidmem`, which is the existing instrument proving the filter runs. Guest-RAM
ranges are split off one branch earlier (`b.is_guest_ram()`, `:3690`).

★★★ **THE CONSEQUENCE, which is stronger than a mismatch count.** Every candidate that reaches
the join is **`Vidmem`-declared by construction**, and every one is then backed by a sysmem
`memfd` — `FbLeafBacking::Joined` is passed as a **literal** at both production call sites
(`kayfabe-rt/src/device.rs:4490`, `kayfabe-qemu-raw/src/shim.rs:10902`).

⇒ On this path the declaration-vs-backing disagreement is **not a number to measure. It is
every leaf, always.** `backing_for` (`kayfabe-fwd`) reduces to the constant
`FbLeafBacking::Vidmem` here, and the remedy is a **substitution**, not a decision.

⚠ **What this does NOT say**, kept explicit so the scope is not widened by a later reader:
- It is a statement about the **FB-leaf path only**. Other allocation paths were not measured.
- It does not mean the leaves are *wrongly chosen* — the filter is right. It means the leaves
  are **rightly chosen and wrongly backed**.
- `FbLeaf` (`kayfabe-rt/src/completion_watch.rs:408`) carries `va`, `len`, `phys` and **no
  aperture**, so the aperture is decoded at the filter and dropped before the join. That does
  **not** need plumbing for this path — the filter already guarantees the answer — but it will
  need plumbing for any path where both apertures can reach one join site.

⇒ **This is the measurement that makes the vidmem lane a substitution rather than a design
question**, and it is why the remaining work is `alloc_device_local` + `map_cpu` + the
`ce_copy` conversion of §2.2, with no new decision required.

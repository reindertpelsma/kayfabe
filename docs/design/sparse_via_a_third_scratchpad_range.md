# Sparse BAR1 ranges, served by the GPU's own sparse mapping

**STATUS: DESIGN, 2026-09-11.** Owner's design. ⊘ **Not built, not measured.**

> **Owner:** *"map a sparse region in the scratchpad channel at a third offset (above the GPGA
> and DMA), then you can mmap this as MMIO in guest bar1 when a sparse mapping is needed. Does
> not consume vram, yes means that those regions are not more performant than MMIO but on real
> bare metal this compromise is already the same, so it does not necessarily make a huge
> difference for parity (I think none at all)."*

## The problem it solves

A **sparse** PTE is valid-but-unbacked: a read returns zeros, a write is dropped, and **it does
not fault**. It exists so a stray access to a declared-but-unmapped range is benign.

Two facts make this ours to answer rather than ignorable:

1. **Sparse is not BAR1-only.** `UVM_MAP_EXTERNAL_SPARSE` is a real userspace ioctl
   (`kernel-open/nvidia-uvm/uvm.c:1017` → `uvm_api_map_external_sparse`, gated on
   `parent->sparse_mappings_supported`), so a channel's address space can carry sparse ranges.
2. **BAR1 is untrapped as of w431.** An access that finds no backing falls back to the trap,
   and we must answer it with zeros rather than a fault.

## The design — a third scratchpad range

The scratchpad address space already has two (`gpga_is_one_reserved_object.md` §"The
scratchpad's address space"):

| range | contents |
|---|---|
| `X .. X+\|GPGA\|` | the whole reserved object |
| above it | the **fake range** — promoted buffers, already sparse |
| **above that** | ★ **the sparse range** — one region, mapped sparse, backing nothing |

When the guest needs a sparse mapping in BAR1, point that BAR1 sub-window at the **host BAR1
offset of our sparse range**. The guest's CPU access then reaches the real GPU, which applies
its own sparse semantics.

## Why this is better than emulating it

⊘ The rejected alternative was a private anonymous mapping with `MADV_POPULATE_READ` — readable
zero pages without a data page each. It works for reads and needs us to **emulate** the write
side (drop stores to sparsified ranges), which means knowing which ranges are sparse and
getting the predicate right on every path.

★ This design gets the semantic **from the hardware**: reads return zeros and writes are
dropped because the GPU's own sparse PTE says so. There is no predicate of ours to get wrong.

- **No VRAM.** A sparse mapping backs nothing.
- **One region serves every sparse need**, because every sparse range reads identically.
- **Correct by construction** rather than by our emulation.

## The parity argument, and I agree with it

Such a region is MMIO-speed, not memslot-speed. ⊘ But **on bare metal a sparse BAR1 read is
already a PCIe read to the GPU that returns zeros** — there is no faster path being given up,
because no memory exists to map. ⇒ no parity cost.

⚠ That holds *only* for genuinely sparse ranges. If we ever pointed a **backed** range at this,
we would be trading a memslot for MMIO and the argument would not apply.

## ⊘ What must be measured before building it

1. **Does a CPU read of a sparse range through host BAR1 actually return zeros on GA106** — or
   does it fault? The whole design rests on this and it is one ladder rung.
2. **Does anything ever touch a sparse BAR1 range in our workloads?** We already count it:
   `[measured w455]` `BAR1-PASSTHROUGH arm=on misses=175`, `bar1 touches: 14`. If none of those
   land in a never-mapped range, this is unnecessary and should not be built.
3. **Does the CUDA path use `UVM_MAP_EXTERNAL_SPARSE` at all?** Countable in a guest ioctl
   census.

⇒ Build order is (2) then (1), and (3) decides whether channel VA spaces need it too. **Do not
build it before (2) says something touches a sparse range** — an unexercised mechanism is what
this session has spent its time removing.

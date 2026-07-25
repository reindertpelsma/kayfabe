# Portability — arm64 is a deferred target, never a bolt-on

> Status: **deferred-but-seam-guarded** (decision #36). No arm64-specific code is built
> today; the near-term target is commodity GeForce, which is overwhelmingly x86. This doc
> records the design rules + the CI gate that keep arm64 an *adapter* we can add later, never
> a retrofit through the core. Same pattern as MIG (`multi_gpu_and_mig.md`) and the deferred
> graphics pipeline: design toward it, gate the seam, build when there's a reason.

## Why arm64 matters (eventually)

arm64 + NVIDIA is a real and growing configuration: **Grace-Hopper / GH200** put an arm64
CPU and the GPU on one package, and **Jetson** is arm64 edge silicon with an NVIDIA GPU. If the
product ever targets datacenter/edge hosts, arm64 is part of "commodity" there. It is *not* the
near-term target (GeForce lives in x86 boxes, and the arm64-NVIDIA world leans on the enterprise
features — MIG/vGPU/CC — we deferred). So: worth not-rotting, not worth building now.

## Where arm64 would (and would not) touch the layers

- **L0 core (pure logic) — arch-clean by construction.** The core is pure Rust over abstract
  domain types (`Gpa`, `GpuVa`, `Pdb`, `GpuId`, …). It makes no assumption about pointer width
  (64-bit both), endianness (little-endian both, and the ABI decode is explicit about byte order
  regardless), CPU page size, or any x86 instruction/intrinsic. GPU VA and PDB are *GPU* concepts,
  independent of the host CPU architecture. **This is proven, not asserted** — see the CI gate.
- **★ L1 (`nvkvm-linux-raw` — the mmap / GPA-window layer) — the one real pressure point.**
  arm64 hosts run **16 KiB or 64 KiB base pages** (not x86's 4 KiB). The GPA-window / double-mmap
  machinery aligns and slices by page size. **Binding rule: the host page size is queried at
  runtime (`sysconf(_SC_PAGESIZE)`), NEVER a hardcoded `4096`.** Any alignment, window granularity,
  or slice size derives from that runtime value. This rule is written down *now* so it is a design
  constraint when L1 mmap is first built — not a painful retrofit after 4 KiB is baked in. (This
  doc is the record until the L1 mmap design exists to carry the rule; it must be honored there.)
- **L2 (QEMU/VMM adapter) — already portable.** QEMU runs on arm64 hosts; the trap/BAR/interrupt
  surface is via KVM, which QEMU abstracts (GIC vs APIC is QEMU's concern, not the device's —
  `Vmm::raise_irq` is abstract). No core exposure.
- **L3 (per-arch NVIDIA ABI) — GPU-arch, not CPU-arch.** The RM ioctl structs are explicitly sized
  and cross-platform (the NVIDIA driver is itself cross-platform); the Axis-A codegen from `ogkm`
  headers produces arch-correct layouts per target naturally. GPU *architecture* (Ampere/Ada/Hopper)
  is orthogonal to host CPU architecture.

## The enforcement — a CI gate, not a hope

`.github/workflows/ci.yml` job **`aarch64`** runs
`cargo check --workspace --target aarch64-unknown-linux-gnu` on every push. `cargo check`
type-checks without linking, so it needs no cross-toolchain or QEMU — it is cheap, and it
**structurally proves the core stays arch-portable**: the moment an x86/CPU-arch assumption creeps
into any core crate, this gate fails on the push that introduces it, exactly the way the fmt/clippy
gates work. Verified green at introduction (whole workspace cross-checks clean for aarch64).

## When arm64 is actually built (the later work)

1. Keep the L0 gate green (free — it already is).
2. Implement `nvkvm-linux-raw` with the runtime-page-size rule from day one.
3. Run the Axis-A ABI codegen for the arm64 target (should be a no-op layout-wise, but verified).
4. Validate on real arm64 + NVIDIA hardware (Grace-Hopper / Jetson — the x86 GeForce bench cannot
   exercise it).

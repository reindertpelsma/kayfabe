# Portability — arm64 is a deferred target, never a bolt-on

> Status: **deferred-but-seam-guarded** (decision #36). No arm64-specific code is built
> today; the near-term target is commodity GeForce, which is overwhelmingly x86. This doc
> records the design rules + the CI gate that keep arm64 an *adapter* we can add later, never
> a retrofit through the core. Same pattern as MIG (`multi_gpu_and_mig.md`) and the deferred
> graphics pipeline: design toward it, gate the seam, build when there's a reason.

## ★★ 2026-07-27 — the suite RAN on arm64. Cross-check upgraded to measurement.

**[measured]** `KAYFABE_SLOW=1 cargo test --workspace` on a **GB10 (Grace-Blackwell, `aarch64`)**
host, kernel `6.17.0-1021-nvidia`, 20 cores, rustc 1.97.1, tree ≈ `bd1a547`:

> **372 passed · 0 failed · 0 ignored — identical to the x86_64 count.**

This matters because until now the arm64 claim rested on `cargo check --target
aarch64-unknown-linux-gnu`, which proves *compilation*, not *behaviour*. Nothing had ever
**executed** on ARM. The core's arch-cleanliness above was argued from construction; it is now
observed — including the concurrency suites, whose memory-ordering behaviour is exactly what a
cross-compile check cannot see (aarch64 is weakly ordered; x86 is not, and a missing `Acquire`
or `Release` is the classic bug that passes on x86 and fails on ARM).

**What it does NOT establish, stated so nobody over-reads it:**
- **`getconf PAGESIZE` was 4096 on that host**, so the 16/64 KiB page-size rule below is still
  *unexercised*. The one real pressure point remains untested. The owed run is Grace-Hopper or
  Jetson configured with 64 KiB pages.
- The box was a **container**: no `/dev/kvm`, no `/dev/userfaultfd` (bare `userfaultfd()` returns
  `EPERM` — container root lacks `CAP_SYS_PTRACE`). So **no L1/L2 OS-shell behaviour was
  exercised on ARM**, and the region-lock question (`../reference/region_lock_mechanism_study.md`
  GL13, which *refuses the capability on arm64*) is untouched by this result.
- **[measured] vast.ai has ZERO machines with `cpu_arch=arm64 vms_enabled=true`** — every
  VM-capable offer is x86. arm64 + KVM must come from elsewhere (Graviton bare-metal, Oracle
  Ampere, Hetzner ARM). That is what blocks settling GL13 by experiment.

⇒ The honest summary: **the pure core is arm64-clean by measurement; the OS shell on arm64 is
still entirely unmeasured.**

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
- **★ L1 (`kayfabe-linux-raw` — the mmap / GPA-window layer) — the one real pressure point.**
  arm64 hosts run **16 KiB or 64 KiB base pages** (not x86's 4 KiB). The GPA-window / double-mmap
  machinery aligns and slices by page size. **Binding rule: the host page size is queried at
  runtime (`sysconf(_SC_PAGESIZE)`), NEVER a hardcoded `4096`.** Any alignment, window granularity,
  or slice size derives from that runtime value. This rule is written down *now* so it is a design
  constraint when L1 mmap is first built — not a painful retrofit after 4 KiB is baked in. (This
  doc is the record until the L1 mmap design exists to carry the rule; it must be honored there.)
  ★ **The rule now has a mechanism *and* a stated limit** — `l1_os_shell.md` §5: a
  `HostPageSize` newtype with no literal constructor, pure geometry functions over it, and the
  page size as a **test axis** (4/16/64 KiB). **But the typed rule stops at the adapter**: the
  core's geometry constructors (`GpaSpace::new`, `TargetGeom`) take plain `u64` and cannot take
  a `HostPageSize`, because a pure crate may not depend on `kayfabe-linux-raw`. So a literal
  `4096` *does* typecheck as an `arena_len`, and the obligation is on the composition root —
  **validate core-supplied geometry against the queried page size at construction, loudly.**
- **L2 (QEMU/VMM adapter) — already portable.** QEMU runs on arm64 hosts; the trap/BAR/interrupt
  surface is via KVM, which the VMM abstracts (GIC vs APIC is the adapter's concern, not the
  device's — `Vmm::raise_irq` is abstract). No core exposure.
  ★ **Two places arch and hypervisor intersect, recorded by the portability round
  (`l1_os_shell.md` §14.4) because each is invisible from either axis alone:**
  - **`Vmm::map_read_native`'s `write_trap` sub-range is rounded to whole HOST pages.**
    Read-native vs trapped is a *page* attribute in every hypervisor's mapping machinery, so
    a caller reasoning in 4 KiB gets more pages trapped than it asked for on a 16/64 KiB
    host — correct, and quietly slower. The rustdoc says so; derive it from `HostPageSize`.
  - **`IrqSpec::IntxLevel` is unimplementable on some (VMM, arch) pairs** — a
    cloud-hypervisor adapter's legacy INTx path is a userspace IOAPIC gated
    `#[cfg(target_arch = "x86_64")]`, so on CH/aarch64 the variant must return
    `VmmError::Unsupported`. Harmless in practice (the core only ever emits `Msix(0)`), and
    written into the trait so the first adapter to meet it treats it as a contract.
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
2. Implement `kayfabe-linux-raw` with the runtime-page-size rule from day one.
3. Run the Axis-A ABI codegen for the arm64 target (should be a no-op layout-wise, but verified).
4. Validate on real arm64 + NVIDIA hardware (Grace-Hopper / Jetson — the x86 GeForce bench cannot
   exercise it).

## ★★ 2026-07-30 — portability is not only about the ISA: the host CPU's physical-address width

**[measured]** A KVM-gated test passed on one x86_64 box and failed on another x86_64 box, and
the variable was neither the kernel nor the distro but the **CPU**:

| box | CPU | phys bits | memslot at GPA `0x9000_0000_0000` |
|---|---|---|---|
| dev box | AMD EPYC 7543 | **48** | installs |
| `vr` | Intel Xeon E5-2697A v4 | **46** | **`EINVAL`** |

`KVM_SET_USER_MEMORY_REGION` refuses any memslot whose guest-physical address exceeds the
**host** CPU's physical-address width. A 46-bit host tops out at `0x3FFF_FFFF_FFFF`. This is not
configurable, and `KVM_CAP_NR_MEMSLOTS` says nothing about it — both boxes advertise the identical
`32764`, and both refuse their first out-of-range *slot number* at exactly `32764`, so that cap is
a **count** whose highest legal index is `cap - 1`. Two independent ceilings, one of them invisible
until you cross it.

**Why it belongs in this doc.** The five axes we guard (driver version, GPU architecture, kernel
version, hypervisor, guest OS) are all *software* axes, and the arm64 work above frames portability
as an *ISA* question. This was neither. Two machines of the same ISA, same distro family, same
driver, differing only in silicon generation — and a test changed colour. **"x86_64" is not a
platform; it is a family with measurable spread.** Anything that hands a hardcoded address to the
kernel is making a claim about the host CPU whether or not it knows it.

**What it does NOT establish, stated so nobody over-reads it:**
- Nothing about the *product* was found to depend on the width. The sweep found the offending
  constants only in **tests**. Product code does not currently mint far-away GPAs.
- The other high-address constants in the suite (`l1_mean.rs`, `determinism.rs` — the latter
  `0x1_0000_0000_0000`, i.e. 2⁴⁸, illegal even on the 48-bit box) are safe **only because they
  never reach a real memslot.** That safety is incidental, not designed. The day one of those
  paths becomes real, it breaks, and it will break on someone else's machine.
- `vm.max_map_count` also differs by an order of magnitude between the two boxes (1048576 vs
  65530). It was not the cause here, but it is the next ceiling any test holding tens of thousands
  of live mappings will hit.

**The rules that follow:**
- **Derive the address, don't declare it.** The fix was to compute the probe GPA from the test's
  own layout rather than pick a dramatic constant — then there is no width to be wrong about. A
  literal above 2⁴⁶ is a portability bug even in a test.
- **This family is CI-blind.** The KVM-gated tests are counted by CI and never passed by it (no
  `/dev/kvm` on the runner), so a CPU-dependent failure here is invisible in CI *and* invisible on
  whichever box happens to accept it. A local green is not a portability result — the only
  instrument that has ever caught one of these is running the suite on a *second, different* box.
- **Suspect the CPU before the kernel** when two Linux boxes disagree. This is now the second
  incident where the vendor/silicon, not the kernel version, was the hidden variable.

★ There is a testing lesson here that outlived the bug. The failing assertion read
`.expect("the number that just came back")` — it named the **hypothesis** it was probing, so an
`EINVAL` arriving for a completely unrelated reason surfaced as apparent proof that the slot
**recycling allocator** had broken. A message that names what a call *did* costs nothing; a message
that names what its failure *would prove* actively misdirects the next reader. See
`testing_doctrine.md`.

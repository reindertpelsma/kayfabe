# Mode-2 UVM residency — the canonical plan

Status: DECIDED 2026-06-04 (user + Claude design session). This is the canonical
approach for CUDA Unified Memory (`cudaMallocManaged`) under Mode-2. It resolves
the only Mode-2-specific UVM blocker; everything else reuses Mode-1 machinery.

## The problem (Mode-2 only)

UVM's residency machinery — `uvm_parent_gpu_service_replayable_faults`,
`migrate_vma`, `hmm_range_fault` — is driven by **GPU page faults** and is
**kernel-resolvable only**. There is *no* user-space hook to drive it:
userfaultfd-armed VMAs are explicitly rejected by UVM's HMM path
(`uvm_hmm.c`: "UVM doesn't support userfaultfd"), FUSE doesn't convert GPU faults
into user callbacks, and `/dev/nvidia-uvm` exposes no "pause fault, fill page,
replay" API. (ChatGPT kernel-contract analysis, ~/chat-questions.txt, confirms
all of this.)

Mode-1 never hit this: it forwarded the guest's UVM ioctls to the **host** UVM,
which owned residency on the **real** GPU. There was no guest UVM. Mode-2 runs the
stock guest driver, so the **guest** UVM driver is live — but there is no real GPU
under it to fault, and the real compute runs on the **host** GPU whose faults go
to **host** UVM. That impedance mismatch is the whole problem.

Key scoping fact: **UVM *migration* only matters for `cudaMallocManaged`.** On
Pascal+, nvidia-uvm.ko manages the unified *VA space* for all allocations (the
cuInit `UVM_REGISTER_GPU` registration, already handled), but the fault/residency
machinery only ever runs for *managed* ranges. Explicit `cuMemAlloc` device memory
is vidmem-resident, never faults, never migrates — it forwards exactly like Mode-1.
So **UVM never blocks basic compute**; this doc is only about managed memory.

## The model: guest managed VA = pass-through to a HOST managed allocation

The guest's managed range is backed by a **host** `cudaMallocManaged` allocation.
**Host UVM owns residency**; the **guest UVM is an inert fiction.** Three mappings
exist and decouple cleanly:

1. **guest VA → GPA** (the guest UVM's view): held *static* as "resident in
   sysmem, GPU is DMA-ing it." It never changes, so the guest **never faults at
   this level** for managed memory. The guest's GPU-side PTEs are fiction — no real
   silicon ever consults them (real compute is the host GPU on the host managed
   allocation; the emulated GPU forwards).
2. **GPA → HPA** (EPT/NPT): where the host migrates, transparently. When host UVM
   pulls the page into host VRAM (device-private), the MMU-notifier invalidates the
   EPT entry; the next guest-CPU touch EPT-faults → GUP hits the device-private
   entry → `migrate_to_ram` → host UVM pulls it back to host sysmem → EPT
   reinstalls → guest reads it. The guest sees only a normal memory access.
3. **host GPU → host managed allocation** (host UVM): real faults, real
   VRAM↔sysmem migration, **native speed**.

So the guest CPU literally plays the role of "the host CPU" in an ordinary
host-CUDA managed-memory session, mediated by EPT. That is *why* this is host
parity — it **is** the native host model, with guest-CPU accesses arriving as EPT
faults instead of direct host faults. The guest UVM "always thinks the page is
DMA-resident in sysmem" while the host swaps VRAM underneath it. (Same
residency-ownership model nvproxy runs in production; Mode-1 proved the
GPA-window-over-host-allocation half at parity.)

## Why no fake residency / no lie

The residency we report to the guest UVM ("sysmem-resident, GPU-accessible") is a
*supported, fault-free* UVM state (the `PreferredLocation=CPU` + `AccessedBy=GPU`
zero-copy / pinned-mapped-host-memory configuration: GPU PTE → host RAM, no
migrate-to-VRAM step). The guest UVM's *belief* is simply decoupled from the host
GPU's *actual* mapping — we always point the real GPU at the real host allocation,
whatever the guest thinks.

## The fallback (guaranteed-correct, slower)

If the fast path hits friction, back the GPA window with **plain anonymous host
memory + remote GPU access** (host GPU os-descriptor-maps the guest RAM, accesses
it remotely over PCIe; pinned by the os-descriptor registration, exactly Mode-1
sysmem). No host migration: always-sysmem, always-correct, slower GPU access. This
is the floor; the pass-through model above is the parity target.

## The one spike to de-risk the fast path

Confirm KVM will fault a **memslot through the host's UVM/HMM-managed VMA** with
migration coherence intact: device-private pages + `migrate_to_ram` on GUP + MMU
notifiers invalidating EPT. Soundness depends on whether GA106 UVM uses the HMM
device-private model (clean) vs. the legacy `VM_MIXEDMAP`/`.fault` model (needs
checking). Spike: run a `cudaMallocManaged` workload through a Mode-1-style GPA
window and confirm a host VRAM↔sysmem migration round-trips while the guest CPU
reads correct bytes. Mode-1's GPA-window-over-host-managed precedent + nvproxy
production de-risk this; the spike just confirms the Mode-2 wiring.

## Accepted limitation

The guest cannot independently oversubscribe / swap managed memory to its **own**
disk the way native UVM can (residency is host-owned; the guest can't evict to
backing store it controls). The host can still swap its sysmem copy. For a
GPU-passthrough VM this is a non-issue.

## Mode-2-new code (everything else is Mode-1 machinery)

The only genuinely new piece: **keep the guest UVM quiescent** — report static
sysmem residency and never honor a guest-side migrate-to-vidmem (the emulated GPU
never delivers UVM faults, so the guest UVM stays put on its own; we additionally
ignore any guest-initiated migration host-side, always mapping the real GPU at the
host managed allocation). The backing (host managed alloc behind the GPA window)
and the forwarding are the Mode-1 stub/GPA-window/isolate stack ([[gpa-window-design]],
[[uvm-in-qemu]], [[isolate-architecture]]).

## Slots into M5

- Basic compute (explicit device memory) = the near-term unblock; needs none of
  this (M5.1–M5.3 context backing).
- Managed memory = host managed allocation behind the GPA window + quiescent guest
  UVM, EPT does migration for free. Add the `cudaMallocManaged`-through-GPA-window
  migration round-trip as the M5 managed-memory spike.

See [[mode2-address-virtualization]], [[mode2-promote-ctx-and-uvm-wall]],
docs/design/mode2_plan.md (M5).

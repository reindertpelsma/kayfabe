# Why the isolate exists — VA identity first, security third

⊘ **If you are about to describe the isolate as a security mechanism, stop and read §1.** That
ordering is a bug in the record, and it has already cost this project one reintroduced bug and one
fragile design.

★ This file exists because the reason lived in a conversation and not in the artifact. Recovered by
the owner, 2026-08-09.

## 1. ★★★ The founding reason: NVIDIA sees the host VA and the guest VA as ONE

gVisor's `nvproxy` works because **a guest process IS a host process**. NVIDIA's driver — and UVM in
particular — treats the address a buffer has in the calling process as *the* address. `nvproxy` gets
that identity for free, so UVM works out of the box.

⇒ In a VM it does not. A guest process is not a host process, and the identity breaks. **That is why
porting `nvproxy` to QEMU was believed impossible.**

★★★ **The isolate is the disproof.** One host process per guest process gives each guest process
**its own host address space**, so the guest's VA can *be* the host VA again. Mode 1's whole fight —
getting UVM to work at all — was this fight.

### Why no amount of checking replaces it

Two guest processes routinely use the **same** virtual address. One host address space cannot hold
two different mappings at one address. ⊘ **That is arithmetic, not policy.** Bounds-checking before
an ioctl cannot make an address space hold two things at once.

- `#14` — *two concurrent CUDA apps hang at `cuCtxCreate`* — **is** this collision.
- `#102` is the same fact arriving late: the C names it *"the irreducible primitive the whole data
  plane rests on"* (`C: nvkvm_gpu_emul.c:7663`, quoted in `eight_blockers_resolved.md §1`), and the
  Rust suite was found **asserting its negation** — it required two processes' identical guest VAs
  to get *distinct* host VAs, since corrected, with the old assertion recorded in place at
  `tests/tests/sim_14_two_process.rs:127-135`.
- ★ The resolution is **address-space** separation, not **address** separation: identical guest VAs
  land at the **same** host VA inside **different** host VASes. The C reserves 128 GiB
  `MAP_ANONYMOUS|MAP_NORESERVE|MAP_FIXED` and places guest-derived offsets inside it, **only in the
  isolate** (`C: nvkvm_isolate.c:1270`, `eight_blockers_resolved.md §1`). ⇒ The isolate is not
  merely *where* address identity is implemented; it is *what makes it expressible at all*.
## 1b. ★★★ The strongest objection, and it is the owner's own result — SHARING

**The fact** (owner, 2026-08-09, from Mode 1): every NVIDIA object can be **shared out of the
isolate to the VMM and mapped at an arbitrary VMM address** for a KVM memslot. Mode 2 used exactly
this to pass LLM compute.

⇒ **The objection this licenses**: if any object can be exported by fd and re-mapped anywhere, there
is no address constraint at all — so run every ioctl in the VMM and bounds-check in Rust.

### ★ What it genuinely refutes — and I had it in §1

⊘ **"One host address space cannot hold two mappings at one address" is too broad as stated, and one
half of it does not force an isolate.** Two corrections, both to my own text above:

1. **Object mappings are re-addressable.** An object reached by *handle + offset* does not care
   where it lands; export and re-map moves it freely. The owner's result settles this plane, and it
   is the plane the *data path* lives on.
2. **GPU VASes do not need separate processes.** One RM client may own many `VASpace` objects, so
   two guest processes' identical guest VAs can already live in two different **host GPU** VASes
   inside **one** host process. ⇒ The GPU-side half of §1 is real but is **not** what forces
   per-process isolation. ⊘ I asserted arithmetic where the actual constraint is API shape.

### ★★★ What survives, and it is the sharper statement

**An address passed as an ARGUMENT is re-addressable. An address the driver takes IMPLICITLY from
the calling process is not — and `mm` is not a parameter.**

- **UVM: unconditional, but ⊘ NOT the knock-down argument this document first claimed.**
  See §1c, which corrects it. In short: every CUDA process initialises UVM whether or not it ever
  touches managed memory, but the `mm` binding is **opt-out**, and a production system ships the
  opt-out.

## 1c. ⊘ CORRECTION (2026-08-09, same night) — I OVERSTATED THE UVM RECEIPT

An earlier revision of this file (commit `7f81f5c`) claimed the driver enforces `mm` identity **by
error code**, citing the C. ★ That claim is **too strong**, and the correction matters more than the
original because a decision was about to be built on it.

**What holds** `[measured]`, and it is worth having:
- **UVM init is UNCONDITIONAL.** A program that only calls `cuInit`/`cuDeviceGetCount` and never
  allocates managed memory still opens `/dev/nvidia-uvm` **twice** and runs `UVM_INITIALIZE`,
  `UVM_MM_INITIALIZE`, `UVM_PAGEABLE_MEM_ACCESS`, `UVM_REGISTER_GPU`,
  `UVM_PAGEABLE_MEM_ACCESS_ON_GPU`, `UVM_CREATE_RANGE_GROUP` — all inside `cuInit`, before
  `cuDeviceGetCount` returns (`C: docs/research/captures/ga106_cuinit_shim.log:14-18, :93-95`, real
  GA106; independently on 575.51.03 + CUDA 12.9 at `C: docs/CUINIT_BLOCKER.md:193-205`).
- And it is **causal**, not incidental: *"cuInit gates `cuDeviceGetCount > 0` on `UVM_REGISTER_GPU`
  succeeding"* (`C: docs/CUINIT_BLOCKER.md:202-204`), with the run table at `:318-325` showing
  `cuInit FAILED: 100` when UVM init was broken. ⇒ There is no "case 1" of CUDA-without-UVM to
  design for.

**What does NOT hold** — three findings, each `[src]` against the drivers on disk:
1. ⊘ **`uvm_api_mm_initialize` contains no `current->mm` comparison at all.** Its only
   `NV_ERR_INVALID_ARGUMENT` paths are "not a UVM file" and "fd is not `UVM_FD_VA_SPACE`"
   (`ogkm-580: kernel-open/nvidia-uvm/uvm.c:59-137`; byte-identical in `ogkm-610`; the 575.51.03
   source quoted at `C: docs/CUINIT_BLOCKER.md:210-217` shows the same three checks). ⇒ The C's
   comment at `C: src/stub/nvkvm_stub.c:242-253` is a **reconstruction**: the `SCM_RIGHTS` rejection
   was real, but the attributed mechanism is unsupported — most likely the fd failed the
   `UVM_FD_VA_SPACE` test, not an `mm` test. ★ **An observed error plus a plausible mechanism is not
   a measured mechanism**, and this one survived into three documents including mine.
2. The `mm` **is** captured — but at `UVM_INITIALIZE`, not `MM_INITIALIZE`: `uvm.c:953`
   `uvm_va_space_create()` → `uvm_va_space.c:264` `uvm_va_space_mm_register()` →
   `uvm_va_space_mm.c:195` `va_space_mm->mm = current->mm`.
3. ★★★ **And it is skipped entirely under a flag.** `uvm_va_space_mm_enabled()` returns false when
   `initialization_flags & UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE` (`= 0x2`,
   `C: src/abi/uvm.h:55`) is set (`ogkm-580: uvm_va_space_mm.c:172-179`). With it false,
   `va_space_mm->mm` is never set and the real cross-process gate — *"when the VA space is
   associated with an mm, all vmas under the VA space must come from that mm"* (`uvm.c:782-788`,
   `-EINVAL`) — **is never armed**.
   ⇒ ★ **gVisor already ships this exact opt-out**: `nvproxy` forcibly ORs
   `MULTI_PROCESS_SHARING_MODE` into every app's `UVM_INITIALIZE` — comment verbatim *"This is
   necessary to share the host UVM FD between sentry and application processes"* — then masks the
   flag back out of the reply so the app never sees it
   (`gvisor/pkg/sentry/devices/nvproxy/uvm.go:188-200`).
   ⚠ And on our own bench the binding was inert for a second reason anyway: `UVM_MM_INITIALIZE`
   returns `NV_WARN_NOTHING_TO_DO` on the vanilla host because that build has
   `UVM_CAN_USE_MMU_NOTIFIERS() = 0` (`C: docs/CUINIT_BLOCKER.md:236-240`).

### ⇒ What the isolate argument actually rests on, restated honestly

⊘ **"UVM state is `mm`-bound, therefore one host process per guest process is forced" DOES NOT
FOLLOW.** The binding is opt-out, and gVisor's `nvproxy` ships that opt-out on every app it runs
(`gvisor/pkg/sentry/devices/nvproxy/uvm.go:188-200`).

What survives is **weaker in kind and still real**: UVM VA ranges are **identity-mapped**
(`uvm.c:793`, `vm_start == vm_pgoff << PAGE_SHIFT`), and `MAP_EXTERNAL_ALLOCATION` /
`ALLOC_SEMAPHORE_POOL` carry **raw VAs**. So sharing one host `va_space` across guests does not hit
an `mm` impossibility — it collapses every guest into **one flat host VA allocator**, where two
guest processes at the same VA collide. ⇒ That is `#14` again, and it is an *allocator* problem, not
a *kernel-refusal* problem: solvable in principle, at the cost of owning a global VA plan across
mutually distrusting tenants.

★★ **So the strongest remaining argument is §2, not §1** — a new host process with a **new
`hClient`** is separation RM itself performs, on hardware, rather than separation we assert. That is
the owner's reading and it is the one to lead with.

⚠ Also recorded from the C, and it refines the whole picture: in Mode 1 the UVM work was **split** —
part isolate-created, but the **map sequence, using a second fd, ran in the VMM**, one of the very
few operations the VMM performed directly, under an ioctl set **stricter** than the isolate's. ⇒ The
boundary was never "everything in the isolate"; it was already a considered split, which is exactly
what §1b's re-addressability result predicts.

⚠ Stale comment found while citing this: `C: src/qemu/nvkvm_isolate.c:152-153` says
*"/dev/nvidia-uvm is intentionally absent: UVM is opened by QEMU, never by the sandboxed stub"* —
contradicted by the stub's own workaround at `nvkvm_stub.c:246-253`. Believe the stub.
- **§2's RM namespace argument is untouched** by sharing: it is about *handles*, not addresses, and
  no amount of re-mapping makes one process into two clients.

### ★★ And the inversion — sharing is the argument FOR the isolate

The cost objection is the strongest case against isolates: *"you have put a process boundary in the
data path."* **Sharing is precisely what defeats it.** Because objects export by fd and land in VMM
memslots, the isolate is **not** in the data path — which is why Mode 2 could pass LLM compute
through this arrangement at parity.

⇒ **Sharing removes the isolate's COST, not the isolate's REASON.** It is what makes the design
affordable rather than what makes it unnecessary. ⚠ And note the empirical rebuttal to the "just
check it in the VMM" position: the C **had** the sharing mechanism the whole time and still
reintroduced `#14`.

- ⚠ The GPU-side answer ("just separate channels and VAS") addresses a **later** stage than the
  `mm`-keyed state above.

## 2. ★★ Second reason: we cannot partition RM's namespace from outside

RM handles are a **global, access-gated namespace** — ⊘ not fd-scoped. RM mints client-scoped
handles from one shared base (`RS_CLIENT_HANDLE_BASE = 0xC1D00000`,
`ogkm-580: src/nvidia/generated/g_resserv_nvoc.h:188`), so isolate A's handle and isolate B's handle
can carry the **same value and both be live and unrelated** — a free would destroy a bystander
(`crates/kayfabe-isolate/src/lib.rs`, `HostHandle`'s doc). And it was **measured** on the GA106
bench on 2026-07-29 (task #95, C at `fc4164d`, two concurrent `cup8`) that two concurrent CUDA
processes **share one dup-DST client** (`0xc1d00001`), aliasing both together. ⊘ Not re-measured on
this tree.

⇒ RM does not hand us a per-process boundary for free. The only lever we have is to **arrive as
genuinely different clients** — one `RmConnection::open` per isolate child, with **no** shared RM
client, device fd or host VA table between isolates.

★ Validation cannot produce this either. One process is one client however carefully its arguments
are checked.

## 3. Third: blast radius

Real, and worth having — but ⚠ **weaker than it sounds**: the isolate still holds a GPU fd, so
compromising one is not empty-handed (`guest_blast_radius.md`, which states the boundary as a
falsifiable property rather than a slogan). ⊘ **Do not lead with this.** It is defence in depth, not the reason the
mechanism exists.

## 4. ⊘ How the ordering was lost, and what it cost

The design was right twice; **the reason lived in a conversation instead of in the artifact.**

1. Mode 1 invented the isolate for VA identity and won.
2. It was later re-described as a security boundary — true, but secondary.
3. ⊘ **Mode 2 (the C) reintroduced `#14`**, because only the security story had survived. It also
   reached for a **fragile CR3-register read** as the process key — the design smell of a mechanism
   whose purpose is no longer understood.
4. ⊘ The Rust rewrite inherited **the mechanism and the weaker story together**, and its suite
   asserted the negation of the primitive for the life of that test (`#102`).

★ **A mechanism whose purpose is not written down will be re-justified by whatever is easiest to say
about it** — and the easy justification will not protect the property that actually matters.

⇒ That is why this file exists, and why the ordering in it is load-bearing. Anywhere the isolate is
introduced security-first, correct it.

## 5. What this does *not* say

⊘ It does not say the security framing is wrong — §3 is real. It says the security framing is
**insufficient**: it cannot derive the one-process-per-guest-process shape, because a design that
only wanted blast-radius containment could have chosen any partition (per-GPU, per-VM, per-N-procs)
and would have picked a coarser one. **Only the `mm`-keyed state of §1/§1b forces the granularity to
be exactly per guest process** — that is the one requirement no partition other than
per-guest-process satisfies.

⚠ Consequence to hold onto: any future change that coarsens the isolate granularity — sharing one
isolate between two guest processes for efficiency — **reintroduces `#14`**, whatever it does for
security. That is the property this document is here to protect.

Related: `eight_blockers_resolved.md §1`, `isolate_vmm_fd_crossing.md`,
`gpu_compartmentalisation.md`, `core_security_threat_model.md`.

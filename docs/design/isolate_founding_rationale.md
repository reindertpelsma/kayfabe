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

- ★★★ **UVM is the clean case, and the C already hit it head-on.** Managed memory's contract *is*
  CPU VA == GPU VA, and a UVM `va_space` binds to the **caller's `mm`** — there is no handle to
  re-address and nothing to pass.

  **And this is exactly where sharing was tried and REFUSED.** The C records that
  `UVM_MM_INITIALIZE` returns `NV_ERR_INVALID_ARGUMENT` *"when the file passed via `uvm_fd` was
  opened by a different `mm` than the caller"* — QEMU opened `/dev/nvidia-uvm` and passed it by
  `SCM_RIGHTS`, and the driver rejected the isolate's call because the file's owning `mm` was
  QEMU's (`C: src/stub/nvkvm_stub.c:246-253`). The remedy was not a wrapper or a check: the stub
  **opens the device itself, twice, before seccomp, and DROPS the passed fd**.

  ⇒ ★★ **The one object that cannot be shared is the one the whole VA-identity argument rests on**,
  and the driver enforces it *by error code*, not by convention. Mode 2 forwards guest managed VAs
  straight to host `cudaMallocManaged` (`mode2_uvm_residency.md`), so this sits on our path.

  ⚠ Stale comment found while citing this: `C: src/qemu/nvkvm_isolate.c:152-153` still says
  *"/dev/nvidia-uvm is intentionally absent: UVM is opened by QEMU, never by the sandboxed stub"* —
  which the stub's own workaround contradicts. Believe the stub. Do not port the comment.
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

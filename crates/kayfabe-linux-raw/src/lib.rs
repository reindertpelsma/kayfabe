//! # `kayfabe-linux-raw` — the one audited raw-OS adapter
//!
//! `l1_os_shell.md` §4 (decision 9). This is the **only** crate in the workspace where
//! the `unsafe_code` lint is not `forbid`, and its API shape — not its implementation —
//! is the subject of the L1-M2 security exit gate (§11).
//!
//! Stage **M2-b** builds the foundation only: the host page size ([`HostPageSize`]), the
//! pure geometry that takes it ([`geometry`]), the bounded region objects
//! ([`MappedRegion`], [`RegionView`], [`VolatileRegion`], [`Reservation`]) and the
//! `mmap`/`munmap` primitives underneath them. See [§ What is deliberately absent].
//!
//! [§ What is deliberately absent]: #what-is-deliberately-absent
//!
//! ---
//!
//! ## ★ The asymmetry that justifies the whole design (§4.2.1)
//!
//! The same arithmetic mistake has two categorically different consequences depending on
//! **which address space** it happens in:
//!
//! - `Gpa` / BAR offset / GPGA / host-**GPU** VA are guest- or device-address spaces.
//!   Out-of-range there is a **guest-visible fault**: MISS = FAULT already covers it, the
//!   answer is a loud refusal, and the blast radius is the guest's own aperture (or, for a
//!   host GPU VA, the *isolate's own* host VAS — the #14 boundary doing its job). These may
//!   be plain integers with checked arithmetic, and elsewhere in this workspace they are.
//! - A **host CPU address** is different *in kind*. Out-of-range there is not a fault; it is
//!   a **hypervisor escape**. There is no aperture to be confined to, no MISS to report and
//!   no guest to fault. A different consequence demands a different treatment, and the
//!   treatment has to be structural because the failure has no detection story of its own.
//!
//! ### And `forbid(unsafe_code)` is not evidence here
//!
//! Creating a raw pointer, casting it to an integer and doing arithmetic on it are all
//! **safe** operations — only the dereference is not. So a boundary that *mints* a host
//! address (a `*mut u8`, a `NonNull`, a `usize`, a `u64` field called `host_va`) and hands
//! it upward has already lost: the safe code above it does unchecked address math with a
//! clean conscience, the dereference that eventually trusts it is three crates away, and
//! the containment gates, the block ratchet and every `// SAFETY:` comment all still pass.
//! **The unsoundness happened where the pointer was minted; everything after it is safe
//! code being wrong.** That is why the rule below is about *what crosses the seam*, never
//! about who writes the dangerous keyword.
//!
//! ## ★★ The rule, stated constructively — this is the part that matters
//!
//! The prohibitive form of the rule is *"no host CPU address ever crosses a crate
//! boundary, in any representation"*. It is true, and on its own it is the weaker
//! statement, because **a rule that only forbids invites people to route around it the
//! moment they need to express something.** So the shape that is actually built here is
//! the constructive one:
//!
//! > **What crosses the seam is a bounded *object*: a region that carries its own length
//! > and offers only checked accessors — `slice(off, len) -> Result<…>`, `read_into`,
//! > `load_u32` — with domain-typed offsets, the length always carried with the base it
//! > belongs to, and no way to obtain a bare address.**
//!
//! Callers can still say *"the thing at offset X, N bytes long"*; they simply cannot say
//! it with an integer, and every step of that arithmetic is bounds-checked **by
//! construction** rather than by discipline. [`HostOffset`] has no `Add` impl at all —
//! only [`HostOffset::checked_add`], which returns a `Result`. [`MappedRegion::slice`]
//! returns another bounded object, so composition re-checks. Nothing in the public API
//! returns `&[u8]`, `*const T`, `*mut T`, `NonNull<T>`, or a base address as an integer,
//! and no region type implements `Deref`, `AsRef`, `Borrow` or `Index`. Rules you can work
//! with survive; rules that only forbid get circumvented.
//!
//! **The exemption, written down because the naming actively misleads (§4.2.1):**
//! `RmBackend::map_gpu_va` returns a `u64` that the core stores as `host_va`. That is *not*
//! a host CPU address — it is an address in the host **GPU's** virtual address space,
//! minted by RM inside the owning isolate, where out-of-range faults the GPU MMU inside one
//! isolate's own VAS. It stays a plain `u64`. Both available mistakes are live: banning it
//! on a name match makes the design worse for nothing, and waving a real host pointer
//! through because "the other `host_va` was fine" makes it unsound.
//!
//! ### What the API refuses, and how each refusal is held
//!
//! | # | Refusal (§4.2.1) | Held by |
//! |---|---|---|
//! | 1 | an accessor returning `&[u8]`, `&mut [u8]`, `*const T`, `*mut T`, `NonNull<T>` | type shape + `tests/ui/no_borrow_escape.rs` |
//! | 2 | a `Deref`/`AsRef`/`Borrow`/`Index` impl on a region type | type shape + `tests/ui/no_borrow_escape.rs` |
//! | 3 | a public field/constructor/getter exposing a base address as an integer | type shape + `tests/ui/no_base_address.rs` |
//! | 4 | `Gpa` ⇄ [`HostOffset`] by `From`, `as`, or arithmetic | type shape + `tests/ui/offset_arithmetic.rs` |
//! | 5 | an address-taking placement API (`map_fixed(addr, …)`) | [`Reservation::map_fixed_in`] takes an **offset**; `tests/ui/map_fixed_bare_address.rs` |
//!
//! **Refusal 5's compile-fail row is the weak one, and the honesty belongs here rather
//! than in a footnote.** A compile-fail test can prove that *the free function does not
//! exist today*; it cannot prove that a future one will not be added, and it cannot see a
//! *semantically* unbounded bounded object — a region whose length field is right but
//! whose backing was mapped shorter. **There is no mechanism for that and this crate does
//! not pretend one exists.** It is a named **review obligation**: every new public item
//! here, and in the real `Vmm` impl, is checked against the five refusals as part of §11's
//! exit gate. Types close four of the five; the fifth is a human.
//!
//! ## The audit surface, enumerable with `ls`
//!
//! ```text
//! $ ls crates/kayfabe-linux-raw/src/*_unsafe.rs
//! chardev_unsafe.rs   epoll_unsafe.rs   host_fd_unsafe.rs   kvm_unsafe.rs
//! mapping_unsafe.rs   sandbox_unsafe.rs scm_unsafe.rs       signal_unsafe.rs
//! spawn_unsafe.rs     sysconf_unsafe.rs vcpu_unsafe.rs      window_unsafe.rs
//! ```
//!
//! §4.1.1's standing rule: the dangerous keyword may appear **only** in a file whose
//! basename ends `_unsafe.rs`. Every other file in this crate — [`error`], [`page_size`],
//! [`geometry`], [`bounds`], [`view`] — is ordinary safe Rust holding the newtypes, the
//! arithmetic and the bounds checks. That split is not cosmetic: §5.2 already required the
//! geometry to be pure and parameterised so it can be tested at three page sizes, and §4.7
//! already required this crate to hold no business logic. The file rule is the mechanical
//! form of both.
//!
//! Four CI gates hold it, all `grep`, no allowlist: **A** every other crate inherits
//! `forbid`; **B** the keyword appears nowhere outside this crate's `src/`; **C** it appears
//! only in `*_unsafe.rs`; **ratchet** the block count matches a committed number, so a new
//! block is a reviewed commit rather than a line inside a debugging diff.
//!
//! ### ★ The house rule for every block in those two files
//!
//! > **Every invariant a `// SAFETY:` comment relies on is established in the *same
//! > function*, on a line the reviewer can see above the block.**
//!
//! No block in this crate has a precondition its caller is trusted to have met. That is
//! why the bounds arithmetic lives in pure functions ([`bounds::checked_span`],
//! [`geometry`]) that the dangerous file *calls itself* immediately before dereferencing,
//! rather than in a safe wrapper that hands a pre-validated offset down. The one type-level
//! invariant that cannot be re-derived per call — "this base is valid for this length" — is
//! documented on the private type that owns it and is established by its two constructors,
//! both of which are `mmap` return values in this same file.
//!
//! ## ★★ Every syscall asserts BOTH witnesses (§4.5, extended at M2-d)
//!
//! §4.5 says *"every raw entry point asserts the lockwitness"*, and until M2-d that meant
//! one witness: `kayfabe_util::lockwitness`, over the core's **ranked** locks. M2-c then
//! found (§14.8 F1) that an adapter's own lock has no rank and is therefore invisible to
//! it, and built a second, thread-local witness inside `kayfabe-vmm-kvm`.
//!
//! **A bite-check at M2-d showed that placing the second witness in the adapter is not
//! enough, and the reason generalises:** the KVM adapter asserts it once per method, at its
//! own execute phase, so *deleting that one line* is undetectable — there is no second line
//! of defence, and every future adapter starts with the same single point of enforcement.
//! (The reactor's registrar was written with exactly that shape and the bite passed
//! silently: `epoll_ctl` under the registrar's unranked table lock, with every assert in
//! the workspace green.)
//!
//! So the second witness moved to [`kayfabe_util::leafwitness`] and is asserted **here**,
//! at the syscall, beside the ranked one — which is what §4.5 always said and what makes
//! it structural: an adapter that forgets its own assert is still caught, because the
//! syscall itself refuses.
//!
//! ## Volatile versus atomic (§4.3)
//!
//! `l1_concurrency.md` §9.1 called this *"volatile access to concurrently-GPU-written
//! pages"*. The intent is right; the mechanism named is not, and the imprecision matters
//! because it is what gets implemented:
//!
//! - A concurrent write by an agent **outside the Rust abstract machine** is a data race.
//!   `read_volatile` is not defined to be atomic and carries no non-tearing guarantee. It
//!   is the right tool for **MMIO registers**, where the *side effect* of the access is the
//!   point — not for shared memory.
//! - For shared memory concurrently written by another agent, the defined primitive is an
//!   **atomic access with `Relaxed` ordering**. It lowers to the same instruction on every
//!   target we care about, it is defined under concurrent modification, and the optimiser
//!   may neither split nor duplicate it.
//!
//! So [`VolatileRegion`] is `&AtomicU32` / `&AtomicU64` views, `Relaxed`, naturally aligned,
//! ≤ 8 bytes, and *nothing else* — the width and alignment refusals are what stop a torn
//! read of a semaphore payload. Ordering against **other** memory is a separate, explicit
//! act (a release fence before ringing a doorbell, an acquire fence after observing a
//! semaphore), named at the two seams that need it rather than smeared into every access;
//! neither seam exists yet, so neither fence is written yet.
//!
//! ## ★ Cacheability is a required parameter, and an honest non-mechanism
//!
//! Every mapping door takes a [`CachePolicy`], never defaulted and never inferred. The
//! reason it is a *parameter* and not a property of the region type is that the attribute
//! belongs to **what the pages are**, not to who writes them: a semaphore in sysmem and a
//! doorbell in a BAR are both [`VolatileRegion`]s and need different attributes, while
//! everything backed by a `memfd` needs the same one whichever region type wraps it.
//!
//! **What this layer can enforce is one thing only: the refusal.** `mmap` has no
//! cacheability flag — page-cache memory is write-back and a device mapping gets whatever
//! the driver's own `mmap` handler chose — so a request for an unattainable attribute is
//! [`RawError::CachePolicyUnattainable`] rather than a silent write-back mapping. That
//! silent downgrade is exactly the shape of the C's #111, which cost days.
//!
//! The three deciders this crate cannot touch — the EPT/NPT type, the guest PTE, and the
//! `NV_MEMORY_*` attribute of the RM allocation — are named with their owners in
//! [`cache`], per variant, along with the observable symptom of each mismatch. Read that
//! module once before choosing a value; the full research is
//! `docs/reference/memory_cacheability.md`.
//!
//! **It is deliberately not reassuring.** An API that *appeared* to set cacheability would
//! reproduce the original bug: a request that is silently not honoured.
//!
//! ## Copy-only on [`MappedRegion`] is a security property, not a style choice
//!
//! Anything read from guest RAM — pushbuffer bytes, RPC queue entries, page-table entries —
//! is mutable by the guest *while we look at it*. A borrow into that memory invites a
//! double-fetch: validate a length field, then re-read it, the classic. [`read_into`] copies
//! once and the parser then works on memory the guest cannot touch. The core is already
//! shaped for this (its fuzz harness feeds it a `&[u8]` it owns), so the rule costs nothing
//! and closes a whole vulnerability class by construction.
//!
//! [`read_into`]: MappedRegion::read_into
//!
//! ## `MAP_FIXED` is the breakout surface (§4.4)
//!
//! `MAP_FIXED` silently unmaps whatever is already at the target address, and in a process
//! that also contains the VMM, "whatever is already there" can be its heap, our stack, or
//! our own text. So placement is a method on `&mut `[`Reservation`] taking an **offset
//! within an address range this process already owns**, never an address — see that type
//! for the finding about `MAP_FIXED_NOREPLACE`, which §4.4 proposes as belt-and-braces and
//! which is **mutually exclusive with reserving first**.
//!
//! ## Linux-only, and how the aarch64 gate stays green
//!
//! This crate is Linux-specific and says so with a `compile_error!` rather than by
//! accident. It is **not** x86-specific: `aarch64-unknown-linux-gnu` is Linux, the
//! `libc` constants it uses are defined for both, and the arch-portability CI job
//! (`cargo check --workspace --target aarch64-unknown-linux-gnu`) therefore covers it —
//! for the first time covering a crate that actually contains arch-sensitive code. The
//! arch sensitivity is exactly [`HostPageSize`]: x86-64 is always 4 KiB, arm64 hosts run
//! 16 KiB or 64 KiB, and **no literal `4096` appears in this crate's alignment path
//! because it does not typecheck**.
//!
//! ## What is deliberately absent
//!
//! Named, because an honest absence is a design statement and a `todo!()` is not:
//!
//! - ~~**`memfd` creation and sealing.**~~ **Landed at M2-c** as [`SharedRam`], with
//!   §11 item 4's `F_SEAL_SHRINK` and the two seals that go with it.
//! - **The double-mmap of guest RAM into an isolate.** [`Reservation`] is the mechanism it
//!   needs and [`SharedRam::dup_for_export`] is the descriptor half; the isolate side is
//!   M2-d. Its unstated precondition is stated at [`Backing::PrivateAnonymous`].
//! - ~~**vCPUs.**~~ **Landed at M2-d** as [`KvmVcpu`] / [`VcpuExit`]: `KVM_CREATE_VCPU`,
//!   the shared run structure, `KVM_RUN`, and the MMIO exit in both directions. The
//!   *dispatch* into a device lives one crate up; this is the plumbing.
//! - ~~**`epoll`**~~ **Landed at M2-d** as [`Poller`] — level-triggered, keyed on a
//!   caller-chosen 64-bit token that is deliberately **not** the descriptor (§3.2). The
//!   **notify descriptor** ([`Notifier`]) landed at M2-c because `Vmm::raise_irq` needed
//!   it, and it is where §4.5's one named `*_under_lock` exception lives.
//! - **`timerfd`.** Still absent, and deliberately: both `Vmm` implementations drive
//!   `defer` off an explicit virtual clock (`kayfabe_vmm::DeferQueue` is *"an
//!   accumulator, not a scheduler"*), so a real deadline descriptor would buy nothing
//!   today and would make every suite that composes this backend non-deterministic —
//!   which §8.3 forbids. It becomes real when something outside a test has to be woken by
//!   a deadline nobody is waiting on.
//! - **`Sync` for [`MappedRegion`] and [`Reservation`].** Refused, and pinned by
//!   `tests/ui/region_is_not_sync.rs`. `MappedRegion::write_from` performs a bulk `memcpy`
//!   through `&self`, so two threads sharing one could race; a caller that must share puts
//!   it behind a lock, which needs only `Send`.
//!   ★ Every grant in this crate is **per type, with the argument written above the
//!   `impl`**, never one blanket relaxation — and the three that exist say different
//!   things. [`VolatileRegion`] has both, because every access is a naturally-aligned
//!   `Relaxed` atomic and there is no bulk accessor to tear. [`MappedRegion`] has `Send`
//!   only (the isolate's guest-RAM plane shares its mappings across a worker pool through a
//!   `Mutex`). [`GuestWindow`] has both: it is the one object a hypervisor's guest memory
//!   genuinely is, live to every thread at every instant.
//! - **The `KAYFABE_FORCE_HOST_PAGE_SIZE` env knob** (§5.2 item 3). The *test axis* it
//!   exists for is delivered — this crate's geometry runs at 4/16/64 KiB — but the knob
//!   itself is consumed by the integration harness (M2-e), and implementing it now would
//!   be untested code that makes [`HostPageSize::query`] depend on the ambient environment.
//!   [`HostPageSize::forced`] is the mechanism it will call.

#[cfg(not(target_os = "linux"))]
compile_error!(
    "kayfabe-linux-raw is the LINUX raw-OS adapter (l1_os_shell.md §4). It is not \
     x86-specific — aarch64-unknown-linux-gnu is a supported target and the CI \
     arch-portability job checks it — but it is Linux-specific by construction. A second \
     OS costs a second adapter crate, never a `cfg` inside this one."
);

pub mod bounds;
pub mod cache;
mod chardev_unsafe;
mod epoll_unsafe;
pub mod error;
pub mod geometry;
mod host_fd_unsafe;
pub mod ioctl;
pub mod kvm_gate;
mod kvm_unsafe;
mod mapping_unsafe;
pub mod memtype;
pub mod page_size;
pub mod procfd;
mod sandbox_unsafe;
mod scm_unsafe;
mod signal_unsafe;
mod spawn_unsafe;
mod sysconf_unsafe;
mod vcpu_unsafe;
pub mod view;
mod window_unsafe;

/// ★★★ The isolate's **containment**: a private mount namespace, a `pivot_root` onto a
/// `tmpfs` holding only the granted device nodes, the `/dev` descriptor minted **inside**
/// it, and a process that comes out the other side holding no capability at all.
///
/// A facade over the audited module, so the security boundary has a name that says what it
/// is. Start at [`sandbox::enter`] — its docs carry the measured escape this closes and the
/// one property that closes it, which is the **order** of the last three steps.
/// [`sandbox::privileges`] is the instrument the last of them fails closed on.
pub mod sandbox {
    pub use crate::sandbox_unsafe::{
        Privileges, SandboxPolicy, enter, namespaces_available, privileges, report, report_gate,
        user_namespaces_available,
    };
}

pub use bounds::HostOffset;
pub use cache::CachePolicy;
pub use chardev_unsafe::{CharDevice, DevDir, Indirect, POINTER_FIELD_WIDTH};
pub use epoll_unsafe::{MAX_READY_BATCH, PollTimeout, Poller, ReadyTokens};
pub use error::RawError;
pub use host_fd_unsafe::{Notifier, SharedRam, descriptor_budget};
pub use kvm_unsafe::{Kvm, KvmMemslot, KvmVm};
pub use mapping_unsafe::{
    Backing, HostProt, MappedRegion, PlacementId, Reservation, VolatileRegion, release_fence,
};
pub use page_size::HostPageSize;
pub use procfd::{MemfdCandidate, MemfdCensus, MemfdRefusal};
pub use scm_unsafe::{
    DescriptorKind, MAX_FDS_PER_FRAME, descriptor_kind, recv_with_fds, require_kind, send_with_fds,
};
pub use signal_unsafe::{
    BREAK_SIGNAL, ThreadId, current_thread_id, install_break_handler, interrupt_thread,
};
pub use spawn_unsafe::{
    ChildSpec, FdGrant, ProgramImage, SandboxChild, adopt_inherited_fd, geteuid,
};
pub use vcpu_unsafe::{KvmVcpu, VcpuExit};
pub use view::RegionView;
pub use window_unsafe::GuestWindow;

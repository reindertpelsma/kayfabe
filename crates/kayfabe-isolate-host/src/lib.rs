//! # `kayfabe-isolate-host` — the REAL host isolate
//!
//! `host_execution_plane.md` §0 states the finding this crate answers, measured at
//! `f0053ef`: **the host execution plane does not exist.** The only implementors of
//! `IsolateFactory`, `Isolate`, `RmBackend` and `Present` were in `kayfabe-mocks`; nothing
//! spawned a host process and nothing issued a real RM ioctl. `kayfabe-vmm-kvm` was real;
//! the entire *host GPU* side was a double.
//!
//! This crate is §3 step 2: a sandboxed child process per `(Proc, GpuId)`, a wire protocol
//! between it and the parent, and an [`RmBackend`] whose implementation is genuine NVIDIA
//! frontend ioctls.
//!
//! [`RmBackend`]: kayfabe_isolate::RmBackend
//!
//! ## The shape, and that it is a PORT
//!
//! The C's Mode-1 isolate architecture is proven and is reproduced rather than redesigned
//! (`port_the_c_not_a_redesign`): one host process per guest `mm`, a numbered descriptor
//! contract, a `/dev` `O_PATH` grant at fd 4, a break signal installed without
//! `SA_RESTART`, and a txn check so a cancel cannot land on an innocent operation. Where
//! this deviates, the deviation is argued at the point it occurs — there are exactly three,
//! and each is written down:
//!
//! 1. **One socket per pool worker, not one shared socket with a txn demux**
//!    (`proto`). `l1_concurrency.md` §11 B6 rules it, and it makes the C's audit-R2-H2
//!    use-after-free unrepresentable.
//! 2. **`CLOSE_RANGE_CLOEXEC` rather than a closefrom** (`kayfabe_linux_raw`'s
//!    `spawn_unsafe`). The C's literal close breaks `std`'s exec-status reporting, so a
//!    failed `execve` reads as a successful spawn.
//! 3. **A dedicated control socket for cancels**, so "out of band" means it rather than
//!    meaning "expects no reply" (`isolate`).
//!
//! ## ★★ What is NOT here — the security gap of record
//!
//! The C's hardened spawn also enters user/pid/net/ipc/uts/mount namespaces, `pivot_root`s
//! onto a tmpfs containing only the bound nvidia nodes, drops every capability, and applies
//! a seccomp allowlist with `TSYNC`. **None of that is implemented.** It is named rather
//! than stubbed, in `kayfabe_linux_raw::spawn_unsafe`'s own docs and here, because an
//! untested sandbox reads as a boundary in every review that follows it. What this crate
//! does give is the part that can be executed and therefore tested on any host: a closed
//! descriptor table, a cleared environment, `PR_SET_NO_NEW_PRIVS`, and a process boundary
//! that is a real reclamation boundary.
//!
//! ## Layout
//!
//! - [`proto`] — the wire vocabulary and its framing. Pure; the peer is untrusted and every
//!   malformation is a named refusal.
//! - [`isolate`] — the parent: the factory, the pool, the proxy backend, the cancel seam.
//! - [`child`] — what runs inside: a thread per worker, one control thread.
//! - [`rm`] — the real RM ioctls, and the bring-up ladder.
//! - [`loopback`] — the transport's fixture, which is emphatically not a model of RM.

pub mod child;
pub mod isolate;
pub mod loopback;
pub mod proto;
pub mod rm;

pub use isolate::{HostIsolate, HostIsolateFactory, RmMode};
pub use loopback::ParkVerb;

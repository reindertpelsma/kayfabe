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
//!    `spawn_unsafe`). The C's literal close would take the pre-`exec` error pipe and the
//!    image descriptor with it, so a failed `exec` would be unreportable — and it is
//!    strictly better security besides: the descriptors stop existing at the `exec` boundary
//!    atomically.
//! 3. **A dedicated control socket for cancels**, so "out of band" means it rather than
//!    meaning "expects no reply" (`isolate`).
//!
//! ## ★★★ The isolate is EMBEDDED, and that is a security property
//!
//! `build.rs` builds this package's `kayfabe-isolate` binary as a **static musl** image and
//! `include_bytes!`s it; the factory publishes it into a **sealed `memfd`** and `exec`s the
//! descriptor with `execveat(AT_EMPTY_PATH)` — the C's own posture
//! (`C: src/qemu/nvkvm_isolate.c:9-10`). There is no path, no `KAYFABE_ISOLATE_BIN`, and no
//! search beside `current_exe()`. This process hands the child a descriptor for `/dev`; it
//! no longer lets an environment variable, or whoever can write one directory, choose which
//! binary that is.
//!
//! ## ★★ What is NOT here — the security gap of record
//!
//! The C's hardened spawn enters user/pid/net/ipc/uts/mount namespaces, `pivot_root`s onto a
//! tmpfs containing only the bound nvidia nodes, drops every capability, and applies a
//! seccomp allowlist with `TSYNC`. Of that list:
//!
//! - **the namespaces are pre-`exec`** — the child is `clone`d into all six, so it cannot
//!   decline them, and `CLONE_NEWPID` (unreachable from a process's own `main`) is had;
//! - **the `pivot_root` and the capability drop are post-`exec`**, in the child's own `main`
//!   via `kayfabe_linux_raw::sandbox::enter`. They allocate, and allocating between `fork`
//!   and `exec` in a threaded process is the malloc-lock deadlock; the C escapes it only by
//!   being freestanding C with no allocator in the path;
//! - **seccomp is absent.** Named rather than stubbed, because an untested control reads as
//!   a boundary in every review that follows it. It is a property of the isolate's *verb
//!   surface* and needs its own falsification test.
//!
//! ## Layout
//!
//! - [`proto`] — the wire vocabulary and its framing. Pure; the peer is untrusted and every
//!   malformation is a named refusal.
//! - [`isolate`] — the parent: the factory, the pool, the proxy backend, the cancel seam.
//! - [`child`] — what runs inside: a thread per worker, one control thread.
//! - [`rm`] — the real RM ioctls, and the bring-up ladder.
//! - [`loopback`] — the transport's fixture, which is emphatically not a model of RM.
//! - [`export`] — ★ the two ends of `RmBackend::export_backing`: the isolate's minted
//!   backings and the VMM's registry of what it adopted. **The first consumer of
//!   [`fdcross`]**, and the answer to that module's standing bound *"no verb uses the
//!   crossing yet"*.

pub mod child;
pub mod export;
pub mod fbjoin;
pub mod fdcross;
pub mod guestram;
pub mod isolate;
pub mod loopback;
pub mod proto;
pub mod rm;

pub use export::{ChildExports, ExportRegistry};
pub use fbjoin::FbJoinTable;
pub use fdcross::{CrossedFd, FdFrameError, FdOrigin, read_frame_with_fds, write_frame_with_fds};
pub use isolate::{HostIsolate, HostIsolateFactory, RmMode, embedded_isolate_bytes};
pub use loopback::ParkVerb;

//! ★★★ The **filesystem containment** an isolate's `/dev` descriptor is only safe inside,
//! and the one property that makes it work: **ordering**.
//!
//! ## The measured defect this file closes
//!
//! Before this file existed, the parent opened `/dev` `O_PATH | O_DIRECTORY` and granted the
//! descriptor to the isolate child, which `openat`s `nvidiactl` from it. Measured on a real
//! host, against the real child:
//!
//! ```text
//!   openat(dirfd, "nvidiactl")      -> OPENED   (intended)
//!   openat(dirfd, "kvm")            -> OPENED
//!   openat(dirfd, "mem")            -> OPENED
//!   openat(dirfd, "../etc/shadow")  -> OPENED   ★ escape
//!   openat(dirfd, "../proc/1/maps") -> OPENED   ★ escape
//! ```
//!
//! An `O_PATH` descriptor places **no restriction on `..`**. There is no `RESOLVE_BENEATH`
//! anywhere in the path, and with no mount namespace `..` walks straight out to the real
//! host root. The whole host filesystem was one relative path away from a process whose
//! entire job is to be the blast radius.
//!
//! ★ [`DevDir`]'s own rustdoc used to argue the opposite — that `O_PATH` "opens the
//! directory **without** the ability to read it, so the grant is *you may name things under
//! here*". That sentence is *true about enumeration* and *irrelevant to the threat*: the
//! threat is naming `..`. It reasoned about the wrong property and concluded safety, which
//! is how the descriptor got shipped. The comment is corrected there; this module is the
//! fix.
//!
//! ## ★★★ THE ORDERING IS THE FIX
//!
//! Adding a `pivot_root` and leaving the `open` where it was closes **nothing**. A
//! descriptor keeps naming the directory it was opened on, and `..` from it keeps resolving
//! in the mount namespace and root that were current *at open time*. So the grant must be
//! **minted after the pivot**, from inside the sandbox, and that is why this module returns
//! a [`DevDir`] rather than taking one: there is no way to call it in the wrong order.
//!
//! `crates/kayfabe-isolate-host/tests/sandbox_escape.rs` holds the non-vacuity half — a
//! committed test that opens the descriptor **before** the pivot and asserts the escape
//! comes back. A containment test that has never been seen to fail is not evidence.
//!
//! ## The sequence, ported from the C
//!
//! `C: src/qemu/nvkvm_isolate.c:155-231` (`nvkvm_child_enter_mount_ns`), whose own comment
//! at `:141-149` is the C's account of finding this exact bug in itself:
//!
//! 1. a mount namespace, so nothing below propagates back to the host;
//! 2. `mount(MS_REC | MS_PRIVATE)` on `/`, for the same reason;
//! 3. a scratch `tmpfs` over **`/proc`** — a directory guaranteed to exist, and mounting
//!    over it masks the host `/proc` for the window before the pivot, a second containment
//!    for free;
//! 4. `dev/` inside it, holding **only** bind mounts of the named device nodes (the runc
//!    device idiom: `mknod` is not permitted in a user namespace, an existing node's bind
//!    is, hence create-empty-file-then-bind);
//! 5. `chdir` → `pivot_root(".", ".")` → `umount2(".", MNT_DETACH)` → `chdir("/")`, which
//!    detaches the old root entirely;
//! 6. **then** `open("/dev", O_PATH)`, whose `..` is now a tmpfs holding one directory;
//! 7. reseal the root `MS_REMOUNT | MS_RDONLY`, **checked**.
//!
//! ★ Step 7's check is not tidiness. A C audit (R4-L1) found that return value ignored, and
//! the consequence was silent: a partial fail-open in which the stub ran with a **writable**
//! root and nothing said so. Here a failed reseal is a refusal, and because the [`DevDir`]
//! is the return value, a refusal means the caller never receives the capability at all.
//!
//! ## ★★★ The second half: the process that comes out the other side holds NO privilege
//!
//! A filesystem boundary around a process with `CapEff = 000001ffffffffff` is a boundary
//! with a key taped to it. Measured on the spawning host: the VMM runs as uid 0 with **every
//! capability** and `Seccomp: 0`, and the isolate child inherited all of it. With
//! `CAP_SYS_PTRACE` such a child reaches its **parent** through `process_vm_readv`
//! regardless of Yama — and the parent holds the KVM descriptor, all guest RAM, and every
//! other isolate's socket. `PR_SET_NO_NEW_PRIVS` (which `spawn_unsafe.rs` already sets) is
//! **inert** against that: it blocks *gaining* privilege via setuid/fcaps and nothing else.
//!
//! So [`enter`] ends by surrendering privilege, ported from `C: nvkvm_isolate.c:66-81`:
//! `PR_SET_NO_NEW_PRIVS`, `PR_SET_DUMPABLE 0`, the `PR_CAPBSET_DROP` loop, `capset` to zero,
//! `PR_CAP_AMBIENT_CLEAR_ALL`. Two things are ours rather than the C's, and both are cheap:
//!
//! 1. ★★ **The outcome is read back and the refusal is on the measurement, not on the
//!    calls.** The C checks the return of `prctl(PR_SET_NO_NEW_PRIVS)` and lets the rest go
//!    — reasonable, since `PR_CAPBSET_DROP` legitimately answers `EINVAL` past the last
//!    capability. But then "the caps were dropped" is an argument about a sequence of calls,
//!    which is exactly the shape of the R4-L1 audit finding one function above. Here
//!    [`privileges`] is called again afterwards and a single surviving bit is a refusal —
//!    and because the [`DevDir`] is the return value, a refusal means the isolate never
//!    receives the capability it would have been holding.
//! 2. ★ **The user namespace is taken first, not as a fallback.** It is what makes the
//!    capability drop irreversible against the *parent*: a tracer in a different user
//!    namespace needs `CAP_SYS_PTRACE` **in the tracee's** namespace, and this process has
//!    none anywhere. The C takes `CLONE_NEWUSER` for the same reason
//!    (`C: nvkvm_isolate.c:117-133`).
//!
//! ## What this is NOT
//!
//! Not seccomp. The C's stub filters its syscalls with a `TSYNC` allowlist
//! (`C: src/stub/nvkvm_stub.c:2505-2587`) and this does not, deliberately **named rather
//! than stubbed** (the discipline `spawn_unsafe.rs` states): the allowlist is a property of
//! the isolate's *verb surface*, it needs its own falsification test, and an untested
//! control reads as a boundary in every review that follows.
//!
//! Nor a pid namespace — **from here**. `CLONE_NEWPID` is now taken by the parent, in the
//! `clone` that creates the isolate ([`crate::ChildSpec::in_new_namespaces`]); it still
//! cannot be had from this side, and [`ISOLATING_NAMESPACES`] says so at the flag word.
//!
//! ## ★ A port finding: why this runs in the child's own `main`, not before `exec`
//!
//! The C does all of this **between `clone` and `fexecve`**. The half of that which is a
//! real security property — *the exec'd image cannot decline the namespaces it was born in*
//! — is now ported: the isolate is `clone`d into user, pid, mount, net, IPC and UTS
//! namespaces before it exists, and it is a **sealed memfd image** rather than a path, so
//! there is nothing to substitute for it either.
//!
//! What is still on this side is the *filesystem construction* — the tmpfs, the binds, the
//! `pivot_root`, the reseal — and the reason is concrete rather than stylistic: [`enter`]
//! **allocates** (it formats paths, reads `/proc` and walks a policy), and allocating between
//! `fork` and `exec` in a process that has other threads is the classic malloc-lock deadlock.
//! The C can do it because its stub is freestanding C with no allocator anywhere in the path.
//! Moving it needs a no-allocation `enter`, which is separate work with its own falsification
//! test.
//!
//! ## ★★ A MEASURED finding: this file needed **no change** for the pre-`exec` namespaces
//!
//! The expectation going in was that it would: a process `clone`d into a rootless user
//! namespace has its map already written, and taking a *second* user namespace here looked
//! like it would nest an unmapped one with nobody left to write its map — so a
//! "were we born namespaced?" branch was written, reading `/proc/self/uid_map`.
//!
//! It was then **poisoned to prove it was load-bearing, and the test still passed**. The
//! nested `unshare(CLONE_NEWUSER)` works: a process that is `0` in its parent's namespace may
//! write the single-line map `0 0 1` for a namespace of its own, which is exactly what
//! [`acquire_mount_namespace`] already does. The branch was dead and was deleted rather than
//! shipped as an untested path. The isolate therefore ends up **two** user namespaces deep,
//! and `tests/sandbox_escape.rs`'s `a_child_born_namespaced_still_enters_the_sandbox` is the
//! measurement that says the containment is unaffected.

use crate::chardev_unsafe::DevDir;
use crate::error::{RawError, last_syscall_error};
use kayfabe_util::{leafwitness, lockwitness};
use std::ffi::{CStr, CString};
use std::path::Path;
use std::sync::OnceLock;

/// The directory the scratch `tmpfs` is mounted over, and which becomes the sandbox root.
///
/// `/proc` because it is guaranteed to exist on any Linux host this can run on, and because
/// covering it masks the host `/proc` for the window before the pivot. (After the pivot the
/// old root is detached, so `/proc` is unreachable whichever mountpoint was borrowed — the
/// masking is about the window, not the outcome.)
const SCRATCH: &CStr = c"/proc";

/// [`SCRATCH`] for the `std::fs` half of the sequence. Two spellings of one path, kept
/// adjacent so they cannot drift.
const SCRATCH_STR: &str = "/proc";

/// Where the device nodes are bound inside the scratch root, and therefore the path the
/// granted descriptor is opened on **after** the pivot makes it `/dev`.
const SCRATCH_DEV: &str = "/proc/dev";

/// The path the sandbox root's `/dev` has after the pivot.
const SANDBOX_DEV: &CStr = c"/dev";

/// One device node the sandbox is to contain.
#[derive(Debug, Clone)]
struct Node {
    /// Name relative to `/dev`, e.g. `nvidiactl` or `dri/renderD128`.
    name: String,
    /// Whether its absence is fatal.
    required: bool,
}

/// ★ The **entire** contents of the sandbox root, declared up front.
///
/// Declarative in the safe direction, exactly as [`crate::ChildSpec`] is about descriptors:
/// a caller states the device nodes the isolate may reach and *everything else is absent* —
/// not filtered, not denied, **absent**. There is no "and also inherit `/dev`" door, because
/// a door like that is the defect this module exists to close.
#[derive(Debug, Clone, Default)]
pub struct SandboxPolicy {
    nodes: Vec<Node>,
}

impl SandboxPolicy {
    /// A sandbox containing no device nodes at all.
    #[must_use]
    pub fn empty() -> Self {
        SandboxPolicy { nodes: Vec::new() }
    }

    /// Include `/dev/<name>`; a host that does not have it is a **refusal**.
    #[must_use]
    pub fn required(mut self, name: &str) -> Self {
        self.nodes.push(Node {
            name: name.to_owned(),
            required: true,
        });
        self
    }

    /// Include `/dev/<name>` if the host has it, and carry on if it does not.
    ///
    /// For nodes whose absence is a *configuration*, never a fault: `nvidia-modeset` on a
    /// headless host, a render node on a machine with no graphics stack.
    #[must_use]
    pub fn optional(mut self, name: &str) -> Self {
        self.nodes.push(Node {
            name: name.to_owned(),
            required: false,
        });
        self
    }

    /// The nodes this policy names, in declaration order — for a diagnostic that has to say
    /// what it asked for.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.nodes.iter().map(|n| n.name.as_str()).collect()
    }

    /// ★ The RM posture: the control node plus the one GPU this isolate is for.
    ///
    /// `nvidia-uvm` is deliberately **absent**, as in the C: UVM is opened by the VMM
    /// process, never by a sandboxed isolate (`C: src/qemu/nvkvm_isolate.c:180-181`).
    /// One isolate is one `(Proc, GpuId)`, so it is granted exactly one GPU node — a second
    /// GPU is a second isolate with a second sandbox, and the containment says so.
    #[must_use]
    pub fn for_gpu(gpu: u32) -> Self {
        SandboxPolicy::empty()
            .required("nvidiactl")
            .required(&format!("nvidia{gpu}"))
    }
}

// =====================================================================================
// The raw calls. One relaxation each, each with the invariant established above it.
// =====================================================================================

/// `unshare(2)` with the given clone flags.
fn unshare_(flags: libc::c_int, call: &'static str) -> Result<(), RawError> {
    // SAFETY: `unshare` takes an integer flag word by value and dereferences no memory at
    // all. It returns 0 or -1; the errno is read through `std::io::Error`, which needs no
    // relaxation of its own.
    let rc = unsafe { libc::unshare(flags) };
    if rc < 0 {
        return Err(last_syscall_error(call));
    }
    Ok(())
}

/// `mount(2)`. The one entry point for every mount this module performs — the new
/// namespace's private-propagation flip, the scratch `tmpfs`, each device bind, and the
/// read-only reseal — so there is exactly one place where a mount's return value could be
/// dropped, and it is not dropped.
fn mount_(
    source: &CStr,
    target: &CStr,
    fstype: Option<&CStr>,
    flags: libc::c_ulong,
    data: Option<&CStr>,
    call: &'static str,
) -> Result<(), RawError> {
    // SAFETY: every pointer handed over is either NULL or derived from a `&CStr` that
    // outlives the call, so each is NUL-terminated *by its type* rather than by an argument
    // we made. `mount` reads those strings and dereferences nothing else; `flags` is an
    // integer by value. It returns 0 or -1 and retains no pointer past the call.
    let rc = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            fstype.map_or(std::ptr::null(), CStr::as_ptr),
            flags,
            data.map_or(std::ptr::null(), CStr::as_ptr)
                .cast::<libc::c_void>(),
        )
    };
    if rc < 0 {
        return Err(last_syscall_error(call));
    }
    Ok(())
}

/// `pivot_root(2)`, which glibc does not wrap.
fn pivot_root_(new_root: &CStr, put_old: &CStr) -> Result<(), RawError> {
    // SAFETY: both pointers come from `&CStr` borrows that outlive the call, so both are
    // NUL-terminated by type. `pivot_root` reads the two paths and dereferences nothing
    // else; the syscall number is a constant. Issued through `syscall` because there is no
    // libc wrapper for it on any platform this builds for.
    let rc = unsafe { libc::syscall(libc::SYS_pivot_root, new_root.as_ptr(), put_old.as_ptr()) };
    if rc < 0 {
        return Err(last_syscall_error("pivot_root"));
    }
    Ok(())
}

/// `umount2(path, MNT_DETACH)` — the lazy detach that unhooks the old root.
fn umount_detach(path: &CStr) -> Result<(), RawError> {
    // SAFETY: `path` is NUL-terminated by its type and outlives the call; `umount2` reads it
    // and dereferences nothing else. The flag is an integer by value.
    let rc = unsafe { libc::umount2(path.as_ptr(), libc::MNT_DETACH) };
    if rc < 0 {
        return Err(last_syscall_error("umount2(MNT_DETACH)"));
    }
    Ok(())
}

/// This process's real uid and gid, **as the outer namespace sees them** — which is what a
/// `uid_map` line has to name, and which stops being observable the moment the map is
/// written.
fn outer_ids() -> (u32, u32) {
    // SAFETY: `getuid`/`getgid` take no arguments, dereference nothing, and cannot fail;
    // they are two of the small set of syscalls POSIX documents as always succeeding.
    unsafe { (libc::getuid(), libc::getgid()) }
}

/// `prctl(2)` with two integer arguments and no memory operand — every process-control
/// operation this module performs is of that shape (`PR_SET_NO_NEW_PRIVS`,
/// `PR_SET_DUMPABLE`, `PR_CAPBSET_READ`/`_DROP`, `PR_CAP_AMBIENT`), so they share one
/// relaxation and one invariant instead of five copies of the same sentence.
///
/// Returns the raw non-negative result (`PR_CAPBSET_READ` answers 0/1 in it).
fn prctl_(op: libc::c_int, arg2: libc::c_ulong, arg3: libc::c_ulong) -> Result<i32, i32> {
    // SAFETY: `prctl` is variadic and every operation named above takes its arguments BY
    // VALUE — none of them is a pointer, so this block passes no memory to the kernel and
    // there is nothing for it to dereference. The trailing two arguments are zero, which
    // every one of these operations requires. It returns -1 with `errno` set, or a
    // non-negative result.
    let rc = unsafe { libc::prctl(op, arg2, arg3, 0 as libc::c_ulong, 0 as libc::c_ulong) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EINVAL));
    }
    Ok(rc)
}

/// Linux's capability ABI, version 3 — the 64-bit-capability layout every kernel since 2.6.26
/// speaks. Named here rather than taken from `libc` because the constant is missing on some
/// of the targets this crate builds for, and a wrong version word makes `capget` answer
/// `EINVAL` while looking like a permission problem.
const CAP_VERSION_3: u32 = 0x2008_0522;

/// The `capget`/`capset` header. A kernel uapi struct, in the crate the ABI-quarantine gate
/// names for exactly this (`ci.yml`, Axis A).
#[repr(C)]
struct CapHeader {
    version: u32,
    pid: libc::c_int,
}

/// One 32-bit slice of the three capability sets. Version 3 passes **two** of these, low
/// word then high word, which is the entire reason a `u64` is not simply passed.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

/// Recombine the kernel's two 32-bit slices into the `u64` the rest of this file reasons in.
fn join(low: u32, high: u32) -> u64 {
    u64::from(low) | (u64::from(high) << 32)
}

/// `capget(2)` for this thread — the three sets as they actually are.
fn capget_() -> Result<[CapData; 2], RawError> {
    let mut header = CapHeader {
        version: CAP_VERSION_3,
        pid: 0,
    };
    let mut data = [CapData::default(); 2];
    // SAFETY: both pointers address local variables that outlive the call, and both have the
    // layout the kernel's version-3 ABI defines (`#[repr(C)]`, above): one header and an
    // array of exactly the two `CapData` words version 3 requires — passing fewer is the one
    // way to make this call write out of bounds, and the array's type is what prevents it.
    // `pid: 0` means "this thread", so no other process is addressed. The kernel writes only
    // into `data` and retains neither pointer.
    let rc = unsafe { libc::syscall(libc::SYS_capget, &raw mut header, data.as_mut_ptr()) };
    if rc < 0 {
        return Err(last_syscall_error("capget"));
    }
    Ok(data)
}

/// `capset(2)` for this thread, to the three sets given.
fn capset_(data: &[CapData; 2]) -> Result<(), RawError> {
    let mut header = CapHeader {
        version: CAP_VERSION_3,
        pid: 0,
    };
    // SAFETY: as `capget_` above, with the direction reversed — the kernel READS the two
    // `CapData` words and the header, both borrowed from locals that outlive the call, both
    // laid out by the version-3 ABI this header declares. `pid: 0` addresses this thread
    // only. Nothing is written back and no pointer is retained.
    let rc = unsafe { libc::syscall(libc::SYS_capset, &raw mut header, data.as_ptr()) };
    if rc < 0 {
        return Err(last_syscall_error("capset"));
    }
    Ok(())
}

// =====================================================================================
// Privilege — measured, surrendered, and measured again
// =====================================================================================

/// ★ **What privilege this process holds, measured.**
///
/// Every field is read from the kernel rather than inferred from what was called. That is
/// the whole point of the type: "we dropped the capabilities" is a statement about a
/// sequence of `prctl`s, and the R4-L1 audit finding this module already carries is what
/// happens when such a statement goes unchecked.
///
/// The three capability sets are `u64` bitmaps over `CAP_*` numbers; the *values* are
/// deliberately not interpreted here, because this crate has no business logic (§4.7) and
/// "which capability is bit 21" is the kernel's vocabulary, not ours. The only question
/// asked of them is whether they are **empty**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Privileges {
    /// Capabilities in force right now.
    pub effective: u64,
    /// Capabilities that may be raised into [`Self::effective`] without an `exec`.
    pub permitted: u64,
    /// Capabilities that survive into a child's permitted set across `exec`.
    pub inheritable: u64,
    /// ★ The **bounding** set: the ceiling on everything above, for this process and every
    /// descendant. Non-empty here means privilege is merely *set aside*, not gone.
    pub bounding: u64,
    /// The ambient set — the modern way a non-root process carries capabilities over `exec`.
    pub ambient: u64,
    /// `PR_SET_NO_NEW_PRIVS`. Necessary and **nowhere near sufficient**: it stops privilege
    /// being *gained*, and is inert about privilege already held.
    pub no_new_privs: bool,
    /// Whether this process may be core-dumped — and therefore, under the same kernel
    /// check, `ptrace`d and read by a same-uid process.
    pub dumpable: bool,
}

impl Privileges {
    /// Nothing held, nothing latent, nothing gainable, nothing readable.
    ///
    /// The bounding set is deliberately **not** part of this: a process that never had
    /// `CAP_SETPCAP` cannot empty it, and with `no_new_privs` set it cannot be used either.
    /// [`surrender_privilege`] applies the stricter rule, because it knows whether the
    /// process had the means.
    #[must_use]
    pub fn is_unprivileged(&self) -> bool {
        self.effective == 0
            && self.permitted == 0
            && self.inheritable == 0
            && self.ambient == 0
            && self.no_new_privs
            && !self.dumpable
    }
}

/// The highest capability number this kernel knows, discovered by asking rather than by a
/// compiled-in constant: `PR_CAPBSET_READ` answers `EINVAL` past the last one, and a
/// hard-coded ceiling would silently stop covering the capabilities a newer kernel adds —
/// which is the direction that matters, since a *new* capability is one nobody's drop loop
/// was written for.
fn last_capability() -> u32 {
    let mut last = 0;
    for c in 0..64u32 {
        if prctl_(libc::PR_CAPBSET_READ, libc::c_ulong::from(c), 0).is_err() {
            break;
        }
        last = c;
    }
    last
}

/// ★ Read this process's privilege state. The instrument [`surrender_privilege`] fails
/// closed on, and the one `kayfabe-sandbox-probe` prints.
///
/// # Errors
/// [`RawError::Syscall`] (`capget`, `prctl`).
pub fn privileges() -> Result<Privileges, RawError> {
    let data = capget_()?;
    let mut bounding = 0u64;
    let mut ambient = 0u64;
    for c in 0..=last_capability() {
        if prctl_(libc::PR_CAPBSET_READ, libc::c_ulong::from(c), 0) == Ok(1) {
            bounding |= 1u64 << c;
        }
        if prctl_(
            libc::PR_CAP_AMBIENT,
            libc::PR_CAP_AMBIENT_IS_SET as libc::c_ulong,
            libc::c_ulong::from(c),
        ) == Ok(1)
        {
            ambient |= 1u64 << c;
        }
    }
    let no_new_privs =
        prctl_(libc::PR_GET_NO_NEW_PRIVS, 0, 0).map_err(|errno| RawError::Syscall {
            call: "prctl(PR_GET_NO_NEW_PRIVS)",
            errno: Some(errno),
        })? == 1;
    let dumpable = prctl_(libc::PR_GET_DUMPABLE, 0, 0).map_err(|errno| RawError::Syscall {
        call: "prctl(PR_GET_DUMPABLE)",
        errno: Some(errno),
    })? != 0;
    Ok(Privileges {
        effective: join(data[0].effective, data[1].effective),
        permitted: join(data[0].permitted, data[1].permitted),
        inheritable: join(data[0].inheritable, data[1].inheritable),
        bounding,
        ambient,
        no_new_privs,
        dumpable,
    })
}

/// ★★★ **Surrender every capability, and refuse if any survived.**
///
/// The C's sequence (`C: nvkvm_isolate.c:66-81`) in the C's order, with the read-back this
/// module's own R4-L1 lesson demands. Individual call failures are *tolerated* — the
/// `PR_CAPBSET_DROP` loop answers `EINVAL` past the last capability by design, and `EPERM`
/// for a process that never had `CAP_SETPCAP` — because the verdict is taken from
/// [`privileges`] afterwards and not from any of them.
///
/// ## Errors
/// [`RawError::Syscall`] naming what survived. Three distinct refusals, because "the drop
/// failed" is not a diagnosis:
///
/// - a capability still held (effective/permitted/inheritable/ambient), or
///   `no_new_privs` unset, or the process still dumpable;
/// - a non-empty **bounding** set *when the process began with privilege* — it had
///   `CAP_SETPCAP` and the ceiling is still there, which means the drop did not do what it
///   reported. A process that started unprivileged cannot empty the bounding set and is not
///   held to it, because `no_new_privs` already makes it unusable.
fn surrender_privilege() -> Result<(), RawError> {
    let before = privileges()?;

    // The C's order. `no_new_privs` first so that nothing between here and the end of the
    // function could regain what the lines below give up.
    let _ = prctl_(libc::PR_SET_NO_NEW_PRIVS, 1, 0);
    let _ = prctl_(libc::PR_SET_DUMPABLE, 0, 0);
    for c in 0..=last_capability() {
        let _ = prctl_(libc::PR_CAPBSET_DROP, libc::c_ulong::from(c), 0);
    }
    let _ = capset_(&[CapData::default(); 2]);
    let _ = prctl_(
        libc::PR_CAP_AMBIENT,
        libc::PR_CAP_AMBIENT_CLEAR_ALL as libc::c_ulong,
        0,
    );

    let after = privileges()?;
    if !after.is_unprivileged() {
        return Err(RawError::Syscall {
            call: "sandbox: privilege survived the drop",
            errno: Some(libc::EPERM),
        });
    }
    // ★ The latent half. A full bounding set with everything else empty is not a contained
    // process, it is one `exec` of a file-capability binary away from a privileged one —
    // and this arm only fires for a process that demonstrably HAD the means to empty it.
    if before.effective != 0 && after.bounding != 0 {
        return Err(RawError::Syscall {
            call: "sandbox: the capability bounding set survived the drop",
            errno: Some(libc::EPERM),
        });
    }
    Ok(())
}

// =====================================================================================
// Entering
// =====================================================================================

/// The namespaces taken alongside the mount namespace, matching the C's clone flags
/// (`C: nvkvm_isolate.c:127-129`): the network, SysV IPC and hostname reach an isolate has
/// no use for, removed rather than filtered.
///
/// ★ `CLONE_NEWPID` is **absent from this list and cannot be had here** — but it is no
/// longer absent from the isolate. `unshare(CLONE_NEWPID)` after `exec` moves only the
/// caller's future *children*, of which the isolate creates none, so taking it here would
/// change nothing and read as a boundary. It is taken by the PARENT instead, in the `clone`
/// that creates the child ([`crate::ChildSpec::in_new_namespaces`]) — which is where the C
/// takes it and the only place it can be had.
const ISOLATING_NAMESPACES: libc::c_int =
    libc::CLONE_NEWNS | libc::CLONE_NEWNET | libc::CLONE_NEWIPC | libc::CLONE_NEWUTS;

/// Acquire the namespaces, by whichever route this process is entitled to.
///
/// ★★ **The user namespace is tried FIRST, and that is a security decision rather than a
/// fallback order.** The privileged route (`CAP_SYS_ADMIN` in the initial user namespace)
/// also works — it is what this used to do, because it is what an isolate spawned by a root
/// VMM is entitled to — but it leaves the child in the *same* user namespace as its parent,
/// and there the `ptrace` access check is satisfied by matching uids alone. Taking a user
/// namespace first means a compromised isolate needs `CAP_SYS_PTRACE` **in the parent's**
/// namespace to reach the parent at all, and [`surrender_privilege`] has just made sure it
/// holds no capability in any namespace.
///
/// `setgroups` must be denied before `gid_map` is writable, which is the kernel's rule and
/// not ours. The single-line map names this process's *outer* ids, which is why
/// [`outer_ids`] is read before the `unshare` — afterwards `getuid` answers the overflow id
/// and the map the kernel would accept is no longer the map we could compute.
///
/// ★ There is no third arm. A host that grants neither route gets an error, and the caller's
/// contract is that an error means no [`DevDir`] — never a `/dev` handed out uncontained.
fn acquire_mount_namespace() -> Result<(), RawError> {
    let (uid, gid) = outer_ids();
    if unshare_(libc::CLONE_NEWUSER, "unshare(CLONE_NEWUSER)").is_ok() {
        // Past this point the process is in a namespace with no mapping at all, so every
        // failure below is a refusal and never a fallback: dropping back to the privileged
        // route from here would run the rest of the sequence in a half-built namespace.
        write_proc_self("setgroups", "deny", "write(/proc/self/setgroups)")?;
        write_proc_self(
            "uid_map",
            &format!("0 {uid} 1\n"),
            "write(/proc/self/uid_map)",
        )?;
        write_proc_self(
            "gid_map",
            &format!("0 {gid} 1\n"),
            "write(/proc/self/gid_map)",
        )?;
        return unshare_(
            ISOLATING_NAMESPACES,
            "unshare(mount/net/ipc/uts) after CLONE_NEWUSER",
        );
    }
    unshare_(ISOLATING_NAMESPACES, "unshare(mount/net/ipc/uts)")
}

/// One-shot write of a `/proc/self/<what>` control file. Plain `std::fs`, deliberately:
/// these are the only file writes in the sequence and they need no relaxation.
fn write_proc_self(what: &str, contents: &str, call: &'static str) -> Result<(), RawError> {
    std::fs::write(format!("/proc/self/{what}"), contents).map_err(|e| RawError::Syscall {
        call,
        errno: e.raw_os_error(),
    })
}

/// A path as a `CString`, or the `EINVAL` a NUL byte in a node name deserves.
fn cpath(path: &str, call: &'static str) -> Result<CString, RawError> {
    CString::new(path).map_err(|_| RawError::Syscall {
        call,
        errno: Some(libc::EINVAL),
    })
}

/// ★★ A node name is a **name under `/dev`**, and this is where that is made true rather
/// than assumed.
///
/// [`SandboxPolicy`] is public and its entries are pasted into both a source path
/// (`/dev/<name>`) and a target path inside the scratch root. A name of `../../etc` would
/// therefore bind an arbitrary host object *into the sandbox* — closing the `..` escape on
/// the descriptor and re-opening it on the policy, which is the exact shape of bug this
/// module exists to stop. So: relative, non-empty, no `..` component, no leading `/`.
///
/// Checked here rather than in [`SandboxPolicy::required`] because the builders return
/// `Self` and a builder that cannot refuse would have to panic.
fn validate_node_name(name: &str) -> Result<(), RawError> {
    let bad = name.is_empty()
        || name.starts_with('/')
        || std::path::Path::new(name)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)));
    if bad {
        return Err(RawError::Syscall {
            call: "sandbox: a device node name must be a plain path under /dev",
            errno: Some(libc::EINVAL),
        });
    }
    Ok(())
}

/// ★★★ **Enter the sandbox, mint the `/dev` descriptor inside it, and surrender every
/// capability.**
///
/// Returns the *only* [`DevDir`] the isolate should ever hold: one whose `..` is a `tmpfs`
/// containing nothing but the nodes `policy` named. On return the process has a private
/// mount namespace, its own user/network/IPC/UTS namespaces, a read-only root, no path to
/// the host filesystem at all, and — measured by [`privileges`], not assumed — **no
/// capability in any namespace**, `no_new_privs` set and `dumpable` cleared.
///
/// The three are one call because their **order** is the security property and an API that
/// let a caller choose it would let a caller get it wrong: the descriptor must be minted
/// after the pivot, and the privilege must be surrendered after the last mount. A caller
/// that could sequence them itself is a caller that can ship the mis-ordered build the
/// committed regression test exists to catch.
///
/// ## Preconditions the caller owns
///
/// - **Call it before spawning threads.** The rootless arm needs `CLONE_NEWUSER`, which the
///   kernel refuses to a multi-threaded process. The isolate child calls this while it is
///   still single-threaded and the assertion below is the reminder, not the enforcement.
/// - **Call it before anything guest-influenced is read.** This is the child's containment;
///   entering it late is entering it after the window it was for.
///
/// ## Errors
/// [`RawError::Syscall`], naming the step. Every arm is a **refusal**: there is no partial
/// sandbox, because the value this returns is the capability itself. In particular a failed
/// read-only reseal returns `Err` rather than a `DevDir` inside a writable root — the C's
/// R4-L1 audit finding, ported as a type-level consequence rather than as a comment — and
/// so does a capability that survived [`surrender_privilege`]'s read-back.
///
/// # Panics
/// If called with any ranked or adapter-leaf lock held (R1, §4.5) — this mounts filesystems
/// and pivots a root, none of which belongs under a lock.
pub fn enter(policy: &SandboxPolicy) -> Result<DevDir, RawError> {
    lockwitness::assert_lock_free("entering the isolate filesystem sandbox");
    leafwitness::assert_leaf_free("entering the isolate filesystem sandbox");

    acquire_mount_namespace()?;

    // Nothing below may propagate back to the host's mount table.
    mount_(
        c"",
        c"/",
        None,
        libc::MS_REC | libc::MS_PRIVATE,
        None,
        "mount(MS_REC|MS_PRIVATE /)",
    )?;

    // The scratch root. `mode=0755` so it can be populated now and traversed later; the
    // size is a bound on what a compromised isolate could allocate here, and it needs to
    // hold nothing but empty bind targets.
    mount_(
        c"tmpfs",
        SCRATCH,
        Some(c"tmpfs"),
        libc::MS_NOSUID | libc::MS_NOEXEC,
        Some(c"size=256k,mode=0755"),
        "mount(tmpfs scratch root)",
    )?;

    std::fs::create_dir(SCRATCH_DEV).map_err(|e| RawError::Syscall {
        call: "mkdir(sandbox /dev)",
        errno: e.raw_os_error(),
    })?;

    // ★ Validate the WHOLE policy before binding any of it: a refusal half-way through
    // would leave a sandbox that is neither the declared one nor nothing.
    for node in &policy.nodes {
        validate_node_name(&node.name)?;
    }

    for node in &policy.nodes {
        let source = format!("/dev/{}", node.name);
        let target = format!("{SCRATCH_DEV}/{}", node.name);
        if !Path::new(&source).exists() {
            if node.required {
                return Err(RawError::Syscall {
                    call: "sandbox: a required device node is absent",
                    errno: Some(libc::ENOENT),
                });
            }
            continue;
        }
        // A node may name a subdirectory (`dri/renderD128`); create it before the target.
        if let Some(parent) = Path::new(&target).parent() {
            std::fs::create_dir_all(parent).map_err(|e| RawError::Syscall {
                call: "mkdir(sandbox /dev subdirectory)",
                errno: e.raw_os_error(),
            })?;
        }
        // ★ create-empty-file-then-bind, not `mknod`: `mknod` of a character device is
        // refused in a user namespace, an existing node's bind mount is not. The bind
        // deliberately carries no `MS_NODEV`, or the node would stop being openable —
        // which is the entire point of putting it here.
        std::fs::File::create(&target).map_err(|e| RawError::Syscall {
            call: "create(sandbox device bind target)",
            errno: e.raw_os_error(),
        })?;
        let csource = cpath(&source, "sandbox: device node name")?;
        let ctarget = cpath(&target, "sandbox: device node name")?;
        match mount_(
            &csource,
            &ctarget,
            None,
            libc::MS_BIND,
            None,
            "mount(MS_BIND device node)",
        ) {
            Ok(()) => {}
            Err(e) if node.required => return Err(e),
            // An optional node that would not bind leaves an empty regular file behind,
            // and an empty regular file that answers `open` is worse than an absent one:
            // it makes "the node is present" true and "the node works" false. Remove it,
            // so the sandbox contains only nodes that ARE the device.
            Err(_) => {
                let _ = std::fs::remove_file(&target);
            }
        }
    }

    // ★★★ The pivot. `pivot_root(".", ".")` puts the old root *on top of* the new one and
    // the `MNT_DETACH` immediately below unhooks it, which is the idiom that leaves no
    // `put_old` directory inside the sandbox for anything to walk back through.
    std::env::set_current_dir(SCRATCH_STR).map_err(|e| RawError::Syscall {
        call: "chdir(scratch root)",
        errno: e.raw_os_error(),
    })?;
    pivot_root_(c".", c".")?;
    umount_detach(c".")?;
    std::env::set_current_dir("/").map_err(|e| RawError::Syscall {
        call: "chdir(sandbox root)",
        errno: e.raw_os_error(),
    })?;

    // ★★★ AND ONLY NOW the descriptor. Its `..` is the tmpfs above, which holds one
    // directory and no path off this mount. Moving this line one step earlier restores the
    // escape in full — see the module docs and the committed regression test.
    let dev = DevDir::open(SANDBOX_DEV)?;

    // ★ Fail closed. The C ignored this return for a while and the result was a stub
    // running with a writable root and no diagnostic (audit R4-L1). Here the `?` drops
    // `dev` on the way out, so a weakened sandbox does not merely go unreported — the
    // capability it would have weakened is never handed over.
    mount_(
        c"",
        c"/",
        None,
        libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NOEXEC,
        None,
        "mount(MS_REMOUNT|MS_RDONLY sandbox root)",
    )?;

    // ★★★ LAST, and the position is forced: every step above needs `CAP_SYS_ADMIN`, and
    // this is the line that ends it. Nothing this process does from here on can mount, can
    // be `ptrace`d by a same-uid peer, or can regain a capability across an `exec` — and the
    // `?` means a residual privilege drops `dev` on the way out, so the isolate never
    // receives a device descriptor it would have been holding with privilege.
    surrender_privilege()?;

    Ok(dev)
}

// =====================================================================================
// The capability gate — same discipline as `kvm_gate`, and for the same reason
// =====================================================================================

/// ★ Can **this** process create a mount namespace?
///
/// Measured, not inferred. A `geteuid() == 0` test is wrong under a container that dropped
/// `CAP_SYS_ADMIN`, and a `/proc/sys/user/max_user_namespaces` test is wrong under the
/// AppArmor restriction Ubuntu 24.04 ships — so the probe **forks a child and has it try**,
/// which is the only answer that cannot be wrong. The child does nothing but `unshare` and
/// `_exit`, both async-signal-safe; the parent's namespaces are untouched either way.
///
/// Resolved once per process, so a mid-run change cannot make half a test binary disagree
/// about the policy (the discipline [`crate::kvm_gate::kvm_available`] states).
///
/// ★★ `KAYFABE_NO_SANDBOX_NS=1` forces this to report **absent**, so the gated arm is
/// runnable on purpose rather than only by accident of what a machine lacks. It gates
/// *tests*; it does not weaken [`enter`], which has no opt-out at all.
#[must_use]
pub fn namespaces_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        !forced_absent() && (can_unshare(libc::CLONE_NEWNS) || can_unshare(libc::CLONE_NEWUSER))
    })
}

/// ★ Can this process create a **user** namespace specifically?
///
/// A narrower question than [`namespaces_available`] and a separate gate, because the two
/// have different answers on real hosts: a root process in a container with `CAP_SYS_ADMIN`
/// but `user.max_user_namespaces = 0` gets a mount namespace and no user namespace. The
/// tests that assert the *privilege* boundary — a distinct user namespace, and therefore a
/// `ptrace` refusal against the parent — gate on this one; the tests that assert the
/// *filesystem* boundary do not, because [`enter`]'s privileged route contains the
/// filesystem just as well.
#[must_use]
pub fn user_namespaces_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| !forced_absent() && can_unshare(libc::CLONE_NEWUSER))
}

/// ★★ `KAYFABE_NO_SANDBOX_NS=1` forces both gates to report **absent**, so the gated arm is
/// runnable on purpose rather than only by accident of what a machine lacks. It gates
/// *tests*; it does not weaken [`enter`], which has no opt-out at all.
fn forced_absent() -> bool {
    std::env::var_os("KAYFABE_NO_SANDBOX_NS").is_some_and(|v| v == "1")
}

/// Ask the kernel, in a child that is thrown away. The parent's namespaces are untouched
/// whichever way it answers, which is why the question can be asked at all.
fn can_unshare(flags: libc::c_int) -> bool {
    // SAFETY: `fork` in a possibly-threaded process is sound only if the child confines
    // itself to async-signal-safe calls, and this child makes exactly two — `unshare` (via
    // `unshare_`, which allocates nothing and only reads an integer) and `_exit`, which is
    // on POSIX's list and, unlike `exit`, runs no handler and touches no shared allocator
    // state. It cannot return into this function. On the parent side `waitpid` is an
    // ordinary blocking wait on a child that provably exits at once.
    unsafe {
        let pid = libc::fork();
        if pid < 0 {
            return false;
        }
        if pid == 0 {
            libc::_exit(i32::from(unshare_(flags, "probe").is_err()));
        }
        let mut status: libc::c_int = 0;
        if libc::waitpid(pid, &raw mut status, 0) != pid {
            return false;
        }
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
    }
}

/// Emit this test's gate line. Called by [`crate::require_sandbox`]; public because the
/// macro expands at the call site.
///
/// Straight to `stderr`, bypassing libtest's capture, so the **passing** arm is visible too:
/// a gate whose "it ran" marker only appears on failure cannot be counted, and counting both
/// arms is the whole non-vacuity argument.
pub fn report(test: &str, available: bool) {
    report_gate(test, available, "create a mount namespace");
}

/// As [`report`], naming the capability that was missing. Both gates emit the same two
/// countable markers, so CI's floor covers them together.
pub fn report_gate(test: &str, available: bool, needed: &str) {
    use std::io::Write as _;
    let mut err = std::io::stderr();
    let _ = if available {
        writeln!(err, "SANDBOX-GATE: RAN {test}")
    } else {
        writeln!(
            err,
            "SANDBOX-GATE: SKIPPED {test} — this process may not {needed} \
             (the test asserts nothing; this line is the only record that it did not run)"
        )
    };
}

/// Gate the enclosing `#[test]` on a host that permits namespaces —
/// `require_sandbox!("test_name")`.
///
/// Prints `SANDBOX-GATE: RAN <name>` and continues, or prints `SANDBOX-GATE: SKIPPED <name>
/// …` and returns. **Both arms print**, which is what makes the gate countable and therefore
/// non-vacuous.
#[macro_export]
macro_rules! require_sandbox {
    ($name:expr) => {
        let __ns = $crate::sandbox::namespaces_available();
        $crate::sandbox::report($name, __ns);
        if !__ns {
            return;
        }
    };
}

/// Gate the enclosing `#[test]` on a host that permits **user** namespaces —
/// `require_user_namespace!("test_name")`.
///
/// The narrower gate, for the tests that assert the privilege boundary rather than the
/// filesystem one. Same two markers as [`require_sandbox!`], so one CI floor counts both.
#[macro_export]
macro_rules! require_user_namespace {
    ($name:expr) => {
        let __uns = $crate::sandbox::user_namespaces_available();
        $crate::sandbox::report_gate($name, __uns, "create a user namespace");
        if !__uns {
            return;
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The policy is a *declaration*, and the declaration is what a review reads. Asserted
    /// on the accessor rather than on a private field, so it is the same thing a diagnostic
    /// would print.
    #[test]
    fn a_gpu_policy_names_the_control_node_and_exactly_one_gpu() {
        assert_eq!(
            SandboxPolicy::for_gpu(3).names(),
            vec!["nvidiactl", "nvidia3"]
        );
    }

    /// ★ The absence that is a security property, not an omission: `nvidia-uvm` is opened
    /// by the VMM and must never be inside an isolate's sandbox.
    #[test]
    fn a_gpu_policy_never_contains_the_uvm_node() {
        let policy = SandboxPolicy::for_gpu(0);
        let names = policy.names();
        assert!(
            !names.iter().any(|n| n.contains("uvm")),
            "UVM reached an isolate sandbox: {names:?}"
        );
    }

    #[test]
    fn an_empty_policy_contains_nothing_at_all() {
        assert_eq!(SandboxPolicy::empty().names(), Vec::<&str>::new());
    }

    /// A node name is a *name*, and a NUL byte in one is `EINVAL` rather than a truncated
    /// path that silently binds something else.
    #[test]
    fn a_node_name_carrying_a_nul_is_refused_with_einval() {
        assert_eq!(
            cpath("dev\0null", "sandbox: device node name"),
            Err(RawError::Syscall {
                call: "sandbox: device node name",
                errno: Some(libc::EINVAL),
            })
        );
    }

    /// ★★ The policy is the OTHER way into the sandbox, and a name that walks out of `/dev`
    /// would bind an arbitrary host object *inside* it — the same escape, entered from the
    /// declaration instead of from the descriptor. Every shape is refused with the exact
    /// variant, and the legal shapes are asserted too so the check is not simply "no".
    #[test]
    fn a_node_name_that_leaves_dev_is_refused_and_a_plain_one_is_not() {
        let refusal = Err(RawError::Syscall {
            call: "sandbox: a device node name must be a plain path under /dev",
            errno: Some(libc::EINVAL),
        });
        for bad in [
            "..",
            "../etc",
            "../../etc/shadow",
            "dri/../../etc",
            "/etc/shadow",
            "",
            ".",
        ] {
            assert_eq!(validate_node_name(bad), refusal, "{bad:?} was accepted");
        }
        for good in ["nvidiactl", "nvidia0", "dri/renderD128", "null"] {
            assert_eq!(validate_node_name(good), Ok(()), "{good:?} was refused");
        }
    }

    /// Non-vacuity for the gate itself: it must return the same answer twice (it is
    /// memoised) and it must be *some* answer rather than a panic on a host that forbids
    /// namespaces outright.
    #[test]
    fn the_namespace_gate_is_stable_across_calls() {
        assert_eq!(namespaces_available(), namespaces_available());
        assert_eq!(user_namespaces_available(), user_namespaces_available());
    }

    /// ★ The user-namespace gate is the *narrower* question and must never claim more than
    /// the broad one: a host that permits a user namespace permits [`enter`]'s route, so
    /// `user ⇒ any`. The converse is genuinely false on hosts with `CAP_SYS_ADMIN` and
    /// `user.max_user_namespaces = 0`, which is exactly why they are two gates.
    #[test]
    fn the_user_namespace_gate_implies_the_broad_one() {
        assert!(!user_namespaces_available() || namespaces_available());
    }

    /// ★★ The instrument the whole privilege boundary is asserted with, checked against an
    /// **independent** one: `/proc/self/status` is the kernel's own text rendering of the
    /// same state, produced by different code from `capget`/`prctl`. If the two ever
    /// disagree, every capability assertion in `sandbox_escape.rs` is suspect — and a
    /// measurement nobody cross-checks is how a green gate ends up asserting nothing
    /// (`suspect_the_instrument_first`).
    #[test]
    fn the_privilege_reading_agrees_with_proc_self_status() {
        let measured = privileges().expect("privileges are readable on any Linux host");
        let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
        let field = |name: &str| -> Option<u64> {
            status
                .lines()
                .find_map(|l| l.strip_prefix(name))
                .and_then(|v| u64::from_str_radix(v.trim(), 16).ok())
        };
        assert_eq!(field("CapEff:"), Some(measured.effective), "CapEff");
        assert_eq!(field("CapPrm:"), Some(measured.permitted), "CapPrm");
        assert_eq!(field("CapInh:"), Some(measured.inheritable), "CapInh");
        assert_eq!(field("CapBnd:"), Some(measured.bounding), "CapBnd");
        assert_eq!(field("CapAmb:"), Some(measured.ambient), "CapAmb");
        assert_eq!(
            status
                .lines()
                .find_map(|l| l.strip_prefix("NoNewPrivs:"))
                .map(|v| v.trim() == "1"),
            Some(measured.no_new_privs),
            "NoNewPrivs"
        );
    }

    /// A capability set is a bitmap the kernel hands over in two 32-bit halves, and joining
    /// them the wrong way round is the silent way every assertion above starts comparing
    /// the wrong 32 bits. Asserted on a value whose halves differ.
    #[test]
    fn the_two_capability_words_join_high_word_last() {
        assert_eq!(join(0x0000_00ff, 0x0000_0001), 0x0000_0001_0000_00ff);
        assert_eq!(join(0, 0), 0);
    }

    /// ★ `is_unprivileged` is a conjunction, and a conjunction is exactly the shape that
    /// rots into "returns true" when one clause is dropped. Every clause is falsified
    /// individually — a mutation that deletes any one of them turns a row below red.
    #[test]
    fn every_clause_of_the_unprivileged_verdict_is_load_bearing() {
        let clean = Privileges {
            effective: 0,
            permitted: 0,
            inheritable: 0,
            bounding: 0,
            ambient: 0,
            no_new_privs: true,
            dumpable: false,
        };
        assert!(clean.is_unprivileged());
        for (what, p) in [
            (
                "effective",
                Privileges {
                    effective: 1,
                    ..clean
                },
            ),
            (
                "permitted",
                Privileges {
                    permitted: 1,
                    ..clean
                },
            ),
            (
                "inheritable",
                Privileges {
                    inheritable: 1,
                    ..clean
                },
            ),
            (
                "ambient",
                Privileges {
                    ambient: 1,
                    ..clean
                },
            ),
            (
                "no_new_privs",
                Privileges {
                    no_new_privs: false,
                    ..clean
                },
            ),
            (
                "dumpable",
                Privileges {
                    dumpable: true,
                    ..clean
                },
            ),
        ] {
            assert!(!p.is_unprivileged(), "{what} stopped being load-bearing");
        }
        // ★ And the deliberate non-clause: a bounding set alone does not make a process
        // privileged, because `no_new_privs` makes it unusable. `surrender_privilege`
        // applies the stricter rule where it is achievable, and that is where it belongs —
        // here it would refuse every unprivileged host.
        assert!(
            Privileges {
                bounding: u64::MAX,
                ..clean
            }
            .is_unprivileged()
        );
    }

    /// The kernel's last capability number is *discovered*, and the discovery must land in
    /// the plausible range — a 0 would make the drop loop and the read-back both vacuous,
    /// which is the failure mode that reads as success.
    #[test]
    fn the_last_capability_is_discovered_and_plausible() {
        let last = last_capability();
        assert!(
            (14..64).contains(&last),
            "implausible last capability {last}: CAP_SYS_ADMIN alone is 21"
        );
    }

    /// The `/proc/self` writer reports the exact errno, never a bare failure. A control
    /// file that does not exist is `ENOENT` for root and non-root alike, which is why this
    /// is the arm chosen: a privilege-dependent assertion would pass for the wrong reason
    /// on half the machines that run it.
    #[test]
    fn a_missing_proc_self_control_file_reports_enoent_exactly() {
        assert_eq!(
            write_proc_self("kayfabe-no-such-control", "x", "write(probe)"),
            Err(RawError::Syscall {
                call: "write(probe)",
                errno: Some(libc::ENOENT),
            })
        );
    }
}

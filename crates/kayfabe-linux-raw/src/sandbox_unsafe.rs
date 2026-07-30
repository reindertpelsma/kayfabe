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
//! ## What this is NOT
//!
//! Not a capability drop, not seccomp. Those are tracked as the broader sandbox work and
//! are deliberately **named rather than stubbed** (the discipline `spawn_unsafe.rs` states):
//! an untested boundary reads as a boundary in every review that follows. What is here is
//! the filesystem boundary, and it is measured.
//!
//! ## ★ A port finding: why this runs in the child's own `main`, not before `exec`
//!
//! The C does all of this **between `clone` and `fexecve`**, which is strictly better —
//! the exec'd image cannot decline a sandbox it was born in. That is not available here,
//! and the reason is concrete rather than stylistic: the C `fexecve`s a **statically
//! linked** stub out of a `memfd`, so it needs no path and no loader once the old root is
//! gone. `std::process::Command` `execve`s a *path*, and a dynamically linked Rust binary
//! additionally needs `/lib64/ld-linux-*.so` — neither of which survives the pivot. Doing
//! it pre-`exec` therefore requires a static isolate binary; until that exists, the child
//! enters the sandbox as the **first thing it does with its own descriptors** and before it
//! has read one byte of anything a guest influenced.

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

// =====================================================================================
// Entering
// =====================================================================================

/// Acquire a mount namespace, by whichever route this process is entitled to.
///
/// Privileged first (`CAP_SYS_ADMIN` — the posture the isolate actually ships in), then the
/// rootless route: a **user** namespace, in which this process is root and may therefore
/// unshare the mount namespace it could not unshare outside. `setgroups` must be denied
/// before `gid_map` is writable, which is the kernel's rule and not ours.
///
/// ★ There is no third arm. A host that grants neither gets an error, and the caller's
/// contract is that an error means no [`DevDir`] — never a `/dev` handed out uncontained.
fn acquire_mount_namespace() -> Result<(), RawError> {
    if unshare_(libc::CLONE_NEWNS, "unshare(CLONE_NEWNS)").is_ok() {
        return Ok(());
    }
    let (uid, gid) = outer_ids();
    unshare_(libc::CLONE_NEWUSER, "unshare(CLONE_NEWUSER)")?;
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
    unshare_(
        libc::CLONE_NEWNS,
        "unshare(CLONE_NEWNS) after CLONE_NEWUSER",
    )
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

/// ★★★ **Enter the sandbox and mint the `/dev` descriptor inside it.**
///
/// Returns the *only* [`DevDir`] the isolate should ever hold: one whose `..` is a `tmpfs`
/// containing nothing but the nodes `policy` named. On return the process has a private
/// mount namespace, a read-only root, and no path to the host filesystem at all.
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
/// R4-L1 audit finding, ported as a type-level consequence rather than as a comment.
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
        if std::env::var_os("KAYFABE_NO_SANDBOX_NS").is_some_and(|v| v == "1") {
            return false;
        }
        // SAFETY: `fork` in a possibly-threaded process is sound only if the child confines
        // itself to async-signal-safe calls, and this child makes exactly two — `unshare`
        // (via `unshare_`, which allocates nothing and only reads an integer) and `_exit`,
        // which is on POSIX's list and, unlike `exit`, runs no handler and touches no
        // shared allocator state. It cannot return into this function. On the parent side
        // `waitpid` is an ordinary blocking wait on a child that provably exits at once.
        unsafe {
            let pid = libc::fork();
            if pid < 0 {
                return false;
            }
            if pid == 0 {
                let ok = unshare_(libc::CLONE_NEWNS, "probe").is_ok()
                    || unshare_(libc::CLONE_NEWUSER, "probe").is_ok();
                libc::_exit(i32::from(!ok));
            }
            let mut status: libc::c_int = 0;
            if libc::waitpid(pid, &raw mut status, 0) != pid {
                return false;
            }
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
        }
    })
}

/// Emit this test's gate line. Called by [`crate::require_sandbox`]; public because the
/// macro expands at the call site.
///
/// Straight to `stderr`, bypassing libtest's capture, so the **passing** arm is visible too:
/// a gate whose "it ran" marker only appears on failure cannot be counted, and counting both
/// arms is the whole non-vacuity argument.
pub fn report(test: &str, available: bool) {
    use std::io::Write as _;
    let mut err = std::io::stderr();
    let _ = if available {
        writeln!(err, "SANDBOX-GATE: RAN {test}")
    } else {
        writeln!(
            err,
            "SANDBOX-GATE: SKIPPED {test} — this process may not create a mount namespace \
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

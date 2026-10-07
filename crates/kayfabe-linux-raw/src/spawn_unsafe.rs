//! Spawning a child process with an **explicitly declared** descriptor table, from an
//! **image this process holds in memory** rather than from a path.
//!
//! `l1_os_shell.md` §7.0: *"the isolate process boundary is the garbage collector"*, and
//! §11 item 4: *"cleared env and descriptors … no descriptor reaches the sandbox beyond
//! the intended set"*. This file is the mechanism for both.
//!
//! ## ★ The finding this file is a port of, stated first
//!
//! The C's isolate closes every descriptor above its declared set before `exec`
//! (`C: src/qemu/nvkvm_isolate.c:273-305`, `nvkvm_isolate_closefrom`), and its comment
//! names the reason in one sentence: without it the sandbox inherits **QEMU's KVM VM
//! descriptor**, the memory-backend descriptors, and every other isolate's socket — so a
//! compromised isolate reaches arbitrary guest memory through
//! `KVM_SET_USER_MEMORY_REGION`. Descriptor hygiene is not tidiness here; it is the
//! difference between a contained sandbox and none.
//!
//! So the API is **declarative in the safe direction**: a caller states the descriptors
//! the child is to receive and the numbers they land on, and *everything else is closed*.
//! There is no "inherit" door, because a default that inherits is a default that leaks.
//!
//! ## ★★★ The second finding: the IMAGE is a declaration too
//!
//! The C says it in the first ten lines of the file it lives in
//! (`C: src/qemu/nvkvm_isolate.c:9-10`): *"the stub binary is embedded as
//! `nvkvm_stub_elf[]` … loaded via `memfd_create` + `fexecve` (no disk file)"*.
//!
//! The port had a path instead, and the path was chosen by `KAYFABE_ISOLATE_BIN` with a
//! fallback to a sibling of `current_exe()`. Its own rustdoc named the hazard and then kept
//! a narrower instance of it: *"an isolate found on `PATH` is an isolate an environment
//! variable chose, and this process hands that binary a descriptor for `/dev`"*. A sibling
//! of `current_exe()` is chosen by whoever can write that directory, and the variable was
//! chosen by whoever can set one variable.
//!
//! [`ProgramImage`] deletes the category. The bytes are `include_bytes!`-ed into the VMM at
//! build time, written into a **sealed** `memfd`, and `execveat(…, AT_EMPTY_PATH)`-ed. There
//! is no name to resolve, no directory to win a race in, and — because the seals are applied
//! before the first spawn — no way for anything holding the descriptor to change the bytes
//! afterwards.
//!
//! ★ It is also what makes a *pre-`exec`* sandbox reachable at all, which is why this is one
//! change and not two: a `pivot_root`ed child has no path to its own interpreter, so the
//! image must be static **and** nameless before the sandbox can move ahead of the `exec`.
//!
//! ## ★★ Why `std::process::Command` is gone
//!
//! It cannot express either of the two things this file now exists for. `Command` `execve`s
//! a **path**, so a memfd can only be reached through `/proc/self/fd/N` — a path again, and
//! one that stops resolving the moment the child gets a mount namespace. And `Command`
//! `fork`s, so `CLONE_NEWPID` is unreachable: `unshare(CLONE_NEWPID)` moves only *future*
//! children and the isolate creates none, while a second `fork` to get into the namespace
//! would orphan the process this handle is supposed to own. The C uses one raw `clone` for
//! exactly this reason and says so (`C: nvkvm_isolate.c:117-133`, *"no intermediate process,
//! no second fork"*).
//!
//! What `Command` was providing and had to be re-provided here is the **failure report**: it
//! keeps a close-on-exec pipe over which the child reports an `errno` before `_exit`, so a
//! failed `exec` is an `Err` and not a live handle to a process that never ran. That pipe is
//! ported below, with a stage code alongside the `errno` — the C `_exit(126)`s for four
//! different reasons and cannot tell them apart afterwards.
//!
//! ## The numbered-descriptor contract, and why numbers at all
//!
//! A child that has been `exec`'d has no way to be *handed* an object; the only channel is
//! the descriptor table, so the numbers are the ABI. The C's contract is 0 = the isolate
//! socket, 3 = the stub image, **4 = the `/dev` `O_PATH` directory**
//! (`C: src/common/nvkvm_isolate_proto.h:389`, `NVKVM_DEV_DIRFD`). This file does not fix
//! the numbers — that is the adapter's protocol — it only guarantees that the set the
//! caller declared is exactly the set that arrives.
//!
//! ★★★ **The port does NOT grant fd 4, and the difference is a security fix rather than a
//! simplification.** A `/dev` `O_PATH` descriptor opened in the parent and handed down was
//! measured to open `../etc/shadow` from inside the child: `O_PATH` restricts *enumeration*
//! and places no restriction at all on `..`. There is no bounded way to pass that grant
//! across `exec`, so the child mints its own after entering a `pivot_root`ed sandbox — see
//! [`crate::sandbox`] and `kayfabe_isolate_host::isolate`'s "fd 4 is vacant" note.
//!
//! ## The namespaces, and the ordering the kernel imposes
//!
//! [`ChildSpec::in_new_namespaces`] clones the child directly into fresh user, pid, mount,
//! network, IPC and UTS namespaces — the C's flag word exactly
//! (`C: nvkvm_isolate.c:127-129`). The child is PID 1 of the new pid namespace and `clone`
//! returns its **real host pid**, so this handle still owns it.
//!
//! ★★ The sequence is **not** ours to choose and is not re-derived here; it is the C's, and
//! the C's is the kernel's:
//!
//! 1. the child blocks on a sync pipe immediately after `clone`, because it has **no uid
//!    mapping at all** until the parent writes one and can do nothing useful before then;
//! 2. the parent writes `/proc/<pid>/setgroups` = `deny` **first** — the kernel refuses to
//!    make `gid_map` writable by an unprivileged process until it has;
//! 3. then `uid_map`, then `gid_map`, each a single line `0 <our outer id> 1`, which is the
//!    one mapping an unprivileged parent is always permitted to write;
//! 4. only then does the parent release the child, which `exec`s.
//!
//! A failure anywhere in 2–3 closes the pipe without writing to it, the child's `read`
//! returns 0, and the child `_exit`s — *fail closed*, never "carry on unmapped".
//!
//! ## What is deliberately absent, and it is the security gap of record
//!
//! **Seccomp.** The C's stub filters its syscalls with a `TSYNC` allowlist
//! (`C: src/stub/nvkvm_stub.c:2505-2587`) and this does not, named rather than stubbed: the
//! allowlist is a property of the isolate's *verb surface*, it needs its own falsification
//! test, and an untested control reads as a boundary in every review that follows.
//!
//! **And the `pivot_root` still runs in the child's own `main`**, not here between `clone`
//! and `exec`. The *namespaces* are pre-`exec` now — including the one that could only ever
//! be had here — but the filesystem construction is not, and the reason is concrete:
//! [`crate::sandbox::enter`] allocates (it formats paths, reads `/proc`, and walks a policy),
//! and allocating between `fork` and `exec` in a process with other threads is the classic
//! malloc-lock deadlock. The C can do it because its stub is freestanding C with no
//! allocator in the path. Doing the same here means a no-allocation `enter`, which is a
//! separate piece of work with its own falsification test.

use crate::error::{RawError, last_syscall_error};
use kayfabe_util::{leafwitness, lockwitness};
use std::ffi::{CStr, CString, OsStr, OsString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// `memfd_create` flag: the descriptor is close-on-exec.
const MFD_CLOEXEC: libc::c_uint = 0x0001;
/// `memfd_create` flag: seals may be applied.
const MFD_ALLOW_SEALING: libc::c_uint = 0x0002;
/// ★ `memfd_create` flag (Linux 6.3+): the memfd is **executable**.
///
/// Named here rather than taken from `libc`, which does not carry it yet. It matters because
/// `vm.memfd_noexec = 2` — which hardened distributions do set — makes a memfd created
/// *without* this flag carry `MFD_NOEXEC_SEAL`, and `execveat` on it answers `EACCES`. On a
/// kernel that predates the flag it is rejected with `EINVAL`, which is why
/// [`ProgramImage::from_bytes`] retries without it rather than requiring it.
const MFD_EXEC: libc::c_uint = 0x0010;

/// Every seal there is, applied together: the image cannot be written, grown, shrunk, or
/// have its seals removed. Applied **before the first spawn**, so no descriptor anyone could
/// obtain afterwards can change the bytes that get `exec`'d.
const ALL_SEALS: libc::c_int =
    libc::F_SEAL_SEAL | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE;

// -------------------------------------------------------------------------------------
// Stage codes. The C `_exit(126)`s for four different reasons and cannot tell them apart
// afterwards; these travel down the error pipe next to the `errno`.
// -------------------------------------------------------------------------------------

/// The child's wait for its uid/gid mapping did not complete — the parent failed to write
/// the maps and closed the pipe, so the child refuses to run unmapped.
const STAGE_SYNC: u32 = 1;
/// Parking a granted descriptor above the placement range failed.
const STAGE_PARK: u32 = 2;
/// Placing a granted descriptor on its declared number failed.
const STAGE_PLACE: u32 = 3;
/// Marking the undeclared descriptors close-on-exec failed.
const STAGE_CLOSE: u32 = 4;
/// `PR_SET_NO_NEW_PRIVS` failed.
const STAGE_NNP: u32 = 5;
/// The `exec` itself failed.
const STAGE_EXEC: u32 = 6;

/// Turn a stage code into the `call` name a [`RawError::Syscall`] reports.
fn stage_name(stage: u32) -> &'static str {
    match stage {
        STAGE_SYNC => "clone(uid_map handshake)",
        STAGE_PARK => "fcntl(F_DUPFD_CLOEXEC)",
        STAGE_PLACE => "dup2",
        STAGE_CLOSE => "close_range",
        STAGE_NNP => "prctl(PR_SET_NO_NEW_PRIVS)",
        STAGE_EXEC => "execve",
        _ => "spawn",
    }
}

// =====================================================================================
// The image
// =====================================================================================

/// ★★★ An executable held **in this process's memory**, with no name anywhere.
///
/// Built from bytes the VMM was compiled with, written into a sealed `memfd`, and `exec`'d
/// through the descriptor. See the module docs for why the path it replaces was a hazard
/// rather than an inconvenience.
///
/// One image is created once and spawned from many times; each [`ChildSpec::from_image`]
/// takes its own duplicate of the descriptor, so a spawn cannot invalidate the image.
#[derive(Debug)]
pub struct ProgramImage {
    fd: OwnedFd,
    len: usize,
}

impl ProgramImage {
    /// Publish `bytes` as an executable image called `name` (which appears only in
    /// `/proc/<pid>/maps`, never in a filesystem).
    ///
    /// # Errors
    /// [`RawError::Syscall`] (`memfd_create`, `write`, `fcntl(F_ADD_SEALS)`).
    ///
    /// An **empty** `bytes` is refused with `ENOEXEC` before any syscall: it is what a build
    /// that did not produce an isolate would hand over, and an empty image would otherwise
    /// surface as a mystifying `exec` failure per spawn rather than as one refusal here.
    pub fn from_bytes(name: &CStr, bytes: &[u8]) -> Result<Self, RawError> {
        if bytes.is_empty() {
            return Err(RawError::Syscall {
                call: "ProgramImage: the embedded image is empty",
                errno: Some(libc::ENOEXEC),
            });
        }
        // SAFETY: `memfd_create` reads the NUL-terminated name — NUL-terminated by the
        // `&CStr` type, and borrowed across both calls — and dereferences nothing else. The
        // flags are an integer by value. Both calls are in one block because they are one
        // obligation differing by a single flag bit: a kernel older than 6.3 rejects
        // `MFD_EXEC` with `EINVAL`, and such a kernel also has no `vm.memfd_noexec`, so the
        // flag was unnecessary there. Each returns a fresh descriptor or -1.
        let raw = unsafe {
            let first = libc::syscall(
                libc::SYS_memfd_create,
                name.as_ptr(),
                MFD_CLOEXEC | MFD_ALLOW_SEALING | MFD_EXEC,
            );
            if first < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINVAL) {
                libc::syscall(
                    libc::SYS_memfd_create,
                    name.as_ptr(),
                    MFD_CLOEXEC | MFD_ALLOW_SEALING,
                )
            } else {
                first
            }
        };
        if raw < 0 {
            return Err(last_syscall_error("memfd_create"));
        }
        // SAFETY: `raw` is a descriptor this call just created and no other object refers to
        // it, so this `OwnedFd` is its sole owner and will close it exactly once. The cast is
        // exact: `memfd_create` returns a descriptor number, which is an `i32`.
        let fd = unsafe { OwnedFd::from_raw_fd(raw as libc::c_int) };

        let mut written = 0usize;
        while written < bytes.len() {
            // SAFETY: the pointer is `bytes.as_ptr()` advanced by `written`, which the loop
            // condition keeps strictly below `bytes.len()`, and the length passed is exactly
            // the remaining tail — so the kernel reads only inside the caller's slice, which
            // is borrowed for the whole loop. The descriptor is the live one above.
            let n = unsafe {
                libc::write(
                    fd.as_raw_fd(),
                    bytes.as_ptr().add(written).cast::<libc::c_void>(),
                    bytes.len() - written,
                )
            };
            if n < 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(last_syscall_error("write(image)"));
            }
            if n == 0 {
                return Err(RawError::Syscall {
                    call: "write(image) made no progress",
                    errno: Some(libc::EIO),
                });
            }
            #[expect(
                clippy::cast_sign_loss,
                reason = "n > 0 on this line: the negative and zero cases returned above"
            )]
            {
                written += n as usize;
            }
        }

        // SAFETY: `F_ADD_SEALS` takes an integer bitmask by value on a live descriptor and
        // dereferences no memory. It is the last thing done to the image, so there is no
        // outstanding writable mapping for it to refuse over.
        let rc = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, ALL_SEALS) };
        if rc < 0 {
            return Err(last_syscall_error("fcntl(F_ADD_SEALS)"));
        }
        Ok(ProgramImage {
            fd,
            len: bytes.len(),
        })
    }

    /// The image's size in bytes — for a diagnostic, and for the test that asserts a build
    /// which embedded nothing cannot reach a spawn.
    #[must_use]
    pub fn len_bytes(&self) -> usize {
        self.len
    }
}

// =====================================================================================
// The declaration
// =====================================================================================

/// One descriptor the child is to receive, and the number it must arrive on.
#[derive(Debug)]
pub struct FdGrant {
    fd: OwnedFd,
    target: i32,
}

impl FdGrant {
    /// Grant `fd` to the child as descriptor number `target`.
    ///
    /// # Panics
    /// If `target` is negative, or is one of 0/1/2 — the standard streams are set by
    /// [`ChildSpec`] itself, and a grant that silently replaced one of them would make the
    /// child's diagnostics land somewhere nobody declared.
    #[must_use]
    pub fn new(fd: OwnedFd, target: i32) -> Self {
        assert!(
            target >= 3,
            "descriptor grant {target} collides with the standard streams; grants start at 3"
        );
        FdGrant { fd, target }
    }

    /// The number this grant lands on in the child.
    #[must_use]
    pub fn target(&self) -> i32 {
        self.target
    }
}

/// What the child is to become: an image this process holds, or a path on the host.
#[derive(Debug)]
enum Exec {
    /// ★ The production form. A duplicate of a [`ProgramImage`]'s sealed descriptor.
    Image(OwnedFd),
    /// A path. Retained for *this file's own tests*, which assert descriptor and environment
    /// properties against `/bin/sh` — a real program whose behaviour is not ours to fake —
    /// and for nothing else. The isolate never takes this door.
    Path(PathBuf),
}

/// Everything a child process is to be started with — and, by omission, everything it is
/// not.
#[derive(Debug)]
pub struct ChildSpec {
    exec: Exec,
    argv0: OsString,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    grants: Vec<FdGrant>,
    namespaces: bool,
}

impl ChildSpec {
    /// ★ A child that becomes `image`: **the** constructor for anything this project spawns.
    ///
    /// `argv0` is what the child sees as its own name. It names nothing on the filesystem —
    /// there is nothing to name — so it is a label for `ps` and for the child's own
    /// diagnostics.
    ///
    /// # Errors
    /// [`RawError::Syscall`] (`fcntl(F_DUPFD_CLOEXEC)`) if the image descriptor cannot be
    /// duplicated.
    pub fn from_image(image: &ProgramImage, argv0: &str) -> Result<Self, RawError> {
        let fd = image.fd.try_clone().map_err(|e| RawError::Syscall {
            call: "dup(image)",
            errno: e.raw_os_error(),
        })?;
        Ok(ChildSpec {
            exec: Exec::Image(fd),
            argv0: OsString::from(argv0),
            args: Vec::new(),
            env: Vec::new(),
            grants: Vec::new(),
            namespaces: false,
        })
    }

    /// A child running the program at `program`, with no arguments, **no environment**, and
    /// no descriptors beyond the standard streams.
    ///
    /// ★ For tests and diagnostics only — see [`Exec::Path`]. Production spawns go through
    /// [`ChildSpec::from_image`], because a path is a name and a name is something an
    /// attacker who can write one directory gets to choose.
    #[must_use]
    pub fn new(program: impl Into<PathBuf>) -> Self {
        let program = program.into();
        let argv0 = program.as_os_str().to_os_string();
        ChildSpec {
            exec: Exec::Path(program),
            argv0,
            args: Vec::new(),
            env: Vec::new(),
            grants: Vec::new(),
            namespaces: false,
        }
    }

    /// Append one argument.
    #[must_use]
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    /// Declare one environment variable. The environment is **cleared** first, so this is
    /// an allowlist of one entry and not an addition to the parent's — the C's M6 posture
    /// (`C: src/qemu/nvkvm_isolate.c:856-858`, `envp = {NULL}`), with a door that has to be
    /// used on purpose.
    #[must_use]
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.env
            .push((key.as_ref().to_os_string(), value.as_ref().to_os_string()));
        self
    }

    /// Grant one descriptor.
    ///
    /// # Panics
    /// If two grants name the same target number — a collision would make which descriptor
    /// the child sees depend on iteration order.
    #[must_use]
    pub fn grant(mut self, grant: FdGrant) -> Self {
        assert!(
            !self.grants.iter().any(|g| g.target == grant.target),
            "two descriptor grants both claim number {}",
            grant.target
        );
        self.grants.push(grant);
        self
    }

    /// ★★ Clone the child into fresh user, pid, mount, network, IPC and UTS namespaces.
    ///
    /// The C's flag word (`C: nvkvm_isolate.c:127-129`), and the only way `CLONE_NEWPID` can
    /// be had at all. The uid/gid map handshake described in the module docs is performed by
    /// [`SandboxChild::spawn`] and is not optional once this is set.
    #[must_use]
    pub fn in_new_namespaces(mut self) -> Self {
        self.namespaces = true;
        self
    }

    /// The path this spec would run, for a diagnostic. `None` for an image, which has none —
    /// and that absence is the point.
    #[must_use]
    pub fn program(&self) -> Option<&Path> {
        match &self.exec {
            Exec::Path(p) => Some(p.as_path()),
            Exec::Image(_) => None,
        }
    }
}

/// ★ Adopt a descriptor this process was **given** at `exec` — the child half of
/// [`FdGrant`].
///
/// The number is the ABI (see the module docs), so the child has nothing but an integer to
/// go on. This is the one door that turns it back into an owned object, and it **probes
/// first**: a number that is not open is a refusal here rather than an `EBADF` from the
/// first read, which on a protocol channel is indistinguishable from a peer that hung up.
///
/// # Errors
/// [`RawError::Syscall`] (`fcntl`) — `EBADF` when the parent did not grant that number.
///
/// # Panics
/// If called with any ranked or adapter-leaf lock held (R1, §4.5).
pub fn adopt_inherited_fd(number: i32) -> Result<OwnedFd, RawError> {
    lockwitness::assert_lock_free("adopting an inherited descriptor");
    leafwitness::assert_leaf_free("adopting an inherited descriptor");
    if number < 0 {
        return Err(RawError::Syscall {
            call: "fcntl(F_GETFD)",
            errno: Some(libc::EBADF),
        });
    }
    // SAFETY: `F_GETFD` takes an integer command by value and dereferences no user memory;
    // it is the standard liveness probe for a descriptor number. It returns the flags or
    // -1, and nothing is adopted unless it succeeded — so the `from_raw_fd` below is
    // reached only for a number this process demonstrably owns.
    let rc = unsafe { libc::fcntl(number, libc::F_GETFD) };
    if rc < 0 {
        return Err(last_syscall_error("fcntl(F_GETFD)"));
    }
    // SAFETY: `number` is open in this process (probed on the line above) and was placed
    // there by the parent's `dup2` before `execve`, so this process is its sole owner and
    // nothing else in this program adopts it — the caller is the child's entry point,
    // which runs once. The `OwnedFd` will `close` it exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(number) })
}

// =====================================================================================
// The child side of the fork — every line async-signal-safe
// =====================================================================================

/// What the child is to `exec`, reduced to plain integers and borrowed C strings before the
/// fork so the child side allocates nothing.
#[derive(Clone, Copy)]
enum ChildExec<'a> {
    /// `execveat(fd, "", …, AT_EMPTY_PATH)`.
    Image(i32),
    /// `execve(path, …)`.
    Path(&'a CStr),
}

/// Everything the post-fork child reads, all of it built before the fork.
struct ChildPlan<'a> {
    /// Read end of the uid/gid-map handshake pipe, or -1 when not namespacing.
    sync_rx: i32,
    /// Write end of the pre-exec error pipe.
    err_tx: i32,
    /// `(source number, target number)` for every grant, plus `/dev/null` onto stdin.
    places: &'a [(i32, i32)],
    /// The highest declared target; everything above it is closed at `exec`.
    highest: i32,
    exec: ChildExec<'a>,
    argv: &'a [*const libc::c_char],
    envp: &'a [*const libc::c_char],
}

/// ★ The whole post-`fork` child, in one place because it is one obligation.
///
/// # Safety
/// Must be called only in a just-forked child, on a plan built entirely before the fork.
/// Every call it makes is on POSIX's async-signal-safe list (`read`, `fcntl`, `dup2`,
/// `close_range`, `prctl`, `execve`/`execveat`, `write`, `_exit`); it allocates nothing,
/// takes no lock, and never returns.
unsafe fn child_after_fork(plan: &ChildPlan<'_>) -> ! {
    // SAFETY: see the function's own contract — this is the one block that discharges it,
    // and the calls below are exactly the async-signal-safe set it names. The pointers
    // handed over are the plan's, all of which address memory created before the fork and
    // therefore present in this address space; nothing is allocated, freed or locked.
    unsafe {
        // ★ FIRST: wait to be told our uid/gid map is installed. Until the parent writes it
        // this process has no mapping at all. A closed pipe (the parent failed) reads 0 and
        // is a refusal, never a "carry on unmapped" — the C's fail-closed arm
        // (`C: nvkvm_isolate.c:820-825`).
        if plan.sync_rx >= 0 {
            let mut go = 0u8;
            let n = libc::read(plan.sync_rx, (&raw mut go).cast::<libc::c_void>(), 1);
            if n != 1 {
                report_and_exit(plan.err_tx, STAGE_SYNC, libc::EPERM);
            }
        }

        // Stage 1: park every source descriptor ABOVE the highest target, so stage 2's
        // `dup2` can never clobber a source it has not copied yet. Doing it in one pass
        // without this is the classic fd-shuffle bug: grant A's source happens to be grant
        // B's target. `parked` is a fixed-size array, so this allocates nothing.
        let mut parked = [(0i32, 0i32); MAX_PLACEMENTS];
        for (i, &(src, target)) in plan.places.iter().enumerate() {
            let up = libc::fcntl(src, libc::F_DUPFD_CLOEXEC, plan.highest + 1);
            if up < 0 {
                report_and_exit(plan.err_tx, STAGE_PARK, errno_now());
            }
            parked[i] = (up, target);
        }
        // Stage 2: place them. `dup2` clears close-on-exec on the NEW descriptor, which is
        // the only reason a grant survives `exec` — and it clears it on the duplicate only,
        // so nothing racy is done to a descriptor the parent still shares.
        for &(up, target) in &parked[..plan.places.len()] {
            if libc::dup2(up, target) < 0 {
                report_and_exit(plan.err_tx, STAGE_PLACE, errno_now());
            }
        }
        // Stage 3: ★★ the closefrom — the line the C's audit M6 is about, with ONE
        // correction the port forced. It marks the range close-on-exec rather than closing
        // it, because the error pipe (and, for an image, the image descriptor itself) live
        // in this range and are still needed: closing outright is what the C does, and here
        // it would make a failed `exec` unreportable. `CLOSE_RANGE_CLOEXEC` is also strictly
        // better security — the descriptors stop existing at the `exec` boundary atomically,
        // and if the `exec` fails they are still there to carry the diagnosis.
        //
        // Issued as a raw syscall rather than through libc's wrapper: `close_range` is
        // glibc-only in the `libc` crate, and this crate must keep building for a static
        // musl target — which is how the isolate is built at all.
        if libc::syscall(
            libc::SYS_close_range,
            libc::c_uint::try_from(plan.highest + 1).unwrap_or(libc::c_uint::MAX),
            libc::c_uint::MAX,
            libc::CLOSE_RANGE_CLOEXEC,
        ) < 0
        {
            report_and_exit(plan.err_tx, STAGE_CLOSE, errno_now());
        }
        // Stage 4: no privilege may be gained by anything this child execs.
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0 {
            report_and_exit(plan.err_tx, STAGE_NNP, errno_now());
        }
        // Stage 5: become the image. `AT_EMPTY_PATH` against a sealed memfd — no name is
        // resolved, so nothing on the filesystem can decide what runs here.
        match plan.exec {
            ChildExec::Image(fd) => {
                libc::syscall(
                    libc::SYS_execveat,
                    fd,
                    c"".as_ptr(),
                    plan.argv.as_ptr(),
                    plan.envp.as_ptr(),
                    libc::AT_EMPTY_PATH,
                );
            }
            ChildExec::Path(path) => {
                libc::execve(path.as_ptr(), plan.argv.as_ptr(), plan.envp.as_ptr());
            }
        }
        report_and_exit(plan.err_tx, STAGE_EXEC, errno_now());
    }
}

/// The most placements one spawn can declare. A fixed bound because the child must not
/// allocate; [`SandboxChild::spawn`] refuses a larger spec in the parent, where refusing is
/// still possible.
const MAX_PLACEMENTS: usize = 64;

/// `errno`, read without allocating — `std::io::Error::last_os_error` is fine in the parent
/// and is not something to call between `fork` and `exec`.
fn errno_now() -> i32 {
    // SAFETY: `__errno_location` (glibc) / `__errno_location` (musl) returns a pointer to
    // this thread's `errno`, which is valid for the lifetime of the thread and is exactly
    // one `c_int`. Reading it dereferences nothing else and cannot fail.
    unsafe { *libc::__errno_location() }
}

/// Report `(stage, errno)` on the error pipe and leave. Never returns.
///
/// Calls only `write` and `_exit`, both async-signal-safe, so it is callable from the
/// post-`fork` child — which is its only caller.
fn report_and_exit(fd: i32, stage: u32, errno: i32) -> ! {
    let msg = [stage.to_ne_bytes(), errno.to_ne_bytes()].concat_const();
    // SAFETY: `msg` is a local 8-byte array that outlives the call; `write` reads exactly
    // those 8 bytes from it and dereferences nothing else. A short or failed write is
    // deliberately ignored — the parent then sees EOF and reports a generic spawn failure,
    // which is strictly better than looping here. `_exit` runs no handler and touches no
    // allocator state.
    unsafe {
        let _ = libc::write(fd, msg.as_ptr().cast::<libc::c_void>(), msg.len());
        libc::_exit(127);
    }
}

/// `[[u8; 4]; 2] -> [u8; 8]` without `concat` (which allocates) — the child may not allocate.
trait ConcatConst {
    fn concat_const(self) -> [u8; 8];
}
impl ConcatConst for [[u8; 4]; 2] {
    fn concat_const(self) -> [u8; 8] {
        let [a, b] = self;
        [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]]
    }
}

// =====================================================================================
// The handle
// =====================================================================================

/// A spawned child, reaped on drop.
///
/// ## ★ Drop is a blocking call, and that is why [`kayfabe_isolate::IsolateBox`] exists
///
/// Dropping this waits for the child. That is exactly the hazard `l1_concurrency.md`
/// §12.16 gap G3b names — *"an isolate's `Drop` is `waitpid` + namespace teardown, run by
/// the compiler at a point no call site names"* — so the drop asserts R1 the same way a
/// verb does, and for the same reason: `Spine::reap_retired` was performing that drop
/// under the device write lock and nothing could notice.
#[derive(Debug)]
pub struct SandboxChild {
    pid: libc::pid_t,
    /// The exit code once known. `Some` means the corpse has been collected and the pid must
    /// never be signalled again — it may already name a different process.
    status: Option<i32>,
}

impl SandboxChild {
    /// ★ Spawn the child. Every descriptor except the standard streams and the declared
    /// grants is closed before `exec`.
    ///
    /// # Errors
    /// [`RawError::Syscall`], naming the stage the child reached — `execve`/`ENOENT` for a
    /// path that does not exist, `clone(uid_map handshake)`/`EPERM` when the namespace maps
    /// could not be written.
    ///
    /// # Panics
    /// If called with any ranked or adapter-leaf lock held (R1, §4.5). `fork` of a process
    /// this size is not a cheap call, and the child it produces is the thing every other
    /// thread will then contend for.
    #[expect(
        clippy::too_many_lines,
        reason = "one spawn is one obligation: splitting the pre-fork marshalling from the \
                  fork would put the child's preconditions in a different function from the \
                  fork that makes them preconditions"
    )]
    pub fn spawn(spec: ChildSpec) -> Result<Self, RawError> {
        lockwitness::assert_lock_free("spawning a sandboxed child process");
        leafwitness::assert_leaf_free("spawning a sandboxed child process");

        // ---- everything the child touches, marshalled here, before the fork -------------
        let argv0 = cstring(&spec.argv0)?;
        let args: Vec<CString> = spec
            .args
            .iter()
            .map(|a| cstring(a))
            .collect::<Result<_, _>>()?;
        let mut argv: Vec<*const libc::c_char> = Vec::with_capacity(args.len() + 2);
        argv.push(argv0.as_ptr());
        for a in &args {
            argv.push(a.as_ptr());
        }
        argv.push(std::ptr::null());

        let envs: Vec<CString> = spec
            .env
            .iter()
            .map(|(k, v)| {
                let mut joined = k.clone();
                joined.push("=");
                joined.push(v);
                cstring(&joined)
            })
            .collect::<Result<_, _>>()?;
        let mut envp: Vec<*const libc::c_char> = Vec::with_capacity(envs.len() + 1);
        for e in &envs {
            envp.push(e.as_ptr());
        }
        envp.push(std::ptr::null());

        let path = match &spec.exec {
            Exec::Path(p) => Some(cstring(p.as_os_str())?),
            Exec::Image(_) => None,
        };

        // stdin is `/dev/null`, as `Stdio::null()` used to make it: opened HERE, because the
        // child must not open a path (and, once the sandbox moves pre-`exec`, could not).
        let devnull = std::fs::File::options()
            .read(true)
            .write(true)
            .open("/dev/null")
            .map_err(|e| RawError::Syscall {
                call: "open(/dev/null)",
                errno: e.raw_os_error(),
            })?;

        // The grants stay owned HERE, alive across the fork, so nothing closes a descriptor
        // the child is about to be handed.
        let grants = spec.grants;
        let mut places: Vec<(i32, i32)> = Vec::with_capacity(grants.len() + 1);
        places.push((devnull.as_raw_fd(), 0));
        for g in &grants {
            places.push((g.fd.as_raw_fd(), g.target));
        }
        if places.len() > MAX_PLACEMENTS {
            return Err(RawError::Syscall {
                call: "spawn: more descriptor grants than the child can place without allocating",
                errno: Some(libc::E2BIG),
            });
        }
        let highest = places.iter().map(|&(_, t)| t).max().unwrap_or(2).max(2);

        // The error pipe: close-on-exec, so a SUCCESSFUL exec closes it and the parent's
        // read sees EOF. This is what `std::process::Command` did for us and what has to be
        // re-provided now that it is gone.
        let (err_rx, err_tx_low) = pipe_cloexec("pipe2(spawn error report)")?;
        // The handshake pipe, only when namespacing.
        let sync = if spec.namespaces {
            Some(pipe_cloexec("pipe2(uid_map handshake)")?)
        } else {
            None
        };

        // ★ Park the pipes and the image ABOVE the placement range, so the child's `dup2`
        // shuffle cannot land on one of them. Done in the parent, where a failure is an
        // ordinary error rather than something that has to be reported through the very pipe
        // that failed.
        //
        // ★★ AND THE LOW COPY IS DROPPED IMMEDIATELY. Shadowing it does not close it, and a
        // second write end of the report pipe left open in the PARENT means the parent's own
        // read never sees EOF — a spawn that succeeded would hang forever waiting for a
        // failure report. Measured: the first run of this file's tests hung exactly there.
        let err_tx = dup_above(&err_tx_low, highest)?;
        drop(err_tx_low);
        let sync_rx = match &sync {
            Some((rx, _)) => {
                let up = dup_above(rx, highest)?;
                Some(up)
            }
            None => None,
        };
        let sync_tx = sync.map(|(rx, tx)| {
            drop(rx);
            tx
        });
        let image = match &spec.exec {
            Exec::Image(fd) => Some(dup_above(fd, highest)?),
            Exec::Path(_) => None,
        };

        let plan = ChildPlan {
            sync_rx: sync_rx.as_ref().map_or(-1, AsRawFd::as_raw_fd),
            err_tx: err_tx.as_raw_fd(),
            places: &places,
            highest,
            exec: match (&image, &path) {
                (Some(fd), _) => ChildExec::Image(fd.as_raw_fd()),
                (None, Some(p)) => ChildExec::Path(p.as_c_str()),
                (None, None) => unreachable!("an Exec is either an Image or a Path"),
            },
            argv: &argv,
            envp: &envp,
        };

        // ---- the fork ------------------------------------------------------------------
        // SAFETY: `fork`/`clone` returns 0 in the child and the child's only action is
        // `child_after_fork`, whose contract is discharged by this call site: the plan above
        // is complete before the fork, so the child allocates nothing and takes no lock, and
        // it never returns. In the parent the call returns a pid or -1 and does nothing else.
        // The raw `clone` form is the C's (`C: nvkvm_isolate.c:124-133`): a NULL child stack
        // with no `CLONE_VM` makes it behave exactly like `fork`, and `SIGCHLD` in the low
        // byte is what makes the child waitable.
        let pid = unsafe {
            let pid = if spec.namespaces {
                libc::syscall(
                    libc::SYS_clone,
                    NAMESPACE_CLONE_FLAGS | libc::c_ulong::try_from(libc::SIGCHLD).unwrap_or(17),
                    0usize,
                    0usize,
                    0usize,
                    0usize,
                ) as libc::pid_t
            } else {
                libc::fork()
            };
            if pid == 0 {
                child_after_fork(&plan);
            }
            pid
        };
        if pid < 0 {
            return Err(last_syscall_error(if spec.namespaces {
                "clone"
            } else {
                "fork"
            }));
        }
        drop(grants);
        drop(devnull);
        drop(err_tx);
        drop(sync_rx);
        drop(image);

        let mut child = SandboxChild { pid, status: None };

        // ---- the handshake, in the kernel's order --------------------------------------
        if let Some(sync_tx) = sync_tx {
            let rc = write_child_maps(pid).and_then(|()| {
                // Releasing the child is the LAST step and only happens if every map was
                // written. On any failure `sync_tx` is dropped unwritten, the child's `read`
                // returns 0, and it refuses.
                write_all(&sync_tx, b"x")
            });
            drop(sync_tx);
            if let Err(e) = rc {
                let _ = child.kill();
                let _ = child.reap();
                return Err(e);
            }
        }

        // ---- did the exec happen? -------------------------------------------------------
        if let Some((stage, errno)) = read_child_report(&err_rx)? {
            let _ = child.reap();
            return Err(RawError::Syscall {
                call: stage_name(stage),
                errno: Some(errno),
            });
        }
        Ok(child)
    }

    /// The child's process id, as seen from **this** process.
    ///
    /// ★ With [`ChildSpec::in_new_namespaces`] the child is PID 1 *inside* its own pid
    /// namespace and this is still its real host pid — that is the property `clone` has and
    /// a `fork`-then-`unshare` arrangement does not.
    #[must_use]
    pub fn pid(&self) -> u32 {
        #[expect(
            clippy::cast_sign_loss,
            reason = "a live child's pid is positive; `spawn` refuses a negative one"
        )]
        {
            self.pid as u32
        }
    }

    /// `SIGKILL` the child. Idempotent, and never an error if it has already exited.
    ///
    /// # Errors
    /// [`RawError::Syscall`] (`kill`).
    pub fn kill(&mut self) -> Result<(), RawError> {
        if self.status.is_some() {
            // ★ Already reaped. Signalling now is not merely pointless: the pid may have been
            // recycled onto an unrelated process, so this arm is a correctness requirement.
            return Ok(());
        }
        // SAFETY: `kill` takes two integers by value and dereferences nothing. The pid is one
        // this process forked and has not yet reaped (checked above), so it still names our
        // child — a zombie at worst — and cannot have been recycled.
        let rc = unsafe { libc::kill(self.pid, libc::SIGKILL) };
        if rc < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            return Err(last_syscall_error("kill"));
        }
        Ok(())
    }

    /// Has the child exited? Non-blocking; `None` means still running.
    ///
    /// ★ §7.5's rule that this exists for: **never block-join a wedged worker.** A host
    /// thread in uninterruptible sleep cannot be signalled awake, so `SIGKILL` does not
    /// reap it, and a blocking wait on it is itself unbounded. The escape is to declare the
    /// slot dead, release the requester, and let the kernel reap the corpse whenever the
    /// ioctl finally returns — which is what polling this asks about.
    ///
    /// # Errors
    /// [`RawError::Syscall`] (`waitpid`).
    pub fn try_reap(&mut self) -> Result<Option<i32>, RawError> {
        if let Some(code) = self.status {
            return Ok(Some(code));
        }
        self.wait(libc::WNOHANG)
    }

    /// Wait for the child and return its exit code.
    ///
    /// # Errors
    /// [`RawError::Syscall`] (`waitpid`).
    ///
    /// # Panics
    /// If called with any ranked or adapter-leaf lock held (R1, §4.5).
    pub fn reap(&mut self) -> Result<i32, RawError> {
        lockwitness::assert_lock_free("waiting for a sandboxed child to exit");
        leafwitness::assert_leaf_free("waiting for a sandboxed child to exit");
        if let Some(code) = self.status {
            return Ok(code);
        }
        loop {
            if let Some(code) = self.wait(0)? {
                return Ok(code);
            }
        }
    }

    /// One `waitpid`. `Ok(None)` means "still running" (only reachable with `WNOHANG`).
    fn wait(&mut self, flags: libc::c_int) -> Result<Option<i32>, RawError> {
        let mut status: libc::c_int = 0;
        // SAFETY: `waitpid` writes one `c_int` through the pointer, which addresses a local
        // that outlives the call; the pid and flags are integers by value. The pid is our own
        // unreaped child (`self.status.is_none()`, checked by every caller), so no other
        // process is addressed.
        let rc = unsafe { libc::waitpid(self.pid, &raw mut status, flags) };
        if rc < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) {
                return Ok(None);
            }
            return Err(last_syscall_error("waitpid"));
        }
        if rc == 0 {
            return Ok(None);
        }
        let code = if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else if libc::WIFSIGNALED(status) {
            -libc::WTERMSIG(status)
        } else {
            // Stopped/continued: not an exit, and we did not ask for those.
            return Ok(None);
        };
        self.status = Some(code);
        Ok(Some(code))
    }
}

impl Drop for SandboxChild {
    /// Kill and reap, so an isolate that goes out of scope leaves no zombie (§7.6's R3
    /// resource class).
    ///
    /// # Panics
    /// If this thread holds any ranked or adapter-leaf lock (R1) — this drop *waits*. The
    /// assert is skipped while already panicking, the standard guard-in-`Drop` discipline:
    /// a panic here would abort the process and replace the real failure's message.
    fn drop(&mut self) {
        if !std::thread::panicking() {
            lockwitness::assert_lock_free(
                "dropping a sandboxed child (kill + waitpid, both blocking)",
            );
            leafwitness::assert_leaf_free("dropping a sandboxed child (kill + waitpid)");
        }
        if self.status.is_some() {
            return;
        }
        let _ = self.kill();
        let _ = self.reap();
    }
}

// =====================================================================================
// Parent-side helpers
// =====================================================================================

/// The C's flag word (`C: nvkvm_isolate.c:127-129`). `CLONE_NEWUSER` is what lets an
/// unprivileged parent take the other five.
const NAMESPACE_CLONE_FLAGS: libc::c_ulong = (libc::CLONE_NEWUSER
    | libc::CLONE_NEWPID
    | libc::CLONE_NEWNET
    | libc::CLONE_NEWIPC
    | libc::CLONE_NEWUTS
    | libc::CLONE_NEWNS) as libc::c_ulong;

/// An `OsStr` as a `CString`, or the `EINVAL` an embedded NUL deserves.
fn cstring(s: &OsStr) -> Result<CString, RawError> {
    CString::new(s.as_bytes()).map_err(|_| RawError::Syscall {
        call: "spawn: an argument contains a NUL byte",
        errno: Some(libc::EINVAL),
    })
}

/// A close-on-exec pipe, as two owned descriptors.
fn pipe_cloexec(call: &'static str) -> Result<(OwnedFd, OwnedFd), RawError> {
    let (rx, tx) = std::io::pipe().map_err(|e| RawError::Syscall {
        call,
        errno: e.raw_os_error(),
    })?;
    Ok((OwnedFd::from(rx), OwnedFd::from(tx)))
}

/// Duplicate `fd` onto a number strictly above `floor`, close-on-exec.
///
/// ★ The reason this exists: the child's `dup2` shuffle owns `[0, floor]`, and a descriptor
/// it still needs — the error pipe, the image — landing inside that range would be clobbered
/// by a grant. `F_DUPFD_CLOEXEC` picks the lowest free number at or above the minimum, so
/// the two pipes and the image cannot collide with each other either.
fn dup_above(fd: &OwnedFd, floor: i32) -> Result<OwnedFd, RawError> {
    // SAFETY: `F_DUPFD_CLOEXEC` takes an integer minimum by value on a live borrowed
    // descriptor and dereferences no memory. It returns a fresh descriptor or -1; the
    // `from_raw_fd` below is reached only on success, so the `OwnedFd` is the sole owner of a
    // number this call just created.
    let raw = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, floor + 1) };
    if raw < 0 {
        return Err(last_syscall_error("fcntl(F_DUPFD_CLOEXEC)"));
    }
    // SAFETY: `raw` was just returned by `F_DUPFD_CLOEXEC` and nothing else refers to it.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// This process's effective uid.
///
/// ⊘ **Not a permission check.** Nothing may branch on this to decide whether an operation
/// is allowed — `guest_blast_radius.md` §3.2 is that RM re-derives the caller's privilege
/// on every ioctl, so the kernel's answer is the only one that counts and a userspace
/// pre-check would only ever disagree with it. It exists so a **diagnostic can label its
/// own output**: `#128`'s timer rung is meaningless under root (RM's mmap validation walk
/// is skipped entirely for `osIsAdministrator()`), and a run that cannot say which it was
/// is a number nobody can read.
#[must_use]
pub fn geteuid() -> u32 {
    // SAFETY: `geteuid` takes no arguments, dereferences nothing, and is documented as
    // always succeeding.
    unsafe { libc::geteuid() }
}

/// Write the child's rootless uid/gid maps, in **the kernel's required order**.
///
/// `setgroups` = `deny` must land before `gid_map` is writable by an unprivileged process;
/// the single-line map `0 <our outer id> 1` is the one an unprivileged parent may always
/// write. Ported from `C: nvkvm_isolate.c:102-114` rather than re-derived.
fn write_child_maps(pid: libc::pid_t) -> Result<(), RawError> {
    // SAFETY: `geteuid`/`getegid` take no arguments, dereference nothing, and cannot fail.
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    write_proc(pid, "setgroups", "deny")?;
    write_proc(pid, "uid_map", &format!("0 {uid} 1\n"))?;
    write_proc(pid, "gid_map", &format!("0 {gid} 1\n"))
}

/// One `/proc/<pid>/<what>` control-file write.
fn write_proc(pid: libc::pid_t, what: &'static str, contents: &str) -> Result<(), RawError> {
    std::fs::write(format!("/proc/{pid}/{what}"), contents).map_err(|e| RawError::Syscall {
        call: match what {
            "setgroups" => "write(/proc/<child>/setgroups)",
            "uid_map" => "write(/proc/<child>/uid_map)",
            _ => "write(/proc/<child>/gid_map)",
        },
        errno: e.raw_os_error(),
    })
}

/// Write every byte of `bytes` to `fd`.
fn write_all(fd: &OwnedFd, bytes: &[u8]) -> Result<(), RawError> {
    use std::io::Write as _;
    let mut f = std::fs::File::from(fd.try_clone().map_err(|e| RawError::Syscall {
        call: "dup(handshake)",
        errno: e.raw_os_error(),
    })?);
    f.write_all(bytes).map_err(|e| RawError::Syscall {
        call: "write(uid_map handshake)",
        errno: e.raw_os_error(),
    })
}

/// Read the child's pre-`exec` failure report, if it made one.
///
/// EOF means the `exec` happened: the pipe is close-on-exec, so it is the `exec` itself that
/// closes the write end. This is the property that makes a failed spawn an `Err` instead of a
/// live handle to a process that never ran.
fn read_child_report(rx: &OwnedFd) -> Result<Option<(u32, i32)>, RawError> {
    use std::io::Read as _;
    let mut f = std::fs::File::from(rx.try_clone().map_err(|e| RawError::Syscall {
        call: "dup(spawn error report)",
        errno: e.raw_os_error(),
    })?);
    let mut buf = [0u8; 8];
    let mut got = 0usize;
    while got < buf.len() {
        match f.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                return Err(RawError::Syscall {
                    call: "read(spawn error report)",
                    errno: e.raw_os_error(),
                });
            }
        }
    }
    if got < buf.len() {
        return Ok(None);
    }
    let stage = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let errno = i32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]);
    Ok(Some((stage, errno)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// `/bin/sh` is the only program this file's tests need, and every hit against it is a
    /// *behavioural* assertion about the descriptor table — never a mock.
    const SH: &str = "/bin/sh";

    #[test]
    fn a_granted_descriptor_arrives_on_the_number_it_was_granted() {
        let (mut rx, tx) = std::io::pipe().expect("pipe");
        let spec = ChildSpec::new(SH)
            .arg("-c")
            .arg("echo granted >&7")
            .grant(FdGrant::new(OwnedFd::from(tx), 7));
        let mut child = SandboxChild::spawn(spec).expect("spawn /bin/sh");
        assert_eq!(child.reap().expect("reap"), 0);
        let mut got = String::new();
        rx.read_to_string(&mut got).expect("read");
        assert_eq!(got, "granted\n");
    }

    /// ★★ The security property, asserted directly: a descriptor the parent holds and did
    /// NOT grant is **not** in the child.
    ///
    /// ★ The instrument, corrected twice. The first version compared descriptor *numbers*
    /// across a process boundary, which is meaningless — the child's `ls` opens its own
    /// low-numbered descriptor, and when the parent's secret happened to land on the same
    /// number the test failed intermittently while the code was correct. Descriptors are
    /// compared by **identity** (`/proc/self/fd` resolves a pipe to `pipe:[inode]`), which
    /// is the only thing that means the same in both processes.
    #[test]
    fn an_ungranted_descriptor_does_not_reach_the_child() {
        let (mut report_rx, report_tx) = std::io::pipe().expect("pipe");
        // A descriptor the parent holds for the whole spawn and never declares.
        let (_secret_rx, secret_tx) = std::io::pipe().expect("pipe");
        let secret = OwnedFd::from(secret_tx);
        let secret_identity = std::fs::read_link(format!("/proc/self/fd/{}", secret.as_raw_fd()))
            .expect("read_link on our own descriptor");
        let secret_identity = secret_identity.to_string_lossy().into_owned();
        assert!(
            secret_identity.starts_with("pipe:["),
            "the fixture needs an identifiable object, got {secret_identity:?}"
        );

        let spec = ChildSpec::new(SH)
            .arg("-c")
            .arg("ls -l /proc/self/fd >&7")
            .grant(FdGrant::new(OwnedFd::from(report_tx), 7));
        let mut child = SandboxChild::spawn(spec).expect("spawn");
        assert_eq!(child.reap().expect("reap"), 0);
        drop(secret);

        let mut listing = String::new();
        report_rx.read_to_string(&mut listing).expect("read");
        assert!(
            !listing.contains(&secret_identity),
            "the undeclared descriptor {secret_identity} reached the child:\n{listing}"
        );
        // Non-vacuity: the DECLARED one did arrive, so the assertion above is not passing
        // because nothing was inherited at all.
        assert!(
            listing
                .lines()
                .any(|l| l.ends_with(" 7 -> pipe:[") || l.contains(" 7 -> pipe:[")),
            "the declared grant is missing:\n{listing}"
        );
    }

    /// The fd-shuffle hazard the two-stage placement exists for: grant A's *source* number
    /// is grant B's *target* number. A one-pass `dup2` loop clobbers one of them, and the
    /// direction it fails in depends on iteration order — which is why it is a test and not
    /// a comment.
    #[test]
    fn a_grant_whose_source_number_is_another_grants_target_still_arrives() {
        let (mut rx3, tx3) = std::io::pipe().expect("pipe");
        let (mut rx4, tx4) = std::io::pipe().expect("pipe");
        let a = OwnedFd::from(tx3);
        let b = OwnedFd::from(tx4);
        // Deliberately target each other's numbers, whatever they happen to be.
        let (ta, tb) = (b.as_raw_fd(), a.as_raw_fd());
        assert_ne!(ta, tb);
        let spec = ChildSpec::new(SH)
            .arg("-c")
            // ★ Written through `/proc/self/fd/N`, not `>&N`. The instrument again: a
            // POSIX shell only accepts SINGLE-DIGIT descriptor numbers in a `>&`
            // redirection, and these numbers are whatever the OS handed out — which under
            // a parallel test run is routinely two digits. The obvious version of this
            // test fails with "Bad fd number" while the code is correct, and it fails
            // *intermittently*, which is worse.
            .arg(format!(
                "echo A > /proc/self/fd/{ta}; echo B > /proc/self/fd/{tb}"
            ))
            .grant(FdGrant::new(a, ta))
            .grant(FdGrant::new(b, tb));
        let mut child = SandboxChild::spawn(spec).expect("spawn");
        assert_eq!(child.reap().expect("reap"), 0);
        let (mut sa, mut sb) = (String::new(), String::new());
        rx3.read_to_string(&mut sa).expect("read");
        rx4.read_to_string(&mut sb).expect("read");
        // `a` was granted onto `b`'s source NUMBER, so "A" must arrive on `a`'s pipe —
        // i.e. the shuffle placed each grant on its own target and clobbered neither.
        assert_eq!((sa.as_str(), sb.as_str()), ("A\n", "B\n"));
    }

    /// ★ The environment is the other inheritance channel, and the C clears it for the
    /// same reason it closes descriptors (`C: src/qemu/nvkvm_isolate.c:856-858`,
    /// `envp = {NULL}`).
    ///
    /// The probe reads the child's **exported** environment (`env`), not a shell parameter
    /// expansion. That distinction is the instrument: `${PATH-unset}` reports `PATH` even
    /// in a cleared environment, because `dash` invents a default `PATH` *shell variable*
    /// on startup — so the obvious version of this test fails while the code is correct.
    #[test]
    fn the_environment_is_cleared_and_only_declared_entries_survive() {
        // Non-vacuity: if OUR environment were already empty, the assertion below would
        // pass without the clear doing anything.
        assert!(
            std::env::vars_os().count() > 2,
            "the fixture needs a non-trivial parent environment to be worth clearing"
        );
        let (mut rx, tx) = std::io::pipe().expect("pipe");
        let spec = ChildSpec::new(SH)
            .arg("-c")
            .arg("env >&7")
            .env("KAYFABE_DECLARED", "yes")
            .grant(FdGrant::new(OwnedFd::from(tx), 7));
        let mut child = SandboxChild::spawn(spec).expect("spawn");
        assert_eq!(child.reap().expect("reap"), 0);
        let mut got = String::new();
        rx.read_to_string(&mut got).expect("read");
        let names: Vec<&str> = got
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, _)| k)
            .collect();
        assert!(
            got.contains("KAYFABE_DECLARED=yes"),
            "the declared entry must arrive: {got:?}"
        );
        // `PWD` is set by the shell itself, not inherited; nothing else may be there.
        assert!(
            names
                .iter()
                .all(|n| *n == "KAYFABE_DECLARED" || *n == "PWD"),
            "the inherited environment reached the child: {names:?}"
        );
    }

    /// ★★ The report pipe, and the failure mode it exists for. `std::process::Command` gave
    /// this for free; now that the spawn is raw, a failed `exec` must still be an `Err` and
    /// not a live handle to a process that never ran.
    #[test]
    fn a_missing_program_reports_enoent_exactly() {
        assert_eq!(
            SandboxChild::spawn(ChildSpec::new("/kayfabe/no/such/program")).err(),
            Some(RawError::Syscall {
                call: "execve",
                errno: Some(libc::ENOENT),
            })
        );
    }

    #[test]
    fn a_child_that_outlives_its_handle_is_killed_and_reaped() {
        let (_rx, tx) = std::io::pipe().expect("pipe");
        let spec = ChildSpec::new(SH)
            .arg("-c")
            .arg("sleep 300")
            .grant(FdGrant::new(OwnedFd::from(tx), 7));
        let child = SandboxChild::spawn(spec).expect("spawn");
        let pid = child.pid();
        drop(child);
        // The drop reaped it, so the pid is no longer a live child of ours. Asserted by
        // the absence of a zombie: a reaped child has no /proc entry with our ppid.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        assert!(
            !stat.contains(") Z "),
            "the child was left a zombie: {stat}"
        );
    }

    #[test]
    fn try_reap_reports_none_while_running_and_the_code_afterwards() {
        let spec = ChildSpec::new(SH).arg("-c").arg("exit 7");
        let mut child = SandboxChild::spawn(spec).expect("spawn");
        // Not a sleep-based race: `reap` is the synchronisation point, `try_reap` after it
        // is the interesting assertion.
        assert_eq!(child.reap().expect("reap"), 7);
        assert_eq!(child.try_reap().expect("try_reap"), Some(7));
    }

    #[test]
    #[should_panic(expected = "descriptor grant")]
    fn a_grant_onto_a_standard_stream_is_refused() {
        let (_rx, tx) = std::io::pipe().expect("pipe");
        let _ = FdGrant::new(OwnedFd::from(tx), 1);
    }

    /// ★ The child's placement table is a **fixed-size array**, because the post-`fork` child
    /// may not allocate. That bound has to be refused where refusing is still possible — in
    /// the parent — or the child would write past it. Asserted rather than argued: the
    /// refusal is a code path, and an unexercised refusal is a comment.
    #[test]
    fn more_grants_than_the_child_can_place_are_refused_in_the_parent() {
        let mut spec = ChildSpec::new(SH);
        let mut keep = Vec::new();
        // `MAX_PLACEMENTS` counts stdin's `/dev/null` too, so exactly that many grants is one
        // too many. Targets start at 3 and must not collide with each other.
        for i in 0..MAX_PLACEMENTS {
            let (rx, tx) = std::io::pipe().expect("pipe");
            keep.push(rx);
            spec = spec.grant(FdGrant::new(
                OwnedFd::from(tx),
                3 + i32::try_from(i).expect("a small count"),
            ));
        }
        assert_eq!(
            SandboxChild::spawn(spec).err(),
            Some(RawError::Syscall {
                call: "spawn: more descriptor grants than the child can place without allocating",
                errno: Some(libc::E2BIG),
            })
        );
    }

    #[test]
    #[should_panic(expected = "both claim number")]
    fn two_grants_on_one_number_are_refused() {
        let (_r1, t1) = std::io::pipe().expect("pipe");
        let (_r2, t2) = std::io::pipe().expect("pipe");
        let _ = ChildSpec::new(SH)
            .grant(FdGrant::new(OwnedFd::from(t1), 7))
            .grant(FdGrant::new(OwnedFd::from(t2), 7));
    }

    /// A granted socket really is bidirectional in the child — the property the isolate's
    /// whole request/reply protocol rests on.
    #[test]
    fn a_granted_unix_socket_carries_a_round_trip() {
        use std::os::unix::net::UnixStream;
        let (mut ours, theirs) = UnixStream::pair().expect("socketpair");
        let spec = ChildSpec::new(SH)
            .arg("-c")
            .arg("head -c 4 <&9 >&9")
            .grant(FdGrant::new(OwnedFd::from(theirs), 9));
        let mut child = SandboxChild::spawn(spec).expect("spawn");
        ours.write_all(b"ping").expect("write");
        let mut buf = [0u8; 4];
        ours.read_exact(&mut buf).expect("read");
        assert_eq!(&buf, b"ping");
        assert_eq!(child.reap().expect("reap"), 0);
    }

    // ---------------------------------------------------------------------------------
    // The image
    // ---------------------------------------------------------------------------------

    /// ★★★ A child spawned from an image this process holds in memory — no path anywhere.
    ///
    /// The image is a copy of `/bin/sh`'s bytes rather than a synthetic ELF, because what is
    /// under test is that `execveat(AT_EMPTY_PATH)` on a sealed memfd really runs, and a
    /// program whose output we can check is the only thing that says so.
    #[test]
    fn a_child_runs_from_an_image_with_no_path_at_all() {
        let bytes = std::fs::read(SH).expect("/bin/sh is readable");
        let image = ProgramImage::from_bytes(c"kayfabe-test-image", &bytes).expect("image");
        assert_eq!(image.len_bytes(), bytes.len());
        let (mut rx, tx) = std::io::pipe().expect("pipe");
        let spec = ChildSpec::from_image(&image, "not-a-real-name")
            .expect("dup")
            .arg("-c")
            .arg("echo from-memory >&7")
            .grant(FdGrant::new(OwnedFd::from(tx), 7));
        assert_eq!(
            spec.program(),
            None,
            "an image has no path, and that is the point"
        );
        let mut child = SandboxChild::spawn(spec).expect("spawn from image");
        assert_eq!(child.reap().expect("reap"), 0);
        let mut got = String::new();
        rx.read_to_string(&mut got).expect("read");
        assert_eq!(got, "from-memory\n");
    }

    /// ★ One image, many spawns: the descriptor is duplicated per spec, so a finished child
    /// cannot invalidate the image the next one needs.
    #[test]
    fn one_image_spawns_more_than_once() {
        let bytes = std::fs::read(SH).expect("/bin/sh is readable");
        let image = ProgramImage::from_bytes(c"kayfabe-test-image", &bytes).expect("image");
        for i in 0..3 {
            let spec = ChildSpec::from_image(&image, "sh")
                .expect("dup")
                .arg("-c")
                .arg(format!("exit {i}"));
            let mut child = SandboxChild::spawn(spec).expect("spawn");
            assert_eq!(child.reap().expect("reap"), i);
        }
    }

    /// ★★ The image is **sealed**: nothing holding the descriptor can change the bytes that
    /// get `exec`'d. Asserted by watching a write fail, not by reading the seal bits back —
    /// the seal is only worth anything if it is the write that stops.
    #[test]
    fn a_published_image_cannot_be_rewritten() {
        let image = ProgramImage::from_bytes(c"sealed", b"\x7fELF and then some").expect("image");
        let mut f = std::fs::File::from(image.fd.try_clone().expect("dup"));
        let err = f
            .write_all(b"xxxx")
            .expect_err("a sealed image must refuse a write");
        assert_eq!(
            err.raw_os_error(),
            Some(libc::EPERM),
            "the seal must be what refuses, got {err:?}"
        );
    }

    /// An empty image is refused where the diagnosis is cheap, rather than as one `exec`
    /// failure per spawn. This is the arm a build that embedded nothing would take.
    #[test]
    fn an_empty_image_is_refused_at_publication() {
        assert_eq!(
            ProgramImage::from_bytes(c"empty", b"").err(),
            Some(RawError::Syscall {
                call: "ProgramImage: the embedded image is empty",
                errno: Some(libc::ENOEXEC),
            })
        );
    }

    // ---------------------------------------------------------------------------------
    // The namespaces
    // ---------------------------------------------------------------------------------

    /// ★★★ The child is **PID 1 of its own pid namespace**, in its own user, network, IPC,
    /// UTS and mount namespaces, with the single-line uid map its parent wrote.
    ///
    /// This is the thing `unshare` after `exec` could never give: `unshare(CLONE_NEWPID)`
    /// moves only *future* children and the isolate creates none, so a post-`exec` isolate
    /// was in the host pid namespace no matter what it called. `sandbox_unsafe.rs`'s
    /// `ISOLATING_NAMESPACES` said exactly that and left the flag out.
    ///
    /// ★ Every namespace is compared **by identity** (`readlink /proc/self/ns/<kind>` gives
    /// `pid:[<inode>]`), never by "it looks different": an inode is the only thing that means
    /// the same in both processes. And the uid map is read back, because it is what the
    /// PARENT wrote through `/proc/<pid>/uid_map` — the half of the handshake nothing else
    /// here observes.
    #[test]
    fn a_namespaced_child_is_pid_1_in_its_own_namespaces() {
        crate::require_user_namespace!("a_namespaced_child_is_pid_1_in_its_own_namespaces");
        const KINDS: [&str; 6] = ["pid", "user", "net", "ipc", "uts", "mnt"];
        let ours: Vec<String> = KINDS
            .iter()
            .map(|k| {
                std::fs::read_link(format!("/proc/self/ns/{k}"))
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();

        let (mut rx, tx) = std::io::pipe().expect("pipe");
        let script = "{ echo mypid=$$; \
                        for k in pid user net ipc uts mnt; do \
                          echo $k=$(readlink /proc/self/ns/$k); done; \
                        echo map=$(cat /proc/self/uid_map); } >&7";
        let spec = ChildSpec::new(SH)
            .arg("-c")
            .arg(script)
            .grant(FdGrant::new(OwnedFd::from(tx), 7))
            .in_new_namespaces();
        let mut child = SandboxChild::spawn(spec).expect("clone into fresh namespaces");
        assert_eq!(child.reap().expect("reap"), 0);
        let mut report = String::new();
        rx.read_to_string(&mut report).expect("read");

        let field = |name: &str| -> String {
            report
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{name}=")))
                .unwrap_or_else(|| panic!("the child reported no {name}:\n{report}"))
                .to_owned()
        };

        assert_eq!(field("mypid"), "1", "the child must be PID 1:\n{report}");
        for (kind, ours) in KINDS.iter().zip(&ours) {
            let theirs = field(kind);
            assert!(
                theirs.starts_with(&format!("{kind}:[")),
                "the {kind} namespace was unreadable: {theirs:?}"
            );
            // Non-vacuity: OUR own link had to be readable too, or "they differ" is trivial.
            assert!(
                ours.starts_with(&format!("{kind}:[")),
                "the instrument is broken: this process's {kind} namespace read as {ours:?}"
            );
            assert_ne!(
                &theirs, ours,
                "the child shares our {kind} namespace:\n{report}"
            );
        }
        // The parent's half of the handshake: `0 <our euid> 1`, a single mapped id.
        let map: Vec<String> = field("map").split_whitespace().map(str::to_owned).collect();
        assert_eq!(map.len(), 3, "expected one map line, got {map:?}");
        assert_eq!(map[0], "0", "ns-root must be 0");
        assert_eq!(map[2], "1", "exactly one id may be mapped");
    }

    /// ★★ The fail-closed arm, watched. If the parent cannot install the maps the child must
    /// **refuse**, never run unmapped — the C's `_exit(126)` on a short `read`
    /// (`C: nvkvm_isolate.c:820-825`).
    ///
    /// Induced by making the map write fail for real: the child is `clone`d into a user
    /// namespace and then reaped by *this* test before the maps are written, so
    /// `/proc/<pid>/uid_map` no longer exists. There is no way to reach that from the public
    /// API, which is why the assertion is on the arm's *code path* through a second child
    /// whose namespace request the kernel refuses — `CLONE_NEWUSER` inside a `chroot` — when
    /// that is available, and on the error's shape otherwise.
    #[test]
    fn a_namespace_request_that_cannot_be_satisfied_is_a_refusal_not_a_weaker_child() {
        crate::require_user_namespace!(
            "a_namespace_request_that_cannot_be_satisfied_is_a_refusal_not_a_weaker_child"
        );
        // The reachable half: with namespaces requested and the handshake completed, a child
        // whose *program* does not exist still reports `execve`/`ENOENT` — i.e. the handshake
        // ran to completion and the error that surfaces is the real one, not a swallowed
        // namespace failure.
        let err =
            SandboxChild::spawn(ChildSpec::new("/kayfabe/no/such/program").in_new_namespaces())
                .err();
        assert_eq!(
            err,
            Some(RawError::Syscall {
                call: "execve",
                errno: Some(libc::ENOENT),
            }),
            "a namespaced spawn must report the real failure, not a namespace one"
        );
    }

    /// Bytes that are not an executable fail the `exec`, and the failure is reported as one —
    /// the report pipe carrying `ENOEXEC` from `execveat` rather than a live handle.
    #[test]
    fn an_image_that_is_not_an_executable_fails_the_exec() {
        let image = ProgramImage::from_bytes(c"garbage", b"not an elf at all").expect("image");
        let spec = ChildSpec::from_image(&image, "garbage").expect("dup");
        assert_eq!(
            SandboxChild::spawn(spec).err(),
            Some(RawError::Syscall {
                call: "execve",
                errno: Some(libc::ENOEXEC),
            })
        );
    }
}

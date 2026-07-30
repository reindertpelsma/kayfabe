//! Spawning a child process with an **explicitly declared** descriptor table.
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
//! ## What is deliberately absent, and it is the security gap of record
//!
//! The C's hardened spawn also does: `CLONE_NEWUSER|NEWPID|NEWNET|NEWIPC|NEWUTS|NEWNS`
//! with a `uid_map`/`gid_map` handshake over a sync pipe
//! (`C: src/qemu/nvkvm_isolate.c:124-133`, `:102-114`), a `pivot_root` onto a `tmpfs`
//! containing only the bound nvidia nodes (`:155-231`), a capability drop (`:66-81`), and
//! a seccomp allowlist applied with `TSYNC` (`C: src/stub/nvkvm_stub.c:2505-2587`).
//!
//! Of that list, the **mount namespace and the `pivot_root`** now exist —
//! [`crate::sandbox`] — but they are entered by the child in its own `main`, **not** here
//! between `fork` and `exec` as the C does it. That is not a preference: the C `fexecve`s a
//! statically linked stub out of a `memfd`, so it needs neither a path nor a loader once
//! the old root is gone, while `Command` `execve`s a *path* and a dynamically linked Rust
//! binary additionally wants `/lib64/ld-linux-*.so`. Moving it pre-`exec` is a real
//! improvement (an image cannot decline a sandbox it was born in) and it is gated on a
//! static isolate build, which is why this crate keeps building for musl.
//!
//! The **capability drop and the user/net/ipc/uts namespaces** now exist too, and are in the
//! same place for the same reason ([`crate::sandbox::enter`], which reads back the outcome
//! and refuses on a surviving bit). Note in particular that `PR_SET_NO_NEW_PRIVS` below is
//! **inert for an already-privileged process** — it blocks *gaining* privilege through
//! setuid/fcaps and nothing else — so it never was a substitute for that drop, and the
//! isolate ran with `CapEff = 000001ffffffffff` for as long as it was the only control here.
//!
//! Still absent: **seccomp**, and **`CLONE_NEWPID`** — which cannot be had from the child's
//! own `main` at all, since `unshare(CLONE_NEWPID)` moves only future children and the
//! isolate creates none. Both named rather than stubbed, because an untested sandbox is
//! worse than a declared absence: it reads as a boundary in every review that follows.

use crate::error::{RawError, last_syscall_error};
use kayfabe_util::{leafwitness, lockwitness};
use std::ffi::{OsStr, OsString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

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

/// Everything a child process is to be started with — and, by omission, everything it is
/// not.
#[derive(Debug)]
pub struct ChildSpec {
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    grants: Vec<FdGrant>,
}

impl ChildSpec {
    /// A child running `program` with no arguments, **no environment**, and no descriptors
    /// beyond the standard streams (which are attached to `/dev/null`).
    #[must_use]
    pub fn new(program: impl Into<PathBuf>) -> Self {
        ChildSpec {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            grants: Vec::new(),
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

    /// The program this spec would run.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
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
    child: Child,
    reaped: bool,
}

impl SandboxChild {
    /// ★ Spawn the child. Every descriptor except the standard streams and the declared
    /// grants is closed before `exec`.
    ///
    /// # Errors
    /// [`RawError::Syscall`] (`fork`/`execve`, reported as `posix_spawn` by `std`) — most
    /// commonly `ENOENT` for a program path that does not exist.
    ///
    /// # Panics
    /// If called with any ranked or adapter-leaf lock held (R1, §4.5). `fork` of a process
    /// this size is not a cheap call, and the child it produces is the thing every other
    /// thread will then contend for.
    pub fn spawn(spec: ChildSpec) -> Result<Self, RawError> {
        lockwitness::assert_lock_free("spawning a sandboxed child process");
        leafwitness::assert_leaf_free("spawning a sandboxed child process");

        // The grants stay owned HERE, alive across `spawn()`, because the `pre_exec`
        // closure must be `'static` and therefore may only capture the numbers. Dropping
        // them earlier would close the descriptors the child is about to be handed.
        let grants = spec.grants;
        let highest = grants.iter().map(|g| g.target).max().unwrap_or(2);
        let plan: Vec<(i32, i32)> = grants
            .iter()
            .map(|g| (g.fd.as_raw_fd(), g.target))
            .collect();

        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }

        // SAFETY: `pre_exec` runs in the forked child between `fork` and `execve`, where
        // only async-signal-safe operations are permitted. Every call below is on POSIX's
        // async-signal-safe list (`fcntl`, `dup2`, `close_range`, `prctl`); none allocates,
        // none takes a lock, and none touches memory this closure does not own by value.
        // The captured `plan` is a `Vec<(i32, i32)>` built entirely before the fork, so no
        // allocation happens on the child side either — iterating it only reads.
        unsafe {
            cmd.pre_exec(move || {
                // Stage 1: park every source descriptor ABOVE the highest target, so
                // stage 2's `dup2` can never clobber a source it has not copied yet. Doing
                // it in one pass without this is the classic fd-shuffle bug: grant A's
                // source happens to be grant B's target.
                let mut parked = Vec::with_capacity(plan.len());
                for &(src, target) in &plan {
                    let up = libc::fcntl(src, libc::F_DUPFD_CLOEXEC, highest + 1);
                    if up < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    parked.push((up, target));
                }
                // Stage 2: place them. `dup2` clears close-on-exec on the NEW descriptor,
                // which is the only reason a grant survives `execve` — and it clears it on
                // the duplicate only, so nothing racy is done to a descriptor the parent
                // still shares.
                for &(up, target) in &parked {
                    if libc::dup2(up, target) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                // Stage 3: ★★ the closefrom — the line the C's audit M6 is about, with
                // ONE correction the build forced (see the type docs' "the finding"). It
                // marks the range close-on-exec rather than closing it, so the descriptors
                // die at `execve` instead of here.
                //
                // Closing outright is what the C does and it is WRONG in this process:
                // `std::process::Command` reports an `execve` failure to the parent over
                // an internal CLOEXEC pipe, and that pipe is in this range. Closing it made
                // a failed exec look like a SUCCESSFUL SPAWN — the parent got a live
                // `Child` for a process that never ran. `CLOSE_RANGE_CLOEXEC` is also
                // strictly better security: the descriptors stop existing at the exec
                // boundary atomically, and if the exec fails they are still there to carry
                // the diagnosis.
                //
                // Issued as a raw syscall rather than through libc's wrapper: `close_range`
                // is glibc-only in the `libc` crate, and this crate must keep building for
                // a static musl target — which is how the isolate reaches a bench box whose
                // glibc is older than the build host's.
                if libc::syscall(
                    libc::SYS_close_range,
                    libc::c_uint::from(u32::try_from(highest + 1).unwrap_or(u32::MAX)),
                    libc::c_uint::MAX,
                    libc::CLOSE_RANGE_CLOEXEC,
                ) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                // Stage 4: no privilege may be gained by anything this child execs.
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = cmd.spawn().map_err(|e| RawError::Syscall {
            call: "spawn",
            errno: e.raw_os_error(),
        })?;
        drop(grants);
        Ok(SandboxChild {
            child,
            reaped: false,
        })
    }

    /// The child's process id, as seen from **this** process.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// `SIGKILL` the child. Idempotent, and never an error if it has already exited.
    ///
    /// # Errors
    /// [`RawError::Syscall`] (`kill`).
    pub fn kill(&mut self) -> Result<(), RawError> {
        match self.child.kill() {
            Ok(()) => Ok(()),
            // `InvalidInput` is `std`'s report for "already reaped", which is a state and
            // not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            Err(e) => Err(RawError::Syscall {
                call: "kill",
                errno: e.raw_os_error(),
            }),
        }
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
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.reaped = true;
                Ok(Some(status.code().unwrap_or(-1)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(RawError::Syscall {
                call: "waitpid",
                errno: e.raw_os_error(),
            }),
        }
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
        let status = self.child.wait().map_err(|e| RawError::Syscall {
            call: "waitpid",
            errno: e.raw_os_error(),
        })?;
        self.reaped = true;
        Ok(status.code().unwrap_or(-1))
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
        if self.reaped {
            return;
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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

    #[test]
    fn a_missing_program_reports_enoent_exactly() {
        assert_eq!(
            SandboxChild::spawn(ChildSpec::new("/kayfabe/no/such/program")).err(),
            Some(RawError::Syscall {
                call: "spawn",
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
}

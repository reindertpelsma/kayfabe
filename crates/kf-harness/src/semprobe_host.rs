//! The host side of `kf-semmarker-probe` that does not touch the GPU: the command line, the run guard and the
//! watchdog. Tested GPU-free (the guard reads `/proc` and `/sys` under a root it is given, so a test builds a
//! fake host tree).
//!
//! Variant A of the probe provokes a deliberate host `Xid 32` on its own channel. The guard keeps that off a
//! machine where the Xid could cost someone something: it refuses unless the operator opted in on the command
//! line, and then refuses again when a host fact says the GPU is not otherwise idle:
//! - a display connector of the GPU is connected or enabled;
//! - another process holds the GPU's device node (a CUDA or graphics context, a compositor);
//! - a `qemu` or `kf3` process is alive.
//!
//! Every check fails closed: a `/proc/<pid>/fd` directory that cannot be read counts as a holder, and a GPU
//! that cannot be found in the driver's procfs is a refusal. The facts come from the kernel, never from a
//! tool's text output.

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{Duration, Instant};

/// The flag that says the operator knows the probe raises a host `Xid 32` on purpose and that this
/// machine is a dedicated test box.
pub const OPT_IN_FLAG: &str = "--i-am-on-a-spec-box";

/// `comm` names of processes that hold the GPU's device node without a context and are therefore allowed:
/// `nvidia-persistenced` (the kernel truncates `comm` to 15 bytes).
pub const IDLE_HOLDERS: [&str; 1] = ["nvidia-persiste"];

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cli {
    /// The parent: `attempts` per variant.
    Parent {
        /// Attempts per variant, 1..=10.
        attempts: u32,
        /// [`OPT_IN_FLAG`] was given.
        opt_in: bool,
    },
    /// One attempt of a variant, in its own process (internal).
    Child {
        /// The variant letter, `A`, `B` or `C`.
        variant: String,
        /// [`OPT_IN_FLAG`] was given.
        opt_in: bool,
    },
}

/// Parse the arguments after the program name.
///
/// # Errors
/// A usage message for an unknown argument, a bad attempt count or a missing variant.
pub fn parse_cli(args: &[String]) -> Result<Cli, String> {
    const USAGE: &str = "usage: kf-semmarker-probe --i-am-on-a-spec-box [ATTEMPTS 1..=10] (child A|B|C is internal)";
    let opt_in = args.iter().any(|a| a == OPT_IN_FLAG);
    let rest: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != OPT_IN_FLAG)
        .collect();
    match rest.as_slice() {
        ["child", v] => Ok(Cli::Child {
            variant: (*v).to_owned(),
            opt_in,
        }),
        [] => Ok(Cli::Parent {
            attempts: 3,
            opt_in,
        }),
        [n] => match n.parse::<u32>() {
            Ok(n) if (1..=10).contains(&n) => Ok(Cli::Parent {
                attempts: n,
                opt_in,
            }),
            _ => Err(USAGE.to_owned()),
        },
        _ => Err(USAGE.to_owned()),
    }
}

/// One process seen in `/proc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    /// The process id.
    pub pid: u32,
    /// Its `comm`.
    pub comm: String,
}

/// What the guard learned from the host.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HostFacts {
    /// The PCI address of the gated GPU, when the driver's procfs lists it.
    pub bdf: Option<String>,
    /// Display connectors of that GPU that are connected or enabled.
    pub display: Vec<String>,
    /// Processes other than this one that hold the GPU's device node open.
    pub holders: Vec<Proc>,
    /// `qemu` or `kf3` processes.
    pub vms: Vec<Proc>,
    /// Facts that could not be read; each one is a refusal.
    pub unreadable: Vec<String>,
}

/// Why the guard refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// [`OPT_IN_FLAG`] was not given.
    NoOptIn,
    /// The driver's procfs does not list the gated GPU.
    GpuNotFound(u32),
    /// A display connector of the GPU is connected or enabled.
    Display(String),
    /// Another process holds the GPU's device node.
    Holder(Proc),
    /// A `qemu` or `kf3` process is alive.
    Vm(Proc),
    /// A host fact could not be read, so the guard cannot rule the case out.
    Unreadable(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOptIn => write!(
                f,
                "variant A raises a host Xid 32 on purpose: pass {OPT_IN_FLAG} on a dedicated test box"
            ),
            Self::GpuNotFound(m) => write!(f, "GPU minor {m} is not listed by the driver"),
            Self::Display(c) => write!(
                f,
                "display connector {c} of the GPU is connected or enabled"
            ),
            Self::Holder(p) => write!(
                f,
                "pid {} ({}) holds the GPU's device node: another context is live",
                p.pid, p.comm
            ),
            Self::Vm(p) => write!(f, "pid {} ({}) is a qemu/kf3 process", p.pid, p.comm),
            Self::Unreadable(w) => write!(f, "cannot rule the case out, unreadable: {w}"),
        }
    }
}

/// Pure: the guard's decision. Every refusal is listed.
///
/// # Errors
/// The refusals, when there is at least one.
pub fn decide(opt_in: bool, minor: u32, facts: &HostFacts) -> Result<(), Vec<Refusal>> {
    let mut out = Vec::new();
    if !opt_in {
        out.push(Refusal::NoOptIn);
    }
    if facts.bdf.is_none() {
        out.push(Refusal::GpuNotFound(minor));
    }
    out.extend(facts.display.iter().cloned().map(Refusal::Display));
    out.extend(facts.holders.iter().cloned().map(Refusal::Holder));
    out.extend(facts.vms.iter().cloned().map(Refusal::Vm));
    out.extend(facts.unreadable.iter().cloned().map(Refusal::Unreadable));
    if out.is_empty() { Ok(()) } else { Err(out) }
}

/// Where the guard reads the host from. Production: `/proc` and `/sys`.
#[derive(Debug, Clone)]
pub struct Roots {
    /// The procfs root.
    pub proc: PathBuf,
    /// The sysfs root.
    pub sys: PathBuf,
}

impl Roots {
    /// The real host.
    #[must_use]
    pub fn host() -> Self {
        Self {
            proc: PathBuf::from("/proc"),
            sys: PathBuf::from("/sys"),
        }
    }
}

/// Pure: whether a process name is a VM monitor or a kf3 process.
#[must_use]
pub fn is_vm_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("qemu") || n.contains("kf3")
}

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_owned())
}

/// The PCI address (lower case) of the GPU whose `Device Minor` is `minor`, from the driver's procfs.
fn bdf_of_minor(roots: &Roots, minor: u32) -> Option<String> {
    let dir = roots.proc.join("driver/nvidia/gpus");
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let info = std::fs::read_to_string(e.path().join("information")).unwrap_or_default();
        let m = info
            .lines()
            .find_map(|l| l.strip_prefix("Device Minor:"))
            .and_then(|v| v.trim().parse::<u32>().ok());
        if m == Some(minor) {
            return Some(e.file_name().to_string_lossy().to_lowercase());
        }
    }
    None
}

/// Connectors of the GPU at `bdf` (`<sys>/bus/pci/devices/<bdf>/drm/card*/card*-*`) whose `status` reads
/// `connected` or whose `enabled` reads `enabled`.
fn active_connectors(roots: &Roots, bdf: &str, unreadable: &mut Vec<String>) -> Vec<String> {
    let drm = roots.sys.join("bus/pci/devices").join(bdf).join("drm");
    let cards = match std::fs::read_dir(&drm) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            unreadable.push(format!("{}: {e}", drm.display()));
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for card in cards.flatten() {
        let Ok(conns) = std::fs::read_dir(card.path()) else {
            continue;
        };
        for c in conns.flatten() {
            let name = c.file_name().to_string_lossy().into_owned();
            if !name.starts_with("card") || !name.contains('-') {
                continue;
            }
            let status = read_trim(&c.path().join("status"));
            let enabled = read_trim(&c.path().join("enabled"));
            if status.as_deref() == Some("connected") || enabled.as_deref() == Some("enabled") {
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

/// Read the host facts for the GPU with `minor`, ignoring `self_pid`.
#[must_use]
pub fn collect(roots: &Roots, minor: u32, self_pid: u32) -> HostFacts {
    let mut facts = HostFacts {
        bdf: bdf_of_minor(roots, minor),
        ..HostFacts::default()
    };
    if let Some(bdf) = facts.bdf.clone() {
        let mut unreadable = Vec::new();
        facts.display = active_connectors(roots, &bdf, &mut unreadable);
        facts.unreadable = unreadable;
    }
    let node = format!("/dev/nvidia{minor}");
    let entries = match std::fs::read_dir(&roots.proc) {
        Ok(e) => e,
        Err(e) => {
            facts
                .unreadable
                .push(format!("{}: {e}", roots.proc.display()));
            return facts;
        }
    };
    for e in entries.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if pid == self_pid {
            continue;
        }
        let dir = e.path();
        let Some(comm) = read_trim(&dir.join("comm")) else {
            continue; // the process ended between listing and reading
        };
        let argv0 = std::fs::read(dir.join("cmdline"))
            .ok()
            .and_then(|b| b.split(|&c| c == 0).next().map(<[u8]>::to_vec))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .and_then(|a| a.rsplit('/').next().map(str::to_owned))
            .unwrap_or_default();
        let p = Proc { pid, comm };
        if is_vm_name(&p.comm) || is_vm_name(&argv0) {
            facts.vms.push(p.clone());
        }
        if IDLE_HOLDERS.contains(&p.comm.as_str()) {
            continue;
        }
        match std::fs::read_dir(dir.join("fd")) {
            Ok(fds) => {
                let holds = fds
                    .flatten()
                    .any(|f| std::fs::read_link(f.path()).is_ok_and(|t| t == Path::new(&node)));
                if holds {
                    facts.holders.push(p);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => facts
                .unreadable
                .push(format!("pid {pid} ({}) fd: {e}", p.comm)),
        }
    }
    facts.vms.sort_by_key(|p| p.pid);
    facts.holders.sort_by_key(|p| p.pid);
    facts
}

/// Collect the facts and decide, in one call.
///
/// # Errors
/// The refusals.
pub fn guard(roots: &Roots, opt_in: bool, minor: u32, self_pid: u32) -> Result<(), Vec<Refusal>> {
    decide(opt_in, minor, &collect(roots, minor, self_pid))
}

/// What the watchdog needs from a child process.
pub trait Supervised {
    /// Poll for the exit status.
    ///
    /// # Errors
    /// The OS error.
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    /// Kill the child.
    ///
    /// # Errors
    /// The OS error.
    fn kill(&mut self) -> io::Result<()>;
}

impl Supervised for std::process::Child {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Self::try_wait(self)
    }
    fn kill(&mut self) -> io::Result<()> {
        Self::kill(self)
    }
}

/// How a supervised child ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// The child ended by itself within the watchdog.
    pub exited: bool,
    /// The watchdog killed it.
    pub killed: bool,
    /// Its exit status, when it was reaped.
    pub status: Option<ExitStatus>,
}

/// Wait up to `watchdog` for the child; then kill it once and wait up to `reap` for it to be reaped.
///
/// # Errors
/// A failed poll of the child.
pub fn supervise<C: Supervised>(
    child: &mut C,
    watchdog: Duration,
    reap: Duration,
    poll: Duration,
) -> Result<Outcome, String> {
    let started = Instant::now();
    while started.elapsed() < watchdog {
        if let Some(s) = child.try_wait().map_err(|e| format!("wait: {e}"))? {
            return Ok(Outcome {
                exited: true,
                killed: false,
                status: Some(s),
            });
        }
        std::thread::sleep(poll);
    }
    let _ = child.kill();
    let killed_at = Instant::now();
    let mut status = None;
    while killed_at.elapsed() < reap {
        if let Ok(Some(s)) = child.try_wait() {
            status = Some(s);
            break;
        }
        std::thread::sleep(poll);
    }
    Ok(Outcome {
        exited: false,
        killed: true,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::os::unix::process::ExitStatusExt;
    use std::process::Command;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    /// A fake host: `/proc` and `/sys` of a machine with one GPU (minor 0, `0000:01:00.0`).
    struct Host {
        root: PathBuf,
    }

    impl Host {
        fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let root = std::env::temp_dir().join(format!(
                "kf-semprobe-host-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            let h = Self { root };
            h.write(
                "proc/driver/nvidia/gpus/0000:01:00.0/information",
                "Model: x\nDevice Minor: \t0\n",
            );
            h.write(
                "proc/driver/nvidia/gpus/0000:02:00.0/information",
                "Model: y\nDevice Minor: \t1\n",
            );
            std::fs::create_dir_all(h.root.join("sys/bus/pci/devices/0000:01:00.0")).unwrap();
            h
        }

        fn roots(&self) -> Roots {
            Roots {
                proc: self.root.join("proc"),
                sys: self.root.join("sys"),
            }
        }

        fn write(&self, rel: &str, body: &str) {
            let p = self.root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }

        fn process(&self, pid: u32, comm: &str, argv0: &str, fds: &[&str]) {
            self.write(&format!("proc/{pid}/comm"), &format!("{comm}\n"));
            self.write(&format!("proc/{pid}/cmdline"), &format!("{argv0}\0-x\0"));
            std::fs::create_dir_all(self.root.join(format!("proc/{pid}/fd"))).unwrap();
            for (i, t) in fds.iter().enumerate() {
                symlink(t, self.root.join(format!("proc/{pid}/fd/{i}"))).unwrap();
            }
        }

        fn connector(&self, name: &str, status: &str, enabled: &str) {
            let d = format!("sys/bus/pci/devices/0000:01:00.0/drm/card0/card0-{name}");
            self.write(&format!("{d}/status"), &format!("{status}\n"));
            self.write(&format!("{d}/enabled"), &format!("{enabled}\n"));
        }
    }

    impl Drop for Host {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn quiet() -> Host {
        let h = Host::new();
        h.process(1, "systemd", "/sbin/init", &[]);
        h.process(
            500,
            "kf-semmarker-pr",
            "/x/kf-semmarker-probe",
            &["/dev/nvidia0"],
        );
        h.connector("HDMI-A-1", "disconnected", "disabled");
        h
    }

    fn refusals(h: &Host, opt_in: bool) -> Vec<Refusal> {
        guard(&h.roots(), opt_in, 0, 500).err().unwrap_or_default()
    }

    #[test]
    fn a_quiet_box_with_the_opt_in_passes_and_ignores_the_probe_itself() {
        assert_eq!(refusals(&quiet(), true), vec![]);
    }

    #[test]
    fn without_the_opt_in_the_probe_refuses_even_on_a_quiet_box() {
        assert_eq!(refusals(&quiet(), false), vec![Refusal::NoOptIn]);
    }

    #[test]
    fn a_connected_or_enabled_display_connector_refuses() {
        let h = quiet();
        h.connector("DP-1", "connected", "disabled");
        h.connector("eDP-1", "unknown", "enabled");
        assert_eq!(
            refusals(&h, true),
            vec![
                Refusal::Display("card0-DP-1".into()),
                Refusal::Display("card0-eDP-1".into())
            ]
        );
    }

    #[test]
    fn a_process_holding_the_gpu_node_refuses_and_other_gpus_do_not() {
        let h = quiet();
        h.process(
            700,
            "python3",
            "/usr/bin/python3",
            &["/dev/null", "/dev/nvidia0"],
        );
        h.process(701, "blender", "/usr/bin/blender", &["/dev/nvidia1"]);
        h.process(702, "Xorg", "/usr/lib/xorg/Xorg", &["/dev/nvidiactl"]);
        assert_eq!(
            refusals(&h, true),
            vec![Refusal::Holder(Proc {
                pid: 700,
                comm: "python3".into()
            })]
        );
    }

    #[test]
    fn the_persistence_daemon_is_not_a_context_but_a_process_named_like_it_elsewhere_is() {
        let h = quiet();
        h.process(
            800,
            "nvidia-persiste",
            "/usr/bin/nvidia-persistenced",
            &["/dev/nvidia0"],
        );
        assert_eq!(refusals(&h, true), vec![]);
        h.process(801, "nvidia-persist2", "/tmp/x", &["/dev/nvidia0"]);
        assert_eq!(refusals(&h, true).len(), 1);
    }

    #[test]
    fn a_qemu_or_kf3_process_refuses_by_comm_or_by_program_name() {
        let h = quiet();
        h.process(900, "qemu-system-x86", "/usr/bin/qemu-system-x86_64", &[]);
        h.process(901, "launcher", "/opt/kf3-qemu/bin/kf3-run", &[]);
        h.process(902, "kf3-vamgr", "/opt/x", &[]);
        h.process(903, "bash", "/bin/bash", &[]);
        let pids: Vec<u32> = refusals(&h, true)
            .into_iter()
            .filter_map(|r| match r {
                Refusal::Vm(p) => Some(p.pid),
                _ => None,
            })
            .collect();
        assert_eq!(pids, vec![900, 901, 902]);
    }

    #[test]
    fn names_that_are_vms() {
        assert!(is_vm_name("qemu-system-x86_64"));
        assert!(is_vm_name("QEMU-KVM"));
        assert!(is_vm_name("kf3-vamgr"));
        assert!(!is_vm_name("kf-semmarker-pr"));
        assert!(!is_vm_name("systemd"));
    }

    #[test]
    fn an_unlisted_gpu_is_a_refusal() {
        let h = quiet();
        assert_eq!(
            guard(&h.roots(), true, 7, 500).err().unwrap(),
            vec![Refusal::GpuNotFound(7)]
        );
    }

    #[test]
    fn an_unreadable_fd_directory_fails_closed() {
        let h = quiet();
        // `fd` is a plain file: reading it as a directory fails with something other than NotFound.
        h.write("proc/950/comm", "mystery\n");
        h.write("proc/950/fd", "");
        let r = refusals(&h, true);
        assert_eq!(r.len(), 1);
        assert!(
            matches!(&r[0], Refusal::Unreadable(w) if w.contains("pid 950")),
            "{r:?}"
        );
    }

    #[test]
    fn a_process_that_vanished_is_not_a_refusal() {
        let h = quiet();
        h.write("proc/951/comm", "gone\n"); // no fd directory: it ended while being read
        assert_eq!(refusals(&h, true), vec![]);
    }

    #[test]
    fn every_refusal_is_reported_at_once() {
        let h = quiet();
        h.connector("DP-1", "connected", "enabled");
        h.process(700, "python3", "/usr/bin/python3", &["/dev/nvidia0"]);
        h.process(900, "qemu-system-x86", "/usr/bin/qemu-system-x86_64", &[]);
        let r = refusals(&h, false);
        assert_eq!(r.len(), 4, "{r:?}");
        for r in &r {
            assert!(!r.to_string().is_empty());
        }
    }

    #[test]
    fn the_cli_requires_the_flag_to_be_seen_and_bounds_the_attempts() {
        assert_eq!(
            parse_cli(&s(&[])),
            Ok(Cli::Parent {
                attempts: 3,
                opt_in: false
            })
        );
        assert_eq!(
            parse_cli(&s(&[OPT_IN_FLAG, "5"])),
            Ok(Cli::Parent {
                attempts: 5,
                opt_in: true
            })
        );
        assert_eq!(
            parse_cli(&s(&["2", OPT_IN_FLAG])),
            Ok(Cli::Parent {
                attempts: 2,
                opt_in: true
            })
        );
        assert_eq!(
            parse_cli(&s(&["child", "A"])),
            Ok(Cli::Child {
                variant: "A".into(),
                opt_in: false
            })
        );
        assert_eq!(
            parse_cli(&s(&["child", "B", OPT_IN_FLAG])),
            Ok(Cli::Child {
                variant: "B".into(),
                opt_in: true
            })
        );
        for bad in [
            &["0"][..],
            &["11"],
            &["x"],
            &["1", "2"],
            &["child"],
            &["--other"],
        ] {
            assert!(parse_cli(&s(bad)).is_err(), "{bad:?}");
        }
    }

    const POLL: Duration = Duration::from_millis(5);

    #[test]
    fn a_child_that_ends_by_itself_is_not_killed() {
        let mut c = Command::new("true").spawn().unwrap();
        let o = supervise(
            &mut c,
            Duration::from_secs(10),
            Duration::from_secs(5),
            POLL,
        )
        .unwrap();
        assert!(o.exited && !o.killed);
        assert!(o.status.unwrap().success());
    }

    #[test]
    fn a_child_that_outlives_the_watchdog_is_killed_and_reaped() {
        let mut c = Command::new("sleep").arg("60").spawn().unwrap();
        let o = supervise(
            &mut c,
            Duration::from_millis(100),
            Duration::from_secs(10),
            POLL,
        )
        .unwrap();
        assert!(!o.exited && o.killed);
        let st = o.status.expect("reaped after the kill");
        assert!(!st.success());
        assert_eq!(st.signal(), Some(9));
        // Nothing is left to wait for: the child really is gone.
        assert!(c.try_wait().unwrap().is_some());
    }

    struct Stubborn {
        kills: u32,
        polls: u32,
        poll_error: bool,
    }

    impl Supervised for Stubborn {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            self.polls += 1;
            if self.poll_error {
                return Err(io::Error::other("boom"));
            }
            Ok(None)
        }
        fn kill(&mut self) -> io::Result<()> {
            self.kills += 1;
            Err(io::Error::other("already gone"))
        }
    }

    #[test]
    fn a_child_that_is_never_reaped_is_killed_once_and_reported_unreaped() {
        let mut c = Stubborn {
            kills: 0,
            polls: 0,
            poll_error: false,
        };
        let t = Instant::now();
        let o = supervise(
            &mut c,
            Duration::from_millis(20),
            Duration::from_millis(40),
            POLL,
        )
        .unwrap();
        assert_eq!(c.kills, 1, "one kill, whatever its result");
        assert_eq!(
            o,
            Outcome {
                exited: false,
                killed: true,
                status: None
            }
        );
        assert!(
            t.elapsed() >= Duration::from_millis(60),
            "the reap wait was honoured"
        );
        assert!(c.polls >= 4);
    }

    #[test]
    fn a_failed_poll_before_the_watchdog_is_an_error_and_kills_nothing() {
        let mut c = Stubborn {
            kills: 0,
            polls: 0,
            poll_error: true,
        };
        let e =
            supervise(&mut c, Duration::from_secs(1), Duration::from_secs(1), POLL).unwrap_err();
        assert!(e.starts_with("wait: "), "{e}");
        assert_eq!(c.kills, 0);
    }

    #[test]
    fn an_unreaped_child_makes_the_attempt_unexpected() {
        use crate::semprobe::{Attempt, Variant, Verdict, verdict};
        let o = Outcome {
            exited: false,
            killed: true,
            status: None,
        };
        let a = Attempt {
            exited: o.exited && o.status.is_some_and(|s| s.success()),
            word: Some(Variant::B.expected_word()),
            xids: vec![],
        };
        assert_eq!(verdict(Variant::B, &a), Verdict::Unexpected);
    }
}

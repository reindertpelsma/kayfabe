//! ★★★ The **containment prober** — the program that measures whether an isolate's `/dev`
//! descriptor can reach the host filesystem.
//!
//! The escape this exists for was found by measurement and is closed by measurement. It is a
//! committed, re-runnable binary for the same reason `kayfabe-rm-ladder` is: a security
//! property that can only be re-established by editing source and running it by hand is a
//! property nobody re-establishes.
//!
//! It prints one line per probe and judges nothing. The judgement lives in
//! `tests/sandbox_escape.rs`, which runs this in each of its three modes and asserts the
//! **whole table** — including the mode that must still be wide open.
//!
//! ```text
//! $ kayfabe-sandbox-probe --mode sandboxed
//! MODE sandboxed
//! SANDBOX ok
//! PROBE null OPENED
//! PROBE ../etc/shadow DENIED 2
//! ...
//! STD threads=8 spawned-after-the-pivot
//! ```
//!
//! ## ★★ The three modes, and why the ugly one is the important one
//!
//! - `sandboxed` — what the isolate does: enter the sandbox, **then** mint the descriptor.
//! - `dirfd-before-pivot` — the same sandbox, with the two steps **swapped**. This is not a
//!   configuration anybody would ship; it is the regression, kept executable. Ordering is
//!   the entire fix, so a test suite that cannot produce the mis-ordered build cannot claim
//!   to detect its return. This mode must show the escape, and the test asserts that it
//!   does.
//! - `unsandboxed` — no sandbox at all: the state of the world before this work, and the
//!   "before" column of the report.

use kayfabe_linux_raw::DevDir;
use kayfabe_linux_raw::sandbox::{self, SandboxPolicy};
use std::ffi::CString;

/// The probe table. `null` is the non-vacuity anchor — it is present on every Linux host,
/// it is in the policy, and it must open in **every** mode: a run in which everything is
/// denied proves containment only if something is also permitted.
///
/// `nvidiactl` is the real grant and is probed too, but a host with no NVIDIA driver has
/// nothing to bind, so no assertion may depend on it.
/// `zero` and `full` are the *ungranted-node* anchors: present on every Linux host, never
/// in the policy. Containment is not only about `..` — a sandbox that bound the whole of
/// `/dev` would satisfy every `..` assertion and still hand an isolate `/dev/mem`.
///
/// ★ Both `passwd` and `shadow` are probed on purpose. `passwd` is world-readable, so
/// "escaped" and "opened" coincide for root and non-root alike and the assertion can be
/// exact. `shadow` is the one the finding was written about, and for an unprivileged prober
/// it comes back `EACCES` — which is *also* proof of escape (the kernel resolved the name to
/// a real host file) and is why the test's predicate is `!= ENOENT` rather than `OPENED`.
const PROBES: &[&str] = &[
    "null",
    "zero",
    "full",
    "nvidiactl",
    "kvm",
    "mem",
    "../etc/passwd",
    "../../../../../../etc/passwd",
    "../etc/shadow",
    "../../etc/shadow",
    "../proc/1/maps",
    "../root",
];

/// What the sandbox is allowed to contain in this program: the anchor plus, where it
/// exists, the node an isolate would really be granted.
///
/// ★ Deliberately NOT `zero` or `full`, which are probed above: the policy is the whole
/// contents of the sandbox, so a node's absence from this function is the only reason its
/// probe fails.
fn policy() -> SandboxPolicy {
    SandboxPolicy::empty()
        .required("null")
        .optional("nvidiactl")
}

fn report(dev: &DevDir) {
    for name in PROBES {
        let c = CString::new(*name).expect("a probe name has no NUL");
        match dev.can_reach(&c) {
            Ok(()) => println!("PROBE {name} OPENED"),
            Err(kayfabe_linux_raw::RawError::Syscall { errno, .. }) => {
                println!("PROBE {name} DENIED {}", errno.unwrap_or(0));
            }
            Err(other) => println!("PROBE {name} ERROR {other}"),
        }
    }
}

fn main() -> std::process::ExitCode {
    let mut mode = "sandboxed".to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--mode" => match args.next() {
                Some(v) => mode = v,
                None => {
                    eprintln!("--mode needs a value");
                    return std::process::ExitCode::from(64);
                }
            },
            other => {
                eprintln!("unknown flag {other}");
                return std::process::ExitCode::from(64);
            }
        }
    }
    println!("MODE {mode}");

    // ★ In `dirfd-before-pivot` the descriptor is minted here, on the HOST root, and the
    // sandbox is then built underneath it. Nothing else about the run differs — which is
    // what makes the two columns comparable and the ordering the only variable.
    let early = if mode == "dirfd-before-pivot" || mode == "unsandboxed" {
        match DevDir::open(c"/dev") {
            Ok(d) => Some(d),
            Err(e) => {
                println!("SANDBOX failed: opening the host /dev: {e}");
                return std::process::ExitCode::from(1);
            }
        }
    } else {
        None
    };

    let dev = if mode == "unsandboxed" {
        println!("SANDBOX skipped");
        early.expect("the unsandboxed mode always opens the host /dev")
    } else {
        let entered = sandbox::enter(&policy());
        match (entered, early) {
            (Ok(inside), None) => {
                println!("SANDBOX ok");
                inside
            }
            // The mis-ordered build: the sandbox really was entered, and the descriptor
            // minted before it is used anyway. Dropping `inside` is the point — the
            // process keeps the one it should not have.
            (Ok(_inside), Some(outside)) => {
                println!("SANDBOX ok");
                outside
            }
            (Err(e), _) => {
                println!("SANDBOX failed: {e}");
                return std::process::ExitCode::from(1);
            }
        }
    };

    report(&dev);

    // ★ The isolate spawns one thread per pool worker AFTER this point, inside a masked
    // `/proc` and a read-only root. That the Rust runtime survives it is a property of the
    // sandbox, not an assumption about it, so it is measured here too.
    let joined: usize = (0..8)
        .map(|i| std::thread::spawn(move || i))
        .map(|h| h.join().unwrap_or(usize::MAX))
        .filter(|v| *v != usize::MAX)
        .count();
    println!("STD threads={joined} spawned-after-the-pivot");
    std::process::ExitCode::SUCCESS
}

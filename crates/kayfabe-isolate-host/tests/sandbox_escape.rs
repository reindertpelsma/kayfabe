//! ★★★ The host-filesystem escape, and the three columns that prove it is closed.
//!
//! The defect was **measured**, so the fix is measured the same way, over the same probe
//! table, against real child processes on a real kernel:
//!
//! | probe | `unsandboxed` | `dirfd-before-pivot` | `sandboxed` |
//! |---|---|---|---|
//! | `null` (the granted node) | OPENED | OPENED | OPENED |
//! | `zero`, `full` (ungranted nodes) | OPENED | OPENED | `ENOENT` |
//! | `../etc/passwd` | **OPENED** | **OPENED** | `ENOENT` |
//! | `../../../../../../etc/passwd` | **OPENED** | **OPENED** | `ENOENT` |
//! | `../etc/shadow` | **OPENED** as root, `EACCES` otherwise | same | `ENOENT` |
//! | `../proc/1/maps` | **OPENED** as root, `EACCES` otherwise | same | `ENOENT` |
//!
//! ★ `EACCES` counts as an escape and the tests say so: it means the kernel **resolved the
//! name to a real host file** and only then checked permissions. Asserting `OPENED` would
//! make this whole file silently stop testing anything the day it ran unprivileged.
//!
//! ## ★★ The middle column is the reason this file is worth having
//!
//! `dirfd-before-pivot` builds the **whole** sandbox — namespace, tmpfs, binds,
//! `pivot_root`, read-only reseal — and simply mints the descriptor one step too early. It
//! is the mis-ordered build, kept executable, and it must still be wide open. Without it the
//! right-hand column is only evidence that *something* denies those names, and the single
//! most likely regression — someone adds a `pivot_root` and leaves the `open` where it was,
//! or moves the `open` back for convenience — would land green.
//!
//! A gate never seen to fail is not evidence. This one is seen to fail, in CI, every run.
//!
//! ## ★★ The second half: the privilege boundary, which fails independently
//!
//! A perfect filesystem boundary around a process holding `CapEff = 000001ffffffffff` is a
//! boundary with a key taped to it — `CAP_SYS_PTRACE` reaches the **parent**'s memory, and
//! the parent holds the KVM descriptor, all guest RAM and every other isolate's socket. So
//! the same three modes also carry the capability columns:
//!
//! | reading | `unsandboxed` | `sandboxed` |
//! |---|---|---|
//! | `CapEff` | **the parent's, verbatim** (`000001ffffffffff` as root) | `0` |
//! | `CapBnd` (the ceiling) | the parent's | `0` |
//! | `NoNewPrivs` / `dumpable` | 1 / **1** | 1 / **0** |
//! | user namespace | the parent's | **its own** |
//!
//! The left column is the standing bite, exactly as it is for the escape. Each capability
//! reading is taken **twice, by different instruments** — `capget`+`prctl` and the kernel's
//! own `/proc/self/status` text — because containment here looks like a wall of zeroes and
//! so does a broken prober.
//!
//! ## Why a child process
//!
//! Entering the sandbox rewrites the calling process's mount namespace, root and
//! credentials. A test that did it in-process would contain the rest of the test binary and
//! strip its privilege, so the probe is a committed program (`kayfabe-sandbox-probe`) and
//! these tests read its output.

use std::collections::BTreeMap;

const PROBE: &str = env!("CARGO_BIN_EXE_kayfabe-sandbox-probe");

/// One probe's outcome, exactly as the kernel reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reach {
    /// `openat` succeeded — the descriptor can name this path.
    Opened,
    /// `openat` refused, with this exact `errno`.
    Denied(i32),
}

/// The prober's whole report: the status line, the probe table, and the privilege lines.
struct Report {
    status: String,
    table: BTreeMap<String, Reach>,
    /// `PRIV` — `capget`/`prctl`, the reading `sandbox::enter` itself fails closed on.
    priv_fields: BTreeMap<String, String>,
    /// `STATUS` — the kernel's own text for the same state, through a descriptor opened
    /// before the pivot. An independent instrument for the same measurement.
    status_fields: BTreeMap<String, String>,
}

impl Report {
    /// One `PRIV` field as the u64 bitmap it is.
    fn priv_bits(&self, field: &str) -> u64 {
        let raw = self.priv_fields.get(field).unwrap_or_else(|| {
            panic!("the prober printed no PRIV {field}: {:?}", self.priv_fields)
        });
        u64::from_str_radix(raw, 16).unwrap_or_else(|_| panic!("unparseable PRIV {field}={raw}"))
    }

    /// The same field as `/proc/self/status` rendered it. `CapEff` ⇄ `eff`, etc.
    fn status_bits(&self, field: &str) -> u64 {
        let raw = self.status_fields.get(field).unwrap_or_else(|| {
            panic!(
                "the prober printed no STATUS {field}: {:?}",
                self.status_fields
            )
        });
        u64::from_str_radix(raw, 16).unwrap_or_else(|_| panic!("unparseable STATUS {field}={raw}"))
    }
}

/// Run the prober in `mode` and return `(sandbox status line, probe table)`.
fn run(mode: &str) -> (String, BTreeMap<String, Reach>) {
    let r = run_full(mode, &[]);
    (r.status, r.table)
}

/// Run the prober and parse its whole report.
fn run_full(mode: &str, extra: &[&str]) -> Report {
    let out = std::process::Command::new(PROBE)
        .arg("--mode")
        .arg(mode)
        .args(extra)
        .output()
        .unwrap_or_else(|e| panic!("spawning {PROBE}: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "the prober failed in mode {mode}: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status.code()
    );
    parse(&stdout, mode)
}

/// Parse one prober report. Shared by the capturing and the holding paths, so the two can
/// never drift into disagreeing about what a line means.
fn parse(stdout: &str, mode: &str) -> Report {
    let mut status = String::new();
    let mut priv_fields = BTreeMap::new();
    let mut status_fields = BTreeMap::new();
    let mut table = BTreeMap::new();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("SANDBOX ") {
            status = rest.to_owned();
        } else if let Some(rest) = line.strip_prefix("PROBE ") {
            let (name, verdict) = rest
                .rsplit_once(' ')
                .unwrap_or_else(|| panic!("unparseable probe line {line:?}"));
            let reach = match verdict {
                "OPENED" => Reach::Opened,
                errno => Reach::Denied(
                    errno
                        .parse()
                        .unwrap_or_else(|_| panic!("unparseable errno in {line:?}")),
                ),
            };
            // `DENIED <errno>` splits on the LAST space, so `name` still carries the
            // "DENIED" word; strip it rather than guessing at field counts.
            let name = name.strip_suffix(" DENIED").unwrap_or(name);
            table.insert(name.to_owned(), reach);
        } else if let Some(rest) = line.strip_prefix("PRIV ") {
            for field in rest.split_whitespace() {
                if let Some((k, v)) = field.split_once('=') {
                    priv_fields.insert(k.to_owned(), v.to_owned());
                }
            }
        } else if let Some(rest) = line.strip_prefix("STATUS ")
            && let Some((k, v)) = rest.split_once(':')
        {
            status_fields.insert(k.to_owned(), v.trim().to_owned());
        }
    }
    assert!(
        !table.is_empty(),
        "the prober printed no probe lines in mode {mode}:\n{stdout}"
    );
    Report {
        status,
        table,
        priv_fields,
        status_fields,
    }
}

/// Linux's `ENOENT`. Spelled out rather than pulled from `libc`, which this crate
/// deliberately does not depend on — `kayfabe-linux-raw` is the one crate that speaks to the
/// OS. The value is fixed by the Linux ABI on every architecture.
const ENOENT: i32 = 2;

/// ★★ The predicate the whole file turns on.
///
/// `ENOENT` is the only answer that means *"this name does not exist in my world"*. Anything
/// else — `OPENED`, or `EACCES` from an unprivileged prober meeting a root-owned file —
/// means the kernel **resolved the name to a real host object**, which is the escape whether
/// or not the caller was allowed to read what it found.
///
/// Stated as a function because getting it wrong in the obvious direction (`== OPENED`) is
/// how this test would pass as root and quietly stop asserting anything on an unprivileged
/// CI runner.
fn escaped(reach: Option<&Reach>) -> bool {
    // Written as an equality against a value, not as a `matches!` pattern: a constant in
    // pattern position is one rename away from becoming a catch-all binding that makes this
    // function return `false` for everything.
    reach != Some(&Reach::Denied(ENOENT))
}

/// Escapes that are **world-readable**, so "resolved" and "opened" coincide for every user
/// and the assertion can be exact in both directions.
const ESCAPES_EXACT: &[&str] = &["../etc/passwd", "../../../../../../etc/passwd"];

/// Escapes to root-owned objects — the names the finding was actually written about. An
/// unprivileged prober gets `EACCES` for these, which is why they are asserted with
/// [`escaped`] rather than with `OPENED`.
const ESCAPES_PRIVILEGED: &[&str] = &[
    "../etc/shadow",
    "../../etc/shadow",
    "../proc/1/maps",
    "../root",
];

/// Device nodes that exist on **every** Linux host and are not in the prober's policy.
/// Containment is not only about `..`: a sandbox that bound the whole of `/dev` would
/// satisfy every assertion above and still hand an isolate `/dev/mem`.
///
/// `zero`/`full` rather than `kvm`/`mem` deliberately — a runner with no `/dev/kvm` makes
/// the `kvm` row `ENOENT` in *every* column, and an assertion that is true because the
/// fixture is missing is not an assertion.
const UNGRANTED_NODES: &[&str] = &["zero", "full"];

// =====================================================================================

/// ★ The "before" column, committed. This is the world as it was: a `/dev` `O_PATH`
/// descriptor with no namespace under it, which is what the parent used to grant on fd 4.
///
/// It is an assertion and not a comment because the whole argument rests on it. If this ever
/// goes green-by-denial — a kernel that bounds `O_PATH` dirfds, a runner whose `/etc/shadow`
/// is absent — then the `sandboxed` column below stops meaning anything and somebody has to
/// know.
#[test]
fn without_a_sandbox_a_dev_descriptor_reaches_the_whole_host_filesystem() {
    let (status, table) = run("unsandboxed");
    assert_eq!(status, "skipped");
    assert_eq!(
        table.get("null"),
        Some(&Reach::Opened),
        "the anchor must open, or nothing below is a comparison"
    );
    for escape in ESCAPES_EXACT {
        assert_eq!(
            table.get(*escape),
            Some(&Reach::Opened),
            "★ the premise of this whole test file has changed: {escape} no longer escapes \
             an unbounded /dev descriptor. Re-measure before trusting the sandboxed column."
        );
    }
    for escape in ESCAPES_PRIVILEGED {
        assert!(
            escaped(table.get(*escape)),
            "★ premise changed: {escape} did not resolve out of an unbounded /dev \
             descriptor (got {:?})",
            table.get(*escape)
        );
    }
    for node in UNGRANTED_NODES {
        assert_eq!(
            table.get(*node),
            Some(&Reach::Opened),
            "the ungranted-node anchors must be reachable BEFORE the sandbox, or their \
             absence afterwards proves nothing"
        );
    }
}

/// ★★★ **The deliverable.** Inside the sandbox the descriptor names the granted node and
/// nothing else — every escape is exactly `ENOENT`, because inside the sandbox root those
/// paths do not exist at all.
#[test]
fn the_sandboxed_descriptor_cannot_name_anything_above_it() {
    kayfabe_linux_raw::require_sandbox!("the_sandboxed_descriptor_cannot_name_anything_above_it");
    let (status, table) = run("sandboxed");
    assert_eq!(status, "ok", "the sandbox must have been built");

    // Non-vacuity FIRST: the grant still works. A table of nothing but denials is also what
    // a broken prober produces.
    assert_eq!(
        table.get("null"),
        Some(&Reach::Opened),
        "the granted node stopped opening — this is a broken sandbox, not a safe one"
    );

    for escape in ESCAPES_EXACT.iter().chain(ESCAPES_PRIVILEGED) {
        assert_eq!(
            table.get(*escape),
            Some(&Reach::Denied(ENOENT)),
            "{escape} is reachable from the sandboxed descriptor"
        );
    }
    for node in UNGRANTED_NODES {
        assert_eq!(
            table.get(*node),
            Some(&Reach::Denied(ENOENT)),
            "/dev/{node} is inside the sandbox and the policy never named it"
        );
    }

    // ★ The universe is DERIVED, not restated: every `..` row the prober probes must be
    // covered by one of the two lists above. Shortening a list here would otherwise weaken
    // the gate with no red test — `gates_quantified_over_a_list`.
    for probed in table.keys() {
        if !probed.starts_with("..") {
            continue;
        }
        assert!(
            ESCAPES_EXACT.contains(&probed.as_str())
                || ESCAPES_PRIVILEGED.contains(&probed.as_str()),
            "the prober probes {probed} and no assertion in this file covers it"
        );
    }
}

/// ★★★ **The bite check, as a permanent test.** Same sandbox, descriptor minted one step
/// too early, escape fully restored.
///
/// This is the single most valuable assertion in the file: it proves the test above detects
/// the exact regression that matters, rather than detecting the presence of a `pivot_root`
/// call somewhere. Ordering is the fix; this is the ordering being wrong.
#[test]
fn a_descriptor_minted_before_the_pivot_still_escapes_everything() {
    kayfabe_linux_raw::require_sandbox!(
        "a_descriptor_minted_before_the_pivot_still_escapes_everything"
    );
    let (status, table) = run("dirfd-before-pivot");
    assert_eq!(
        status, "ok",
        "the sandbox itself must have been built — the ONLY difference from the passing \
         mode is when the descriptor was opened"
    );
    for escape in ESCAPES_EXACT {
        assert_eq!(
            table.get(*escape),
            Some(&Reach::Opened),
            "★ {escape} did NOT come back. Either the mis-ordered mode stopped being \
             mis-ordered, or something other than ordering is doing the containment — and \
             in both cases the passing test above is no longer evidence for what it claims."
        );
    }
    for escape in ESCAPES_PRIVILEGED {
        assert!(
            escaped(table.get(*escape)),
            "★ {escape} did NOT come back (got {:?}) — see the message above",
            table.get(*escape)
        );
    }
}

// =====================================================================================
// The privilege boundary — the other half, and it fails independently of the first
// =====================================================================================

/// ★★ **The standing bite for the capability drop**, exactly as
/// [`without_a_sandbox_a_dev_descriptor_reaches_the_whole_host_filesystem`] is for the
/// escape: the "before" column, committed and re-run every time.
///
/// A child spawned without the sandbox inherits its parent's capabilities **verbatim**. On
/// the host this ships on that is `CapEff = 000001ffffffffff` — every capability, including
/// `CAP_SYS_PTRACE`, which reaches the parent's memory (the KVM descriptor, all guest RAM,
/// every other isolate's socket) through `process_vm_readv` regardless of Yama.
///
/// ★ On an unprivileged runner the parent has no capabilities to inherit, so the comparison
/// is still exact but the *bite* is vacuous. That is said out loud on stderr rather than
/// left for someone to discover, because a bite check that quietly stops biting is the
/// failure this file is written against.
#[test]
fn a_child_without_the_sandbox_inherits_every_capability_its_parent_holds() {
    let ours = kayfabe_linux_raw::sandbox::privileges().expect("our own privilege state");
    let r = run_full("unsandboxed", &[]);
    assert_eq!(r.status, "skipped");
    assert_eq!(
        r.priv_bits("eff"),
        ours.effective,
        "an unsandboxed child's effective set must be its parent's, verbatim"
    );
    assert_eq!(
        r.priv_bits("bnd"),
        ours.bounding,
        "an unsandboxed child's bounding set must be its parent's, verbatim"
    );
    if ours.effective == 0 {
        eprintln!(
            "SANDBOX-GATE: NOTE a_child_without_the_sandbox_inherits_every_capability_its_parent_holds \
             — this process holds NO capabilities, so the inheritance assertion is exact but the \
             bite is vacuous here. The privileged bite is the one that matters and it needs a \
             privileged runner."
        );
    } else {
        assert_ne!(
            r.priv_bits("eff"),
            0,
            "the bite: a privileged parent's child is privileged too"
        );
    }
}

/// ★★★ **The deliverable.** Inside the sandbox the process holds nothing: no capability in
/// any set, `no_new_privs` set, not dumpable.
///
/// The bounding set is asserted too, and separately: an empty effective set over a full
/// bounding set is not a contained process, it is one `exec` away from a privileged one.
///
/// ★ Both instruments are asserted and then asserted **against each other**. A wall of
/// zeroes is what containment looks like and also what a broken prober prints; the
/// cross-check is the difference (`suspect_the_instrument_first`).
#[test]
fn the_sandboxed_child_holds_no_capability_at_all() {
    kayfabe_linux_raw::require_sandbox!("the_sandboxed_child_holds_no_capability_at_all");
    let r = run_full("sandboxed", &[]);
    assert_eq!(r.status, "ok", "the sandbox must have been built");

    // Non-vacuity FIRST, as everywhere in this file: the granted node still opens, so this
    // is a contained isolate and not a broken one.
    assert_eq!(
        r.table.get("null"),
        Some(&Reach::Opened),
        "the granted node stopped opening — this is a broken sandbox, not a safe one"
    );

    for set in ["eff", "prm", "inh", "amb"] {
        assert_eq!(
            r.priv_bits(set),
            0,
            "capability set {set} survived the drop"
        );
    }
    assert_eq!(
        r.priv_fields.get("nnp").map(String::as_str),
        Some("1"),
        "no_new_privs is not set"
    );
    assert_eq!(
        r.priv_fields.get("dumpable").map(String::as_str),
        Some("0"),
        "the sandboxed process is still dumpable, so a same-uid peer may ptrace it"
    );

    // ★ The independent instrument: the kernel's own text, read through a descriptor
    // opened before the pivot. If these two ever disagree, the assertions above are not
    // evidence of anything.
    for (priv_field, status_field) in [
        ("eff", "CapEff"),
        ("prm", "CapPrm"),
        ("inh", "CapInh"),
        ("bnd", "CapBnd"),
        ("amb", "CapAmb"),
    ] {
        assert_eq!(
            r.priv_bits(priv_field),
            r.status_bits(status_field),
            "capget and /proc/self/status disagree about {status_field}"
        );
    }
    assert_eq!(
        r.status_fields.get("NoNewPrivs").map(String::as_str),
        Some("1")
    );
}

/// ★ The **latent** half, and the one a capability drop most often gets wrong: a process
/// that dropped its effective set while leaving the ceiling in place is still one
/// file-capability `exec` from privilege.
///
/// Gated separately on a privileged parent, because a process that never held
/// `CAP_SETPCAP` cannot empty its bounding set — and `sandbox::enter` deliberately does not
/// refuse such a host, since `no_new_privs` already makes the set unusable there.
#[test]
fn the_sandboxed_childs_capability_ceiling_is_empty_when_it_could_be_emptied() {
    // ★ The privilege precondition is checked BEFORE the namespace gate, so exactly one
    // marker line is printed for this test whichever way it goes out — the count CI takes
    // as its floor is only meaningful if one test emits one line.
    let ours = kayfabe_linux_raw::sandbox::privileges().expect("our own privilege state");
    if ours.effective == 0 {
        kayfabe_linux_raw::sandbox::report_gate(
            "the_sandboxed_childs_capability_ceiling_is_empty_when_it_could_be_emptied",
            false,
            "hold any capability, so it could not have emptied a bounding set either",
        );
        return;
    }
    kayfabe_linux_raw::require_sandbox!(
        "the_sandboxed_childs_capability_ceiling_is_empty_when_it_could_be_emptied"
    );
    let r = run_full("sandboxed", &[]);
    assert_ne!(
        ours.bounding, 0,
        "the premise: our own ceiling is not empty"
    );
    assert_eq!(
        r.priv_bits("bnd"),
        0,
        "the capability bounding set survived — privilege is set aside, not surrendered"
    );
}

/// ★★★ **The privilege boundary against the PARENT**, which is the reason the user
/// namespace is taken at all.
///
/// `ptrace_may_access` lets a same-uid process attach with no capability whatsoever — so a
/// child that is uid 0 next to a uid 0 parent can `process_vm_readv` it, and the parent
/// holds the KVM descriptor and all guest RAM. Across a **user namespace** boundary the
/// same check demands `CAP_SYS_PTRACE` *in the tracee's* namespace, and the sandboxed child
/// has no capability in any namespace (asserted above). This test measures the mechanism —
/// that the child really is in its own user namespace, and its own mount/net/IPC/UTS ones.
///
/// It cannot be measured from inside: there is no `/proc` in the sandbox, and a namespace id
/// means nothing except in comparison with somebody else's. So the prober holds, and the
/// parent reads `/proc/<pid>/ns/*` — an edge, not a sleep.
#[test]
fn the_sandboxed_child_lives_in_its_own_user_namespace() {
    kayfabe_linux_raw::require_user_namespace!(
        "the_sandboxed_child_lives_in_its_own_user_namespace"
    );
    use std::io::{BufRead as _, BufReader, Read as _};

    let mut child = std::process::Command::new(PROBE)
        .arg("--mode")
        .arg("sandboxed")
        .arg("--hold")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawning the prober");
    let pid = child.id();
    let mut lines = String::new();
    {
        let out = child.stdout.take().expect("piped stdout");
        let mut reader = BufReader::new(out);
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).expect("reading the prober");
            assert_ne!(n, 0, "the prober exited before HOLD:\n{lines}");
            let done = line.trim_end() == "HOLD";
            lines.push_str(&line);
            if done {
                break;
            }
        }

        // The child is now fully inside the sandbox. Compare every namespace it should have
        // left. `/proc/<pid>/ns/*` is a symlink whose target is `user:[4026532xxx]`.
        let mut differed = Vec::new();
        for ns in ["user", "mnt", "net", "ipc", "uts"] {
            let theirs = std::fs::read_link(format!("/proc/{pid}/ns/{ns}"));
            let ours = std::fs::read_link(format!("/proc/self/ns/{ns}"));
            match (theirs, ours) {
                (Ok(t), Ok(o)) => {
                    assert_ne!(
                        t, o,
                        "the sandboxed child shares our {ns} namespace ({t:?}) — it did not \
                         leave it, and for `user` that means a same-uid ptrace of THIS \
                         process is permitted"
                    );
                    differed.push(ns);
                }
                // `PR_SET_DUMPABLE 0` reparents /proc/<pid> to root, so an unprivileged
                // parent cannot read these at all. Say so rather than pass quietly.
                (t, o) => eprintln!(
                    "SANDBOX-GATE: NOTE {ns} namespace not comparable (theirs={t:?} ours={o:?})"
                ),
            }
        }
        assert!(
            differed.contains(&"user"),
            "the user namespace — the one the ptrace refusal rests on — could not be \
             compared at all, so this test asserted nothing about it"
        );

        // Release it: closing stdin is the edge that ends its `read_line`.
        drop(child.stdin.take());
        let mut rest = String::new();
        reader.read_to_string(&mut rest).expect("draining");
        lines.push_str(&rest);
    }
    let status = child.wait().expect("reaping the prober");
    assert!(status.success(), "the prober failed:\n{lines}");
    let report = parse(&lines, "sandboxed");
    assert_eq!(report.status, "ok");
    assert_eq!(
        report.priv_bits("eff"),
        0,
        "the held child must also be the unprivileged one"
    );
}

/// The sandbox is not only a boundary, it is a place the isolate has to keep running in:
/// a masked `/proc` and a read-only root are exactly the shape that breaks a runtime's
/// lazy initialisation. The isolate spawns one thread per pool worker after this point.
#[test]
fn the_runtime_still_spawns_threads_inside_the_sandbox() {
    kayfabe_linux_raw::require_sandbox!("the_runtime_still_spawns_threads_inside_the_sandbox");
    let out = std::process::Command::new(PROBE)
        .arg("--mode")
        .arg("sandboxed")
        .output()
        .expect("spawning the prober");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("STD threads=8 spawned-after-the-pivot"),
        "the runtime did not survive the pivot:\n{stdout}"
    );
}

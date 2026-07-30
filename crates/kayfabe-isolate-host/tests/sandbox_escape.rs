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
//! ## Why a child process
//!
//! Entering the sandbox rewrites the calling process's mount namespace and root. A test that
//! did it in-process would contain the rest of the test binary, so the probe is a committed
//! program (`kayfabe-sandbox-probe`) and these tests read its output.

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

/// Run the prober in `mode` and return `(sandbox status line, probe table)`.
fn run(mode: &str) -> (String, BTreeMap<String, Reach>) {
    let out = std::process::Command::new(PROBE)
        .arg("--mode")
        .arg(mode)
        .output()
        .unwrap_or_else(|e| panic!("spawning {PROBE}: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "the prober failed in mode {mode}: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status.code()
    );

    let mut status = String::new();
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
        }
    }
    assert!(
        !table.is_empty(),
        "the prober printed no probe lines in mode {mode}:\n{stdout}"
    );
    (status, table)
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

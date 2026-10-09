//! ★ The witness of the quiet-logging rule (`docs/design/V3_NONSTALL_THREADS.md` §3.B): no raw
//! `eprintln!` / `println!` / `eprint!` / `print!` / `dbg!` in the crates that run inside the VMM
//! (the register drainer, the act thread, the workers, the VA thread and the display worker all call
//! into them). `eprintln!` panics when stderr cannot be written (`ENOSPC`, `EPIPE`) — which kills the
//! calling thread, silently for the drainer — and its duration is unmeasured. The one entry point is
//! `kf_util::klog!` / `klog_limited!` / `klog_trace!` (`kf-util/src/log.rs`).

use std::path::{Path, PathBuf};

/// The VMM crates (relative to the workspace's `crates/`). `kf-cuda` is not listed: it does not
/// depend on `kf-util` (a deliberately light crate) and its two prints run once, on the CUDA bring-up
/// thread at realize.
const CRATES: &[&str] = &[
    "kf-qemu",
    "kf-chan",
    "kf-rm",
    "kf-gsp",
    "kf-mem",
    "kf-host",
    "kf-core",
    "kf-trap",
    "kf-linux-raw",
];

/// Files with a reason to print raw. `ioctltrace.rs`: the `KF_IOCTL_TRACE` diagnostic, whose output is
/// written from a deadline/abort path that must not allocate or take the logger's counters (default
/// off; the status line prints `PERTURBING_DIAGNOSTIC_ON(KF_IOCTL_TRACE)` while it is armed).
const ALLOWED: &[&str] = &["kf-linux-raw/src/ioctltrace.rs"];

const FORBIDDEN: &[&str] = &["eprintln!", "println!", "eprint!", "print!(", "dbg!("];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            // Binaries are programs of their own, with their own stderr.
            if p.file_name().is_some_and(|n| n == "bin") {
                continue;
            }
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Raw print sites in `text`: `(line number, line)` — comment lines are not code.
fn raw_prints(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//"))
        .filter(|(_, l)| {
            FORBIDDEN.iter().any(|f| {
                // `print!(` also matches `eprint!(`/`println!(`'s tail only as a substring of a
                // longer macro name: match whole macro names.
                l.match_indices(f).any(|(i, _)| {
                    let before = l[..i].chars().next_back();
                    !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                })
            })
        })
        .map(|(i, l)| (i + 1, l.trim().to_string()))
        .collect()
}

#[test]
fn no_raw_print_in_the_crates_that_run_inside_the_vmm() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut bad = Vec::new();
    let mut scanned = 0;
    for c in CRATES {
        let mut files = Vec::new();
        rs_files(&crates.join(c).join("src"), &mut files);
        for f in files {
            let rel = format!(
                "{c}/{}",
                f.strip_prefix(crates.join(c))
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            if ALLOWED.contains(&rel.as_str()) {
                continue;
            }
            scanned += 1;
            let text = std::fs::read_to_string(&f).unwrap();
            for (n, l) in raw_prints(&text) {
                bad.push(format!("{rel}:{n}: {l}"));
            }
        }
    }
    assert!(scanned > 50, "the scan found only {scanned} files");
    assert!(
        bad.is_empty(),
        "a raw print in code that runs inside the VMM — use kf_util::klog!/klog_limited!/klog_trace! \
         (eprintln! panics on a failed write and kills the thread):\n{}",
        bad.join("\n")
    );
}

/// The scanner itself: it must catch the macros, and not their longer namesakes or comments.
#[test]
fn the_scanner_catches_a_raw_print_and_ignores_comments_and_namesakes() {
    let src = "fn f() {\n    eprintln!(\"x\");\n    // eprintln!(\"y\");\n    kf_util::klog!(\"z\");\n    let _ = format_args!(\"a\");\n    foo_println!(\"q\");\n    println!(\"w\");\n}\n";
    let hits = raw_prints(src);
    assert_eq!(
        hits.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        vec![2, 7],
        "{hits:?}"
    );
}

//! ★★★ Stamp the archive with the revision it was built from.
//!
//! The 2026-08-01 bench post-mortem: *"the previous bench binary could not name its own
//! revision. It contained exactly one 40-hex string, `8bab26f4…`, which is not a commit in
//! either repo — a build-id. Its only provenance was the worktree it came from, which was
//! 19 commits behind master."* That is the same trap `CLAUDE.md` already records against
//! the C bench ("silently served a binary built from `862c7c2` for weeks"), and it costs a
//! whole measurement every time it fires, because a bench result attributed to HEAD was not
//! HEAD's.
//!
//! ⊘ **It must survive a release build.** A release build strips enum-variant names and
//! inlines hex constants, so `strings` scored **zero** on every marker in the previous
//! binary. A string literal is different in kind: it lives in `.rodata` and, kept alive by
//! a `#[used]` anchor, is not something the optimiser may drop.
//!
//! Fallback is `"unknown"`, never a guess and never a build failure: this crate is built in
//! CI, in a vendored tarball and inside a hypervisor tree, and only one of those is a git
//! checkout. A binary that says `unknown` is honest; a binary that says a wrong sha is the
//! bug this file exists to close.

use std::process::Command;

fn main() {
    // ★ Rerun when HEAD moves. Without this the stamp is baked at first compile and every
    // later archive from the same target dir carries a stale sha — which would be strictly
    // worse than no stamp, for the reason the module doc gives.
    for p in ["../../.git/HEAD", "../../.git/refs/heads"] {
        println!("cargo:rerun-if-changed={p}");
    }
    println!("cargo:rerun-if-env-changed=KAYFABE_GIT_SHA");

    let sha = std::env::var("KAYFABE_GIT_SHA").ok().or_else(|| {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8(out.stdout).ok()?.trim().to_owned();
        // ★ Shape-checked, not trusted: exactly 40 lowercase hex characters. A `git` that
        // printed a warning, a detached message or an empty line becomes `unknown` rather
        // than a marker that looks like provenance and is not.
        (s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())).then_some(s)
    });

    // ★ And whether the tree was DIRTY, which is a separate question from which commit it
    // was on — the bench has served an archive built from edits that were never committed,
    // and a clean-looking sha would have hidden it.
    let dirty = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false);

    println!(
        "cargo:rustc-env=KAYFABE_BUILD_REV={}{}",
        sha.as_deref().unwrap_or("unknown"),
        if dirty { "-dirty" } else { "" }
    );
}

//! # ★★★ Files that are soundness-critical WITHOUT the keyword (`l1_os_shell.md` §4.2.1.1)
//!
//! Owner's axiom, 2026-08-06: *"the raw layer must assume safe code is bugged, broken, skips
//! validation, and it may still not violate. It facilitates extra functions to safe code, that
//! are still safe — not that safe code calls raw functions or handles pointers."*
//!
//! The filing consequence: **the keyword is not the boundary.** It marks five *operations* and
//! no *reasoning* at all, and Rust's model puts the soundness boundary at the **privacy
//! boundary** — safe code that can reach an abstraction's private fields is part of its safety
//! argument. So a file can be soundness-critical with none of the keyword in it — and the
//! keyword is what `ls`-based auditing can see.
//!
//! ## ⊘ What this file does NOT do, because something else already does it
//!
//! The keyword half — *every `.rs` file using the keyword is named `*_unsafe.rs`* — is already
//! gated, in `scripts/ci_gates.sh`'s **Unsafe-surface gate**, whose rationale is that an
//! auditor must be able to enumerate that whole surface with `ls`. `[measured]`
//! 2026-08-06: it holds exactly, 13 files each side.
//!
//! ⚠ A first cut of this file re-implemented that scan in Rust and **tripped that very gate on
//! its own detection strings** — a duplicate that also broke the thing it duplicated. It is
//! deleted rather than fixed. The scan below therefore matches no keyword text at all.
//!
//! ## What is genuinely missing, and is here
//!
//! That gate is a perfect score *against the keyword criterion*, which is precisely why it
//! cannot see the class above. It proves only that nobody hid the keyword under an innocent
//! name. The residue — **safe files carrying reasoning the raw layer depends on** — is
//! enumerated below by name.
//!
//! ⊘ **This is not an allowlist, and the distinction matters** because the shell gate says in
//! as many words *"never add an allowlist: the moment this gate needs a judgement call, `ls`
//! stops being the audit."* An allowlist would excuse a file from the naming rule. This list
//! does the opposite — it names files the rule must ALSO cover. It can only ever make the
//! surface larger, never smaller, so `ls` remains the audit.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Crates whose sources this gate walks — the two permitted to hold a raw surface at all.
const CRATES: &[&str] = &["kayfabe-linux-raw", "kayfabe-qemu-raw"];

/// ★★★ **The soundness-critical SAFE files** — files the `ls` audit cannot see, that
/// nevertheless carry reasoning a raw-surface block depends on, and therefore owe the
/// `_unsafe` suffix. §4.2.1.1 names the three qualifying constructs.
///
/// **It is empty, and empty is a CLAIM rather than an absence.** It asserts that **no file
/// outside the `*_unsafe.rs` set** carries reasoning a raw block depends on — that the `ls`
/// audit's field of view is complete, not that the code inside it is finished. That second,
/// much larger statement is [`UNDISCHARGED_REDERIVATION`]'s, and it is **false today**.
///
/// The worked standard is `Region::word_at` (`src/mapping_unsafe.rs`): it re-checks alignment,
/// then bounds the access with `bounds::checked_span(self.map.len_bytes(), …)` — from the
/// **mapping's own length**, never from what the caller claimed. Safe code passing a wild
/// offset gets a `RawError`.
///
/// ★ Which is exactly why offsets and lengths are *allowed* to be computed in safe files: the
/// rule is not *"never compute an offset in safe code"*, it is *"never let the raw layer trust
/// one"*. Owner, 2026-08-06: *"offset/length free to exist outside, as long as every use in
/// the raw layer revalidates it."*
///
/// ⊘ Adding a name here is a design decision, not a filing one. Prefer the better fix — make
/// the raw side re-validate, which empties the entry instead of decorating it.
const SOUNDNESS_CRITICAL_SAFE_FILES: &[&str] = &[];

/// ★★★ **Raw blocks that still trust a value they did not re-derive.** Open, by name.
///
/// ## ⚠ Why this list exists: the empty one above used to say something much bigger
///
/// Until 2026-08-07 [`SOUNDNESS_CRITICAL_SAFE_FILES`] was documented as asserting *"no block
/// in this tree's raw layer trusts a value it did not re-derive itself"* — a statement about
/// **every block in the crate**, resting on a list that quantifies over **files outside it**.
/// An audit refuted it, and two of its findings were real:
///
/// - `CharDevice::ioctl` never compared `_IOC_SIZE(request)` against `arg.len()`, so a safe
///   caller whose request number and buffer had drifted apart made the **driver** copy up to
///   16 383 bytes into a buffer that might hold 32. Fixed at `6d82fb4`, and the `// SAFETY:`
///   block there had **named the missing check and assigned it to the caller**.
/// - `recv_with_fds`'s `SCM_RIGHTS` walk derived its descriptor count from `cmsg_len` alone.
///   Fixed by `scm_unsafe::descriptors_in`. ⊘ Note this one was reported as peer-reachable and
///   **is not** — see that function for who actually writes a `cmsghdr`.
///
/// ★ The lesson is the one `gates_quantified_over_a_list` keeps teaching from the other side:
/// an empty list is only as true as the sentence attached to it, and **nothing was checking
/// the sentence.** So the residue gets a list of its own, and the list is not empty.
///
/// ## ⊘ What this does NOT do
///
/// It detects nothing. Every row below is a *known* gap someone has to close; none of them is
/// currently reachable from a peer, which is why they are recorded rather than embargoed. What
/// the list buys is that the count cannot silently drop — closing one means deleting its row,
/// and deleting a row is a diff a reviewer sees.
///
/// `[measured]` 2026-08-07 — every row read at the cited line by me, not transcribed.
const UNDISCHARGED_REDERIVATION: &[(&str, &str)] = &[
    (
        "crates/kayfabe-linux-raw/src/chardev_unsafe.rs",
        "★★ THE SAME DEFECT AS THE ONE JUST FIXED, one level in: `Indirect` knows its buffer's \
         length and offers it as `len()`, but the argument's COMPANION SIZE FIELD — the number \
         the driver actually copies by — is set by the caller and never checked against \
         `buf.len()`. The request-size bound added at 6d82fb4 covers the top-level argument \
         only; a pointer patched into it can still name a buffer smaller than the size field \
         beside it claims. Closing it means `Indirect` writing that field itself.",
    ),
    (
        "crates/kayfabe-linux-raw/src/sandbox_unsafe.rs",
        "★★ FAILS OPEN, and it is a SECURITY INSTRUMENT: `last_capability` breaks out of its \
         probe loop on the first error and returns the last success, so if PR_CAPBSET_READ \
         fails at capability 0 it reports 0. `privileges()` then scans one capability and \
         reports a nearly-empty bounding set — which is exactly what a caller verifying \
         'privilege was surrendered' wants to see. A broken probe reads as a clean result.",
    ),
    (
        "crates/kayfabe-linux-raw/src/vcpu_unsafe.rs",
        "★ `VcpuExit::Mmio.len` is narrowed from the kernel's u32 with `u8::try_from`, which \
         bounds it to 255 — but KVM's contract is <= 8 and `data` is `[u8; 8]`. A consumer \
         slicing `data[..len]` panics rather than reads out of bounds, so this is a liveness \
         gap, not a soundness one; the completion side IS bounded (see its own test). The \
         bound should come from `data.len()`, which is the value that makes it true.",
    ),
    (
        "crates/kayfabe-linux-raw/src/kvm_unsafe.rs",
        "★ `KvmVm::adopt` takes an `OwnedFd` and builds a VM handle without calling \
         `confirm_is_a_vm`, which exists three hundred lines away and is what the clone path \
         uses. Every ioctl afterwards is issued against whatever that descriptor really is. \
         The failure is EINVAL rather than corruption, but the type says 'this is a VM' and \
         nothing established it.",
    ),
    (
        "crates/kayfabe-linux-raw/src/spawn_unsafe.rs",
        "`FdGrant::new` asserts `target >= 3` and has no upper bound, so a grant can name a \
         descriptor number the child's RLIMIT_NOFILE will refuse. The dup2 fails and the spawn \
         reports it, so this is tidiness rather than a hole — recorded so the asymmetry (a \
         lower bound that panics, no upper bound at all) is a decision rather than an oversight.",
    ),
];

/// ⚠ The audit that produced the rows above raised nine findings. Two were fixed, five are
/// listed, and **two are neither** — `Backing::SharedFile` accepting a file shorter than the
/// mapping (a `SIGBUS` on first touch, no `fstat`) and `adopt_inherited_fd` treating a
/// descriptor's openness as evidence of ownership. ⊘ Both are **UNMEASURED by me** — the
/// wording above is the audit's, carried verbatim and adjudicated by nobody. A row I
/// transcribed rather than read would make the five I did read less believable, so they stay
/// out of the list and stay named here: the gap is a gap, not a shorter list.
const UNADJUDICATED_AUDIT_FINDINGS: usize = 2;

/// ★ The audit's findings that are **still open**: five listed above plus two unread.
///
/// ⊘ This is the ratchet, and it points DOWN. Deleting a row from
/// [`UNDISCHARGED_REDERIVATION`] because the gap was closed must decrement this too, and the
/// test that checks the sum is what forces the commit to say *which* finding closed. A residue
/// list nobody has to reconcile shrinks by attrition — a row gets tidied away in an unrelated
/// diff and the count of known-open holes silently drops.
const AUDIT_FINDINGS_STILL_OPEN: usize = 7;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// Every `.rs` path under `<crate>/src`, repo-relative. Paths only — this gate never reads a
/// file's text, which is how it stays clear of the shell gate's pattern.
fn sources() -> BTreeSet<String> {
    let root = repo_root();
    let mut out = BTreeSet::new();
    for c in CRATES {
        walk(&root.join("crates").join(c).join("src"), &root, &mut out);
    }
    out
}

fn walk(dir: &Path, root: &Path, out: &mut BTreeSet<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, root, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.insert(
                p.strip_prefix(root)
                    .expect("under the repo root")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
}

// =====================================================================================

/// ★★★ Every declared soundness-critical safe file exists and carries the suffix.
///
/// ⚠ The list is empty today, so this test's *body* is vacuous — and that is stated rather
/// than hidden. What it buys is that **adding a name is a deliberate edit that must also
/// rename the file**: `gates_quantified_over_a_list`, where the failure mode is a list quietly
/// getting shorter. The companion test below is what stops the emptiness itself from rotting.
#[test]
fn every_declared_soundness_critical_safe_file_carries_the_unsafe_suffix() {
    let present = sources();
    for f in SOUNDNESS_CRITICAL_SAFE_FILES {
        assert!(
            present.contains(*f),
            "⊘ `{f}` is declared soundness-critical but does not exist — a phantom row makes \
             this gate quantify over a file nobody can read"
        );
        assert!(
            f.ends_with("_unsafe.rs"),
            "★★★ `{f}` is declared to carry reasoning a raw-surface block depends on, so it \
             owes the `_unsafe` suffix even though rustc compiled it without the keyword. ⊘ \
             Before renaming, try the better fix: make the raw side RE-VALIDATE, which removes \
             the row instead of decorating it"
        );
    }
}

/// ★★★ Every undischarged row names a file that exists, and says what is trusted.
///
/// ⊘ This cannot tell whether the gap is still open — only a reader can. What it stops is the
/// row outliving the file, which is how a residue list turns into decoration: five confident
/// paragraphs about code nobody can open.
#[test]
fn every_undischarged_rederivation_names_a_live_file_and_states_what_is_trusted() {
    let present = sources();
    for (file, why) in UNDISCHARGED_REDERIVATION {
        assert!(
            present.contains(*file),
            "⊘ `{file}` carries an undischarged-trust row but does not exist. Either the gap \
             was closed and the row should be DELETED (which is the point of the list), or the \
             path rotted and every other row just got less believable"
        );
        assert!(
            why.len() > 120,
            "★ the row for `{file}` is too short to say what value is trusted, where it comes \
             from, and what closing it would mean — and those three are the whole content"
        );
    }
    assert!(
        !UNDISCHARGED_REDERIVATION.is_empty(),
        "★★★ if this list is empty then the crate-wide claim the audit REFUTED has become true \
         again — which is excellent, and is a claim that must be re-established by reading, not \
         by an empty array. Restore the sentence on SOUNDNESS_CRITICAL_SAFE_FILES when it is"
    );
}

/// ★★ The books balance: every audit finding is fixed, listed, or named as unread.
///
/// ⊘ The debt is UNMEASURED and this test cannot measure it — what it does is make the debt
/// **unavoidable to edit**. Closing a gap deletes a row, which breaks this sum, which forces
/// the same commit to say which finding closed and decrement the total. That is the only
/// mechanism here; without it a row is a comment, and comments get tidied.
#[test]
fn the_open_findings_account_for_every_row_and_every_unread_site() {
    assert_eq!(
        UNDISCHARGED_REDERIVATION.len() + UNADJUDICATED_AUDIT_FINDINGS,
        AUDIT_FINDINGS_STILL_OPEN,
        "★ the residue no longer adds up. If a gap was CLOSED, decrement \
         AUDIT_FINDINGS_STILL_OPEN in the same commit and say in the message which one and how \
         it was verified. If a row was ADDED from somewhere other than the 2026-08-07 audit, \
         give it its own provenance — inheriting a citation it does not come from is exactly \
         the move that put a refuted claim on master in the first place"
    );
}

/// ★★ The scan still finds the tree, so an empty list means *"nothing qualifies"* and not
/// *"the walker broke"*.
///
/// ⊘ Without this, deleting `CRATES` or breaking `walk` would leave the test above green while
/// it examined nothing — the same false green as a filter that matches no test name and still
/// reports `ok`.
#[test]
fn the_scan_still_sees_the_raw_crates_so_an_empty_list_means_nothing_qualifies() {
    let present = sources();
    assert!(
        present.len() > 20,
        "the walker found only {} sources under {CRATES:?} — it is broken, and a broken walker \
         makes the declared list vacuously satisfiable",
        present.len()
    );
    let named = present.iter().filter(|p| p.ends_with("_unsafe.rs")).count();
    assert!(
        named >= 10,
        "only {named} `*_unsafe.rs` files found — the raw surface cannot have shrunk that far, \
         so this is the instrument breaking rather than the tree improving"
    );
}

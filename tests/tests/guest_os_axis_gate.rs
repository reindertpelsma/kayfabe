//! ★★ **The fourth axis's vocabulary gate** — `four_axes_of_variation.md` §5 rule 3:
//! *"No OS assumption without a comment naming it, so the future Windows seam is a grep
//! away."*
//!
//! ## Why this file exists
//!
//! That rule was written on 2026-07-27 and **nothing enforced it**. Three of the four
//! axes have a real gate in `ci.yml`; this one had a sentence in a design doc. On
//! 2026-07-29 the seam audit found the one violation the sentence was supposed to
//! prevent, sitting in the highest-consequence function in the crate — a
//! `processID == KERNEL_PID` test whose sentinel the guest driver gates on
//! `RMCFG_FEATURE_PLATFORM_UNIX`, so that on a Windows guest the WDDM kernel's RM clients
//! classified as *user* clients and became merge-eligible. That is the state Axis B's
//! rule was in before it got a gate, and Axis A's before today.
//!
//! ## Why a Rust test and not a `ci.yml` step
//!
//! Two reasons, and only the first is scheduling. `.github/workflows/ci.yml` is being
//! edited concurrently (the `unsafe_code` ratchet is moving), and a second writer to that file
//! is a merge conflict at best. The durable reason is the one `gate_scope.rs` established
//! on 2026-07-29: a gate expressed as a test can assert its own **non-vacuity** — it can
//! run its checker against a synthetic violation and require it to fire — and a `grep`
//! in YAML cannot. This project has now shipped **six** gates that could never fire, one
//! of them inside the `unsafe_code` ratchet itself. A green gate that has never been seen red is
//! decoration.
//!
//! ## What it checks
//!
//! 1. [`the_logic_crates_carry_no_unnamed_guest_os_assumption`] — the rule itself:
//!    OS-shaped vocabulary in **code** (not prose) in a guest-facing logic crate must have
//!    a comment naming the OS within reach.
//! 2. [`the_gate_bites_every_token_it_claims_to_cover`] — the same checker, run against
//!    synthetic violations, one per token. This is the "seen to fail" evidence, in-tree
//!    and permanent.
//! 3. [`every_crate_is_scoped_or_exempted_by_this_gate`] — a crate added tomorrow is
//!    classified tomorrow, not silently uncovered. (`gate_scope.rs`'s lesson: two crates
//!    landed on 2026-07-29 that no seam gate could see.)
//! 4. [`the_kernel_pid_sentinel_lives_in_exactly_one_file`] — containment. The sentinel
//!    is the one OS-conditional constant in the tree, and a second copy of it is a second
//!    place the OS gate can be forgotten.
//!
//! ## What it deliberately does NOT check
//!
//! Whether the naming comment is *correct*. It cannot: a comment is prose. What it buys
//! is that the assumption is **written down at the site**, so the Windows seam is a grep
//! away — which is exactly, and only, what the rule asks for.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("the workspace root is one level above the tests crate")
}

// ---------------------------------------------------------------------------------
// The scope
// ---------------------------------------------------------------------------------

/// The crates this gate is scoped to: everything that reasons about the **guest**.
///
/// ★ The dividing line is `four_axes_of_variation.md` §1.1, and it is not the same line
/// the hexagonal-boundary gate draws. That gate asks *"may this crate name an OS
/// readiness primitive?"*; this one asks *"may this crate assume which OS wrote the bytes
/// it is reading?"* — and §1.1 answers the host half of that question outright:
/// *"host-side OS-specificity may be written **assuming Linux, freely and without
/// apology**"*, naming `kayfabe-linux-raw` as the crate that exists for it. So a host
/// adapter is exempt here on a principled ground and not merely because it is an adapter.
const SCOPED: &[&str] = &[
    "kayfabe-abi",
    "kayfabe-arch",
    // ★ SCOPED, not exempt. `kayfabe-chips` is the arch-impl crate: it may name a GPU
    // generation (that is its job, and the Generation-name gate exempts it for exactly
    // that) but a *silicon* model must never assume a guest OS. Axis B and Axis D are
    // independent, and §4.5 records what it cost the last time one field was conditioned
    // on two axes and the code named neither. Exempting it would have made a new
    // architecture the quiet way past this gate.
    "kayfabe-chips",
    "kayfabe-completion",
    "kayfabe-core",
    "kayfabe-fwd",
    "kayfabe-gsp",
    "kayfabe-isolate",
    "kayfabe-mmu",
    "kayfabe-rmrpc",
    "kayfabe-trace",
    "kayfabe-util",
    "kayfabe-vmm",
];

/// Every other crate, with the reason it is out of scope **on the line**. "It is an
/// adapter" is a reason; silence is not.
const EXEMPT: &[(&str, &str)] = &[
    (
        "kayfabe-crec",
        "Axis-B arch adapter + C-trace replay; test-facing, reads captures from a Linux bench",
    ),
    (
        "kayfabe-isolate-host",
        "HOST adapter — §1.1: host-side OS-specificity may assume Linux freely",
    ),
    (
        "kayfabe-linux-raw",
        "the audited raw-Linux crate; §1.1 names it as the one that needs no abstraction",
    ),
    ("kayfabe-mocks", "test-only doubles"),
    ("kayfabe-qemu-raw", "HOST adapter — the QEMU C ABI"),
    ("kayfabe-rt", "HOST runtime adapter (executor/threads)"),
    (
        "kayfabe-shell",
        "IS the L1 host OS shell — naming Linux primitives is its job",
    ),
    ("kayfabe-vmm-kvm", "HOST adapter — KVM is a Linux interface"),
    ("kayfabe-vmm-qemu", "HOST adapter"),
];

// ---------------------------------------------------------------------------------
// The checker
// ---------------------------------------------------------------------------------

/// A token that only makes sense if you already believe the guest is Linux.
///
/// `word` = require identifier boundaries on both sides (so `ITSELF` is not an `ELF`
/// assumption and `version fork` is not `fork(2)`); path-shaped tokens match anywhere.
struct Token {
    text: &'static str,
    word: bool,
}

const fn word(text: &'static str) -> Token {
    Token { text, word: true }
}
const fn anywhere(text: &'static str) -> Token {
    Token { text, word: false }
}

/// ★ The list is short on purpose. Every entry is something that, appearing in *code* in
/// a guest-facing crate, is a claim about the guest's OS — not merely a word that occurs
/// in Linux. A broad list would be met by broad exemptions, which is how a gate becomes
/// decoration.
const TOKENS: &[Token] = &[
    // The Linux process/VM model, as the guest kernel would express it.
    word("mm_struct"),
    word("task_struct"),
    word("vm_area_struct"),
    word("fork"),
    word("execve"),
    word("getpid"),
    word("pid_t"),
    // ★★★ The one OS-conditional NVIDIA constant in the tree — the token this whole gate
    // was written for. `FOUNDING_TOKEN` pins it: removing it from this list is how the
    // gate would go quietly green over the very defect that motivated it, and that is not
    // hypothetical either — it happened once while this file was being written, by an
    // edit that meant to remove the line below it.
    word("KERNEL_PID"),
    //
    // ★ `RMCFG_FEATURE_PLATFORM_UNIX` — the driver flag that GATES that constant — was in
    // this list and was **removed on 2026-07-29 by the bite test below**, which is the
    // best argument for the bite test that exists. The token contains `UNIX`, which is
    // itself a [`NAMING`] word, so a line naming it always names the OS and the entry
    // could never produce a violation. It was a list entry that covered nothing, in a
    // gate written specifically because this project keeps shipping gates that cover
    // nothing. [`no_token_can_name_its_own_way_out`] now makes that shape impossible.
    //
    // Filesystem shape.
    anywhere("/proc/"),
    anywhere("/sys/"),
    anywhere("/dev/"),
    word("PATH_MAX"),
    word("ELF"),
    // Host-page-size assumptions, which are an OS question and not a GPU one. (GPU page
    // sizes are Axis B and are deliberately NOT here: `4096` in an MMU crate is silicon.)
    word("getpagesize"),
    word("sysconf"),
    word("PAGE_SHIFT"),
];

/// Words that make an OS assumption *named*. Matched case-insensitively, in a comment or
/// anywhere else on the surrounding lines.
const NAMING: &[&str] = &[
    "linux",
    "unix",
    "posix",
    "windows",
    "wddm",
    "guestos",
    "guest os",
    "guest-os",
    "os assumption",
    "tegra",
];

/// How far back a naming comment may sit. An item's doc comment is the natural home for
/// it, and doc comments here run long.
const WINDOW: usize = 40;

/// ★★ The token the gate was written for, pinned so it cannot leave [`TOKENS`] silently.
///
/// A gate's scope list is the part of it nobody re-reads. On 2026-07-29, while this file
/// was being written, an edit aimed at a *neighbouring* entry deleted this one — and all
/// five tests stayed green, because every one of them is quantified over `TOKENS` and a
/// shorter list is simply a smaller true statement. That is the same shape as the
/// `-maxdepth 1` in the `unsafe_code` ratchet: not a broken check, a check that stopped
/// looking.
const FOUNDING_TOKEN: &str = "KERNEL_PID";

#[derive(Debug, PartialEq, Eq)]
struct Violation {
    file: String,
    line: usize,
    token: &'static str,
    text: String,
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn contains_token(haystack: &str, t: &Token) -> bool {
    let mut from = 0;
    while let Some(i) = haystack[from..].find(t.text) {
        let at = from + i;
        let end = at + t.text.len();
        if !t.word {
            return true;
        }
        let before_ok = haystack[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident_char(c));
        let after_ok = haystack[end..]
            .chars()
            .next()
            .is_none_or(|c| !is_ident_char(c));
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

/// Is this line prose rather than code? Prose is not an *assumption* — the rule is about
/// what the code does — and excluding it is what keeps the naming comments themselves,
/// and every `[src]` citation into the driver, from tripping the gate they satisfy.
fn is_comment_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") || t.starts_with("/*") || t.starts_with('*')
}

/// The gate, as a pure function of one file's text, so it can be pointed at a synthetic
/// violation as easily as at the tree.
fn scan(rel_path: &str, text: &str) -> Vec<Violation> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if is_comment_line(line) {
            continue;
        }
        for t in TOKENS {
            if !contains_token(line, t) {
                continue;
            }
            let from = i.saturating_sub(WINDOW);
            let named = lines[from..=i].iter().any(|l| {
                let lower = l.to_lowercase();
                NAMING.iter().any(|n| lower.contains(n))
            });
            if !named {
                out.push(Violation {
                    file: rel_path.to_string(),
                    line: i + 1,
                    token: t.text,
                    text: (*line).trim().to_string(),
                });
            }
        }
    }
    out
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.filter_map(Result::ok) {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Every `.rs` file under the scoped crates' `src/`, as (repo-relative path, text).
fn scoped_sources() -> Vec<(String, String)> {
    let root = repo_root();
    let mut out = Vec::new();
    for c in SCOPED {
        let dir = root.join("crates").join(c).join("src");
        assert!(
            dir.is_dir(),
            "scoped crate {c} has no src/ — the list is stale"
        );
        let mut files = Vec::new();
        rs_files(&dir, &mut files);
        files.sort();
        for f in files {
            let rel = f
                .strip_prefix(&root)
                .expect("under the root")
                .to_string_lossy()
                .into_owned();
            out.push((rel, std::fs::read_to_string(&f).expect("readable")));
        }
    }
    out
}

// ---------------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------------

/// ★★★ **`four_axes_of_variation.md` §5 rule 3, enforced.**
///
/// The tree is expected to be green: as of 2026-07-29 the only OS-shaped code in the
/// scoped crates is the `KERNEL_PID` sentinel, and it now lives in
/// `kayfabe-abi/src/guest_os.rs` under a doc comment that names the flag it is gated on.
/// That is exactly why [`the_gate_bites_every_token_it_claims_to_cover`] exists beside it:
/// a gate whose only evidence is that it passes is a gate nobody has seen work.
#[test]
fn the_logic_crates_carry_no_unnamed_guest_os_assumption() {
    let sources = scoped_sources();
    assert!(
        sources.len() >= 20,
        "★ NON-VACUITY: only {} files scanned — the walk is broken and the assertion \
         below would be about an empty set",
        sources.len(),
    );
    assert!(!TOKENS.is_empty() && !NAMING.is_empty());
    assert!(
        TOKENS.iter().any(|t| t.text == FOUNDING_TOKEN),
        "★ NON-VACUITY: {FOUNDING_TOKEN} is not in TOKENS, so this gate is green over \
         exactly the defect it was written to catch. Every test in this file is \
         quantified over TOKENS and a shorter list is a smaller true statement — which \
         is why the founding token is pinned rather than trusted",
    );
    assert!(
        TOKENS.len() >= 12,
        "★ NON-VACUITY: the token list has shrunk to {} — a gate list that quietly \
         empties is a gate that quietly stops",
        TOKENS.len(),
    );

    let mut violations = Vec::new();
    for (rel, text) in &sources {
        violations.extend(scan(rel, text));
    }

    assert!(
        violations.is_empty(),
        "★ AN OS ASSUMPTION WITH NO COMMENT NAMING IT — \
         `four_axes_of_variation.md` §5 rule 3.\n\n{}\n\n\
         The guest OS is the fourth axis, and it is UNDETECTABLE ON THE WIRE: it is a\n\
         build flag in the guest driver, not a protocol field. So a line that assumes it\n\
         cannot be checked by anything at runtime — the only defence is that it says so.\n\n\
         Two ways out, and picking one is a design decision:\n\
         - the assumption is REAL -> put it behind `kayfabe_abi::GuestOs`, where the arm\n\
           for an OS we have no rule for is a typed refusal;\n\
         - the assumption is INCIDENTAL -> say so in a comment naming the OS, within {} \
           lines above the line.\n\n\
         This gate exists because on 2026-07-29 the answer was neither: \
         `client_kind_from_process_id` applied a UNIX-only rule to every guest and folded \
         the guest kernel's RM clients in with a guest process's.",
        violations
            .iter()
            .map(|v| format!("  {}:{}  [{}]  {}", v.file, v.line, v.token, v.text))
            .collect::<Vec<_>>()
            .join("\n"),
        WINDOW,
    );
}

/// ★★★ **THE GATE IS SEEN TO FAIL — once per token it claims to cover.**
///
/// For every entry in [`TOKENS`], a synthetic code line using it with no naming comment
/// must produce **exactly one** violation, and the same line with a naming comment above
/// it must produce **none**. Both halves are needed: the first proves the token is
/// detected, the second proves the escape hatch works and therefore that a green tree is
/// green because it is annotated rather than because the checker is inert.
///
/// ★ This is not decoration for its own sake. The five-axis audit found a ratchet whose
/// `-maxdepth 1` and `src/*` prune between them hid any amount of `unsafe_code`, and it had
/// been
/// green for weeks. A per-token bite test is the cheapest thing that would have caught it.
#[test]
fn the_gate_bites_every_token_it_claims_to_cover() {
    for t in TOKENS {
        let bare = format!("fn f() {{ let x = \"{}\"; }}", t.text);
        let hits = scan("synthetic.rs", &bare);
        assert_eq!(
            hits.len(),
            1,
            "★ the gate does not detect {:?} at all — it is in the list and covers \
             nothing ({hits:?})",
            t.text,
        );
        assert_eq!(hits[0].token, t.text);
        assert_eq!(hits[0].line, 1);

        let named = format!("// This is a Linux-guest-only shape.\n{bare}");
        assert!(
            scan("synthetic.rs", &named).is_empty(),
            "★ naming the OS must clear {:?}, or the rule is unfollowable and every site \
             will end up exempted",
            t.text,
        );
    }

    // The window is a real bound, not a "somewhere in the file".
    let far = format!(
        "// Linux.\n{}fn f() {{ let x = mm_struct; }}",
        "\n".repeat(WINDOW + 2)
    );
    assert_eq!(
        scan("synthetic.rs", &far).len(),
        1,
        "★ a naming comment {WINDOW}+ lines away must NOT clear a hit, or the escape \
         hatch is 'the word Linux appears somewhere in this file'",
    );

    // Prose is not an assumption — asserted, because if it were, every `[src]` citation
    // into the driver would trip the gate and the tree would be exempted into silence.
    assert!(scan("synthetic.rs", "// KERNEL_PID is a sentinel.").is_empty());
    assert!(scan("synthetic.rs", "/// see /proc/self/maps").is_empty());
    assert!(scan("synthetic.rs", " * mm_struct").is_empty());

    // Word boundaries do real work: these are the false positives a bare `grep` produced
    // when this gate was prototyped against the tree.
    assert!(scan("synthetic.rs", "let x = ITSELF;").is_empty(), "ELF");
    assert!(scan("synthetic.rs", "let forked = 1;").is_empty(), "fork");
    assert!(scan("synthetic.rs", "let KERNEL_PIDS = 1;").is_empty());
}

/// ★★★ **No token may contain a word that clears it.**
///
/// A [`TOKENS`] entry that contains a [`NAMING`] word is unenforceable: any line matching
/// it also names the OS, so it can never be a violation. That is not a hypothetical —
/// `RMCFG_FEATURE_PLATFORM_UNIX` was in the list for exactly as long as it took the bite
/// test to run, and it would otherwise have sat there indefinitely as a list entry that
/// covered nothing.
///
/// Checked separately from the bite test so the failure *says which shape is wrong*: the
/// bite test would report "the gate does not detect X", which is true but sends the
/// reader looking for a bug in `contains_token`.
#[test]
fn no_token_can_name_its_own_way_out() {
    for t in TOKENS {
        let lower = t.text.to_lowercase();
        for n in NAMING {
            assert!(
                !lower.contains(n),
                "★ token {:?} contains the naming word {n:?}, so every line that matches                  it also clears it. The entry covers NOTHING. Either the token is                  redundant (mentioning it already names the OS) or the naming word is too                  broad",
                t.text,
            );
        }
    }
}

/// ★★ **Every crate is either scoped by this gate or exempted with a reason.**
///
/// `gate_scope.rs` measured what happens without this on 2026-07-29: two crates were
/// added and the three vocabulary gates could not see either, silently. A gate that
/// enumerates today's crates stops covering the code the moment someone adds one, so the
/// enumeration is checked against the filesystem rather than trusted.
#[test]
fn every_crate_is_scoped_or_exempted_by_this_gate() {
    let root = repo_root();
    let mut crates: Vec<String> = std::fs::read_dir(root.join("crates"))
        .expect("crates/ exists")
        .filter_map(|e| {
            let e = e.expect("readable dir entry");
            let name = e.file_name().to_string_lossy().into_owned();
            e.path().join("Cargo.toml").is_file().then_some(name)
        })
        .collect();
    crates.sort();
    assert!(
        crates.len() >= 20,
        "★ NON-VACUITY: only {} crates discovered — the scan is broken ({crates:?})",
        crates.len(),
    );

    let mut unclassified = Vec::new();
    for c in &crates {
        let scoped = SCOPED.contains(&c.as_str());
        let exempt = EXEMPT.iter().any(|(x, _)| x == c);
        match (scoped, exempt) {
            (true, true) => unclassified.push(format!("  {c}: BOTH scoped and exempted")),
            (false, false) => unclassified.push(format!("  {c}: neither scoped nor exempted")),
            _ => {}
        }
    }
    assert!(
        unclassified.is_empty(),
        "★ A CRATE IS INVISIBLE TO THE GUEST-OS GATE.\n{}\n\n\
         Each crate must be one of two things, and saying which is a design decision:\n\
         - SCOPED -> add it to SCOPED in this file (it reasons about the GUEST)\n\
         - EXEMPT -> add it to EXEMPT, WITH the reason on the line. \
         `four_axes_of_variation.md` §1.1 grants host-side crates an outright exemption \
         (*\"host-side OS-specificity may be written assuming Linux, freely and without \
         apology\"*); guest-side crates get none.",
        unclassified.join("\n"),
    );
}

/// ★★ **Containment: the `KERNEL_PID` sentinel exists in exactly one source file.**
///
/// It is the only OS-conditional NVIDIA constant in the tree, and the defect this whole
/// milestone closes was that the *rule* using it lived away from any statement of which
/// OS it was true of. One home means one place to change when a second OS lands; a second
/// copy would be a second place to forget.
///
/// Test wire-builders are excluded by construction — this walks `crates/*/src` only —
/// because a fixture that *writes* the sentinel onto the wire is modelling a guest, not
/// assuming one.
#[test]
fn the_kernel_pid_sentinel_lives_in_exactly_one_file() {
    let root = repo_root();
    let mut files = Vec::new();
    let mut crates: Vec<_> = std::fs::read_dir(root.join("crates"))
        .expect("crates/ exists")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    crates.sort();
    for c in crates {
        rs_files(&c.join("src"), &mut files);
    }
    assert!(
        files.len() >= 40,
        "★ NON-VACUITY: only {} files",
        files.len()
    );

    let sentinel = word("KERNEL_PID");
    let homes: BTreeSet<String> = files
        .iter()
        .filter(|f| {
            std::fs::read_to_string(f)
                .expect("readable")
                .lines()
                .any(|l| !is_comment_line(l) && contains_token(l, &sentinel))
        })
        .map(|f| {
            f.strip_prefix(&root)
                .expect("under the root")
                .to_string_lossy()
                .into_owned()
        })
        .collect();

    assert_eq!(
        homes,
        ["crates/kayfabe-abi/src/guest_os.rs".to_string()]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        "★ the OS-gated sentinel must have exactly one home. A new one is a new place \
         where 'which OS is this true of?' can go unasked",
    );
}

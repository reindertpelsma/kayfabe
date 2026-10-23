//! ★ A guest-reachable emit is bounded (`CLAUDE.md`, *Hostile guest*). The TSG mirror refuses a channel birth by
//! name, and a hostile guest can ask for a refused birth in a loop (a legacy channel in an explicit TSG, a refused flag
//! bit): every `TSG-MIRROR REFUSED` line the plane prints must pass the mirror's own log budget
//! (`tsgmirror::refused_line_ok`), as the BIRTH and CTX lines do (`LOG_MAX` per process). The counters still count every
//! refusal; only the text is capped. Two checks, no GPU needed.

/// Every non-comment `eprintln!` that prints `TSG-MIRROR REFUSED` sits behind `refused_line_ok` (looked for in the five
/// lines above the format string, where the `if` or the `&&` guard of the statement is).
#[test]
fn every_mirror_refusal_line_passes_the_log_budget() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut seen = 0;
    for entry in std::fs::read_dir(src).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            if l.trim_start().starts_with("//") || !l.contains("TSG-MIRROR REFUSED") {
                continue;
            }
            seen += 1;
            let from = i.saturating_sub(5);
            let guarded = lines[from..i].iter().any(|g| g.contains("refused_line_ok"));
            assert!(
                guarded,
                "{}:{}: a guest-reachable refusal line without the mirror log budget (tsgmirror::refused_line_ok)",
                p.display(),
                i + 1
            );
        }
    }
    assert!(
        seen >= 1,
        "the refusal line moved: this guard must follow it, not pass vacuously"
    );
}

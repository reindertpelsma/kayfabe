//! `cargo run -p kayfabe-crec --example cap1_report` — the differential, in full, as a
//! report a human reads.
//!
//! The assertions live in `tests/cap1_differential.rs`; this prints what they assert, plus
//! the parts that are evidence rather than a pass/fail (the answer census, the
//! reconstructions, the planes this differential does not cover).

use kayfabe_crec::{Fill, Note, Replay, census, load_cap1};

fn main() {
    let trace = load_cap1()
        .expect("cap1 is committed")
        .expect("cap1 decodes");
    println!("== capture ==");
    println!("records      : {}", trace.records().len());
    println!("hermetic     : {}", trace.header().hermetic());
    println!("rom overlay  : {}", trace.header().rom_overlay());
    println!("census       : {:?}", trace.census());

    let abi = kayfabe_crec::bench_abi();
    for fill in [Fill::Observed, Fill::Lookahead, Fill::Reconstructed] {
        let res = Replay::new(&trace, abi).run(fill);
        println!("\n================ Fill::{fill:?} ================");
        println!("transactions projected : {}", res.txns.len());
        println!("unprojected            : {:?}", res.unprojected);
        println!("answers                : {:?}", res.answers);
        println!("unobserved reads       : {}", res.unobserved.len());
        for (t, u) in res.unobserved.iter().take(5) {
            println!("    txn {t} gpa=0x{:x} len={}", u.gpa, u.len);
        }
        println!("max lookahead (records): {}", res.max_lookahead);
        println!("closure limit (txn)    : {:?}", res.closure_limit);
        println!("reconstructions        : {:?}", res.reconstructions);
        println!("final phase            : {:?}", res.final_phase);
        println!("transitions seen       : {:?}", res.transitions_seen);
        println!("C projection census    : {:?}", res.c.census());
        println!("Rust projection census : {:?}", res.rust.census());

        let global = kayfabe_trace::diff(&res.c.events, &res.rust.events);
        match &global {
            None => println!("GLOBAL diff            : IDENTICAL"),
            Some(d) => println!(
                "GLOBAL diff            : first divergence at position {} (txn {:?})",
                d.at,
                res.c.txn.get(d.at)
            ),
        }

        let cen = census(&res);
        println!("divergences            : {} total", cen.items.len());
        println!("  beyond closure limit : {}", cen.beyond_closure());
        println!("by ledger id (in reach): {:?}", cen.by_id());
        let un = cen.unexplained();
        println!("UNEXPLAINED (in reach) : {}", un.len());
        for (shown, it) in un.iter().enumerate() {
            if shown >= 12 {
                println!("    … and {} more", un.len() - shown);
                break;
            }
            println!(
                "    txn {:5} reg {:?} at {}  C={}  RUST={}",
                it.txn,
                it.reg,
                it.at,
                brief(it.c.as_ref()),
                brief(it.rust.as_ref())
            );
        }
        let refused: Vec<_> = res
            .txns
            .iter()
            .enumerate()
            .filter_map(|(i, t)| t.refusal.map(|f| (i, t.reg, f)))
            .collect();
        println!("FSM refusals           : {}", refused.len());
        for (i, reg, f) in refused.iter().take(4) {
            println!("    txn {i} reg {reg:?} -> {f:?}");
        }
        if refused.len() > 4 {
            println!("    … and {} more", refused.len() - 4);
        }
        if let Some((first, _, _)) = refused.first() {
            let t = &res.txns[*first];
            println!("    first refusing txn {first}: reads =");
            for (g, l, a) in &res.read_log[t.reads.0..t.reads.1] {
                println!("        gpa=0x{g:x} len={l} -> {a:?}");
            }
            let prev = &res.txns[first.saturating_sub(1)];
            println!("    previous txn reads =");
            for (g, l, a) in &res.read_log[prev.reads.0..prev.reads.1] {
                println!("        gpa=0x{g:x} len={l} -> {a:?}");
            }
        }
    }
}

fn brief(n: Option<&Note>) -> String {
    match n {
        None => "(nothing)".to_string(),
        Some(Note::Register(r)) => format!("{r:?}"),
        Some(Note::Irq) => "Irq".to_string(),
        Some(Note::Undecoded { gpa, len }) => format!("Undecoded(gpa=0x{gpa:x},len={len})"),
        Some(Note::Decoded(o)) => format!("{o:?}"),
    }
}

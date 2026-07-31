//! `cargo run -p kayfabe-crec --example cap1b_report` — the same report as `cap1_report`,
//! over `cap1b` (GSP-D6 witnessed) and driven by the policy chain a **guest** is answered
//! by rather than by the C-baseline echo.
//!
//! The assertions live in `tests/cap1b_differential.rs`; this prints what they assert plus
//! the evidence that is not a pass/fail.

use kayfabe_crec::{Fill, Note, Replay, census, load_cap1b};
use kayfabe_gsp::Observation;

fn main() {
    let trace = load_cap1b()
        .expect("cap1b is committed")
        .expect("cap1b decodes");
    println!("== capture ==");
    println!("records      : {}", trace.records().len());
    println!("hermetic     : {}", trace.header().hermetic());
    println!("census       : {:?}", trace.census());

    let abi = kayfabe_crec::bench_abi();
    for served in [false, true] {
        let mut r = Replay::new(&trace, abi);
        if served {
            r = r.with_policy(kayfabe_crec::served_policy);
        }
        let res = r.run(Fill::Reconstructed);
        println!("\n================ served={served} ================");
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

        // Every command the guest sent, the control it names, and whether the element we
        // posted for it is byte-equal to the one the C posted.
        let diverged: std::collections::BTreeSet<usize> = census(&res)
            .items
            .iter()
            .filter(|i| matches!(i.c, Some(Note::Decoded(Observation::ElementPosted { .. }))))
            .map(|i| i.txn)
            .collect();
        println!("commands decoded       : {}", res.commands.len());
        for (txn, cmd) in res.commands.iter().take(64) {
            let ctrl = if cmd.code == 76 && cmd.payload.len() >= 12 {
                format!(
                    "cmd 0x{:08x}",
                    u32::from_le_bytes(cmd.payload[8..12].try_into().unwrap())
                )
            } else {
                String::new()
            };
            println!(
                "    txn {txn:5} fn {:5} seq {:5} paylen {:6} elems {} {ctrl}  reply {}",
                cmd.code,
                cmd.sequence,
                cmd.payload.len(),
                cmd.elements,
                if diverged.contains(txn) {
                    "DIFFERS"
                } else {
                    "matches-C"
                }
            );
        }

        // And the commands the C's own stream carried, so the two can be lined up.
        let cen = census(&res);
        println!("divergences            : {} total", cen.items.len());
        println!("  beyond closure limit : {}", cen.beyond_closure());
        println!("by ledger id (in reach): {:?}", cen.by_id());
        let un = cen.unexplained();
        println!("UNEXPLAINED (in reach) : {}", un.len());
        for (shown, it) in un.iter().enumerate() {
            if shown >= 24 {
                println!("    … and {} more", un.len() - shown);
                break;
            }
            println!(
                "    txn {:5} at {}  C={}  RUST={}",
                it.txn,
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

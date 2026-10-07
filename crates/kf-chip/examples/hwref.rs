//! Print how ogkm hardware names resolve per die group (`kf_chip::hwref`), as a Markdown table.
//!
//! ```text
//! cargo run -p kf-chip --example hwref -- NV_PGSP_QUEUE_HEAD(0) NV_PRISCV_RISCV_IRQMASK
//! cargo run -p kf-chip --example hwref -- --differs      # every name whose value differs by die group
//! ```
//! A cell is the value, marked `·` when inherited from an ancestor directory, `⊘amb` when the
//! nearest tier disagrees (needs a pin), `—` when no directory of the lineage defines it.

use kf_chip::hwref::{DieGroup, HwValue, Resolved, table};

fn show(v: HwValue) -> String {
    match v {
        HwValue::Val(x) => format!("{x:#x}"),
        HwValue::Range { hi, lo } if hi < 4096 => format!("{hi}:{lo}"),
        HwValue::Range { hi, lo } => format!("{hi:#x}:{lo:#x}"),
        HwValue::MultiWord { hi, lo } => format!("mw {hi}:{lo}"),
        HwValue::Offset(o) => format!("off {o:#x}"),
        HwValue::Size(s) => format!("size {s:#x}"),
        HwValue::Absent => "absent".into(),
    }
}

fn cell(r: &Resolved) -> String {
    match r {
        Resolved::Own { value, .. } => show(*value),
        Resolved::Inherited { value, .. } => format!("{}·", show(*value)),
        Resolved::Pinned { value, .. } => format!("{}📌", show(*value)),
        Resolved::Ambiguous { .. } => "⊘amb".into(),
        Resolved::Absent => "—".into(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let t = table();
    let names: Vec<String> = if args.first().map(String::as_str) == Some("--differs") {
        let mut all: Vec<String> = kf_chip::hwref::TABLE
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split('\t').nth(2).map(str::to_owned))
            .collect();
        all.sort();
        all.dedup();
        all.into_iter()
            .filter(|n| {
                let v: Vec<_> = DieGroup::ALL
                    .iter()
                    .map(|g| t.resolve(*g, n).value())
                    .collect();
                v.iter().any(|x| x.is_some()) && v.windows(2).any(|w| w[0] != w[1])
            })
            .collect()
    } else {
        args
    };
    println!("| name | TU10x | GA100 | GA10x | AD10x | GH100 | GB10x | GB20x |");
    println!("|---|---|---|---|---|---|---|---|");
    for n in &names {
        let cells: Vec<String> = DieGroup::ALL
            .iter()
            .map(|g| cell(&t.resolve(*g, n)))
            .collect();
        println!("| `{n}` | {} |", cells.join(" | "));
        for g in DieGroup::ALL {
            if let Resolved::Ambiguous { candidates } = t.resolve(g, n) {
                let c: Vec<String> = candidates
                    .iter()
                    .map(|(d, v)| format!("{d}={}", show(*v)))
                    .collect();
                eprintln!("  {n} {g:?} ambiguous: {}", c.join(", "));
            }
        }
    }
}

//! `kf-census`: decode queue captures, run the completeness oracle, write the census.
//!
//! ```text
//! kf-census [--ogkm-tag 580.82.07] [--tier1 docs/design/tier_checks/tier1_ogkm.tsv]
//!           [--out PREFIX] INPUT...
//! ```
//!
//! `INPUT` is an observer `gsp.jsonl`, a `KFGSPCAP` capture, or `-` (stdin; the format is
//! sniffed from the first byte: `{` is the observer's JSON lines). Outputs
//! `PREFIX.controls.tsv`, `PREFIX.functions.tsv`, `PREFIX.anomalies.tsv`, `PREFIX.json`; without
//! `--out`, a summary goes to stdout. Exit status 0 only if the oracle found nothing,
//! 1 if it found anomalies (all of them counted and printed), 2 for a usage or I/O error.

use kf_abi::DriverVersion;
use kf_census::{analyzer_for, capture, observer, report, stream};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage: kf-census [--ogkm-tag MAJOR.MINOR.PATCH] [--tier1 FILE.tsv] [--out PREFIX] INPUT..."
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut tag = DriverVersion {
        major: 580,
        minor: 82,
        patch: 7,
    };
    let mut tier1: Option<String> = None;
    let mut out: Option<String> = None;
    let mut inputs: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--ogkm-tag" => match args.next().as_deref().and_then(DriverVersion::parse) {
                Some(t) => tag = t,
                None => return usage(),
            },
            "--tier1" => tier1 = args.next(),
            "--out" => out = args.next(),
            "-h" | "--help" => return usage(),
            _ => inputs.push(a),
        }
    }
    if inputs.is_empty() {
        return usage();
    }
    let tier1_text = match tier1.as_deref().map(std::fs::read_to_string) {
        None => None,
        Some(Ok(t)) => Some(t),
        Some(Err(e)) => {
            eprintln!("kf-census: --tier1: {e}");
            return ExitCode::from(2);
        }
    };
    let mut an = match analyzer_for(tag, tier1_text.as_deref()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("kf-census: {e}");
            return ExitCode::from(2);
        }
    };
    for path in &inputs {
        let raw: Box<dyn Read> = if path == "-" {
            Box::new(std::io::stdin().lock())
        } else {
            match File::open(path) {
                Ok(f) => Box::new(f),
                Err(e) => {
                    eprintln!("kf-census: {path}: {e}");
                    return ExitCode::from(2);
                }
            }
        };
        let mut r = BufReader::with_capacity(1 << 20, raw);
        let first = match r.fill_buf() {
            Ok(b) => b.first().copied(),
            Err(e) => {
                eprintln!("kf-census: {path}: {e}");
                return ExitCode::from(2);
            }
        };
        let res = if first == Some(b'{') {
            observer::read(&mut an, path, r).map_err(|e| e.to_string())
        } else {
            stream::read(&mut an, path, r).map_err(|e: capture::CaptureError| e.to_string())
        };
        if let Err(e) = res {
            eprintln!("kf-census: {path}: {e}");
            return ExitCode::from(2);
        }
    }
    let rep = an.finish();
    if let Some(prefix) = &out {
        let files = [
            ("controls.tsv", report::controls_tsv(&rep)),
            ("functions.tsv", report::functions_tsv(&rep)),
            ("anomalies.tsv", report::anomalies_tsv(&rep)),
            ("json", report::json(&rep)),
        ];
        for (ext, text) in files {
            if let Err(e) = std::fs::write(format!("{prefix}.{ext}"), text) {
                eprintln!("kf-census: {prefix}.{ext}: {e}");
                return ExitCode::from(2);
            }
        }
    }
    println!(
        "kf-census: ogkm {}.{}.{:02}, {} sources, {} cmd + {} status messages, {} controls",
        tag.major,
        tag.minor,
        tag.patch,
        rep.sources.len(),
        rep.messages[0],
        rep.messages[1],
        rep.controls.len()
    );
    if rep.oracle_ok() {
        println!("oracle: PASS (no gap, no parse failure, no trailing bytes, no unknown function)");
    } else {
        println!("oracle: FAIL, {} anomalies", rep.anomaly_total());
        for (k, a) in &rep.anomalies {
            println!(
                "  {:<22} {:>8}  e.g. {}",
                k.name(),
                a.count,
                a.examples.first().map_or("", String::as_str)
            );
        }
        println!(
            "  missing messages (sum of forward gaps): {}",
            rep.missing_messages
        );
    }
    println!(
        "pairing: {} controls with unanswered requests or orphan replies; rpc.sequence non-zero on {} messages",
        rep.unbalanced_controls(),
        rep.rpc_sequence_nonzero
    );
    if rep.oracle_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

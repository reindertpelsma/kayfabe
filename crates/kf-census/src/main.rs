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
//!
//! ```text
//! kf-census scrub [--ogkm-tag 580.82.07] [--no-default-allow] [--allow-screened] [--allow-list FILE] [--allow ID]...
//!                 --out SCRUBBED INPUT
//! ```
//!
//! Writes a default-deny copy of a capture (`scrub.rs`): headers kept, message bodies replaced
//! by a fill pattern unless the control id is allow-listed. The output is created with mode
//! 0600 and is still a capture file (host-only). Exit 0 if the input was clean, 1 if the scrub
//! had to drop or zero something it could not parse (the output is written either way), 2 for
//! a usage or I/O error.

use kf_abi::DriverVersion;
use kf_census::scrub::{self, AllowList, Scrubber};
use kf_census::{analyzer_for, capture, observer, report, stream};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage: kf-census [--ogkm-tag MAJOR.MINOR.PATCH] [--tier1 FILE.tsv] [--out PREFIX] INPUT...\n       kf-census scrub [--ogkm-tag T] [--no-default-allow] [--allow-screened] [--allow-list FILE] [--allow ID]... --out SCRUBBED INPUT"
    );
    ExitCode::from(2)
}

fn open_out(path: &str) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

fn scrub_main(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut tag = DriverVersion {
        major: 580,
        minor: 82,
        patch: 7,
    };
    let mut default_allow = true;
    let mut screened = false;
    let mut list_files: Vec<String> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    let mut out: Option<String> = None;
    let mut input: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--ogkm-tag" => match args.next().as_deref().and_then(DriverVersion::parse) {
                Some(t) => tag = t,
                None => return usage(),
            },
            "--no-default-allow" => default_allow = false,
            "--allow-screened" => screened = true,
            "--allow-list" => match args.next() {
                Some(f) => list_files.push(f),
                None => return usage(),
            },
            "--allow" => match args.next() {
                Some(i) => ids.push(i),
                None => return usage(),
            },
            "--out" => out = args.next(),
            "-h" | "--help" => return usage(),
            _ if input.is_none() => input = Some(a),
            _ => return usage(),
        }
    }
    let (Some(out), Some(input)) = (out, input) else {
        return usage();
    };
    let mut allow = if default_allow {
        AllowList::builtin()
    } else {
        AllowList::empty()
    };
    if screened {
        for id in AllowList::screened().ids() {
            let _ = allow.insert(id);
        }
    }
    for f in &list_files {
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("kf-census scrub: --allow-list {f}: {e}");
                return ExitCode::from(2);
            }
        };
        match AllowList::parse(&text) {
            Ok(l) => {
                for id in l.ids() {
                    let _ = allow.insert(id);
                }
            }
            Err(e) => {
                eprintln!("kf-census scrub: {f}: {e}");
                return ExitCode::from(2);
            }
        }
    }
    for i in &ids {
        match scrub::parse_id(i).ok_or_else(|| format!("not a control id: {i:?}")).and_then(|id| allow.insert(id)) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("kf-census scrub: --allow {i}: {e}");
                return ExitCode::from(2);
            }
        }
    }
    let abi = match kf_rm::abi::gsp_abi_for(tag) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("kf-census scrub: no ABI for {tag:?}: {e:?}");
            return ExitCode::from(2);
        }
    };
    let raw: Box<dyn Read> = if input == "-" {
        Box::new(std::io::stdin().lock())
    } else {
        match File::open(&input) {
            Ok(f) => Box::new(f),
            Err(e) => {
                eprintln!("kf-census scrub: {input}: {e}");
                return ExitCode::from(2);
            }
        }
    };
    let mut r = BufReader::with_capacity(1 << 20, raw);
    let first = match r.fill_buf() {
        Ok(b) => b.first().copied(),
        Err(e) => {
            eprintln!("kf-census scrub: {input}: {e}");
            return ExitCode::from(2);
        }
    };
    let file = match open_out(&out) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("kf-census scrub: {out}: {e}");
            return ExitCode::from(2);
        }
    };
    let mut w = BufWriter::with_capacity(1 << 20, file);
    let mut sc = Scrubber::new(abi, allow.clone());
    let res = if first == Some(b'{') {
        scrub::scrub_observer(&mut sc, r, &mut w).map_err(|e| e.to_string())
    } else {
        scrub::scrub_capture(&mut sc, r, &mut w).map_err(|e| e.to_string())
    };
    if let Err(e) = res.and_then(|()| w.flush().map_err(|e| e.to_string())) {
        eprintln!("kf-census scrub: {input}: {e}");
        return ExitCode::from(2);
    }
    let st = &sc.stats;
    println!(
        "kf-census scrub: {} messages, allow-list of {} control ids; bodies kept {} / scrubbed {} ({} bytes), continuations kept {} / scrubbed {}, other payloads scrubbed {}",
        st.messages,
        allow.len(),
        st.per_cmd.values().map(|c| c.0).sum::<u64>(),
        st.per_cmd.values().map(|c| c.1).sum::<u64>(),
        st.body_bytes_scrubbed,
        st.continuations.0,
        st.continuations.1,
        st.other_scrubbed,
    );
    println!(
        "  undecodable {}, padding zeroed on {} messages, unframed bytes zeroed {}, lines dropped {}, keys dropped {}",
        st.undecodable, st.padding_zeroed, st.bytes_zeroed_unframed, st.lines_dropped, st.keys_dropped
    );
    if let Some(t) = &st.truncated {
        println!("  input ended inside a record ({t}); the rest was not written");
    }
    if st.input_was_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn main() -> ExitCode {
    let mut argv = std::env::args().skip(1).peekable();
    if argv.peek().map(String::as_str) == Some("scrub") {
        argv.next();
        return scrub_main(argv);
    }
    main_census(argv)
}

fn main_census(argv: impl Iterator<Item = String>) -> ExitCode {
    let mut tag = DriverVersion {
        major: 580,
        minor: 82,
        patch: 7,
    };
    let mut tier1: Option<String> = None;
    let mut out: Option<String> = None;
    let mut inputs: Vec<String> = Vec::new();
    let mut args = argv;
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

// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! cargo run -p kf-census --example sequence -- OGKM_TAG INPUT > metadata.tsv
//! Same decoder/oracle for KFGSPCAP and observer JSONL. Output is metadata only.
//! Exit 0 requires a complete parse, both directions and balanced controls.
//! Redirect stdout into an Admin/SYSTEM-only directory on native Windows.

use kf_abi::DriverVersion;
use kf_census::{
    analyzer_for, observer,
    sequence::{MAX_ROWS, Sequence},
    stream,
};
use std::cell::RefCell;
use std::io::{BufRead, BufReader, BufWriter};
use std::process::ExitCode;
use std::rc::Rc;

fn run() -> Result<bool, String> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    if a.len() != 2 {
        return Err("usage: sequence OGKM_TAG INPUT".into());
    }
    let tag = DriverVersion::parse(&a[0]).ok_or("invalid driver tag")?;
    let mut an = analyzer_for(tag, &[])?;
    let out = Sequence::new(*an.abi(), BufWriter::new(std::io::stdout()), MAX_ROWS)
        .map_err(|e| e.to_string())?;
    let out = Rc::new(RefCell::new(out));
    let failure = Rc::new(RefCell::new(None::<String>));
    let (tap_out, tap_failure) = (out.clone(), failure.clone());
    an.set_record_tap(Box::new(move |source, m| {
        if tap_failure.borrow().is_none() {
            if let Err(e) = tap_out.borrow_mut().message(source, m) {
                *tap_failure.borrow_mut() = Some(e.to_string());
            }
        }
    }));
    let mut input = BufReader::new(std::fs::File::open(&a[1]).map_err(|e| e.to_string())?);
    if input.fill_buf().map_err(|e| e.to_string())?.first() == Some(&b'{') {
        observer::read(&mut an, "input", input).map_err(|e| e.to_string())?;
    } else {
        stream::read(&mut an, "input", input).map_err(|e| e.to_string())?;
    }
    let report = an.finish();
    out.borrow_mut().finish().map_err(|e| e.to_string())?;
    if let Some(e) = failure.borrow().as_ref() {
        return Err(e.clone());
    }
    eprintln!(
        "SEQUENCE_ORACLE_OK={} anomalies={} unbalanced_controls={} messages={:?}",
        report.oracle_ok(),
        report.anomaly_total(),
        report.unbalanced_controls(),
        report.messages
    );
    Ok(report.oracle_ok()
        && report.unbalanced_controls() == 0
        && report.messages.iter().all(|&n| n > 0))
}
fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("sequence: {e}");
            ExitCode::from(2)
        }
    }
}

//! Reader for the VFIO GSP observer's `gsp.jsonl` (`tools/vfio-gsp-observer/qemu/gsp-observer.c`,
//! schema `kayfabe-gsp-text/1`).
//!
//! One header line, one line per sampled queue message (`kind":"record"`, with the 64-byte
//! observer record fields and the message bytes as `payload_hex`), one footer line. The
//! `payload_hex` bytes are the whole message starting at the 48-byte element header
//! (32 bytes of CC buffers, `checkSum`, `seqNum`, `elemCount`, then the RPC envelope),
//! trimmed to the declared length rounded up to 8.
//!
//! The reader checks the framing the observer promises (header first, footer last, footer
//! record and byte counts equal to what was read) and reports every deviation as an anomaly.
//! It does **not** verify the footer's sha-256 digests; per-message checksums, sequence
//! continuity and the count/byte equalities are what this tool attests (inferred to be
//! sufficient; the digests remain checkable with `tools/vfio-gsp-observer/decode.py`).

use crate::analyze::{Analyzer, AnomalyKind, In, ObsMeta};
use crate::capture::Dir;
use std::io::{BufRead, Read};

/// Longest line accepted: a 65 536-byte message in hex plus the record fields (the reference
/// decoder, `tools/vfio-gsp-observer/decode.py`, uses 140 000 too).
pub const MAX_LINE: usize = 140_000;

/// A parsed JSON value (only what the observer writes: unsigned integers, strings, booleans,
/// arrays, objects).
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    /// Unsigned integer.
    Int(u64),
    /// String (escapes are not used by the observer and are rejected).
    Str(String),
    /// Boolean.
    Bool(bool),
    /// Array.
    Arr(Vec<Val>),
    /// Object, in order.
    Obj(Vec<(String, Val)>),
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", c as char, self.i))
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.eat(b'"')?;
        let start = self.i;
        while self.i < self.b.len() && self.b[self.i] != b'"' {
            if self.b[self.i] == b'\\' {
                return Err("escape in string".into());
            }
            self.i += 1;
        }
        if self.i >= self.b.len() {
            return Err("unterminated string".into());
        }
        let s = std::str::from_utf8(&self.b[start..self.i])
            .map_err(|_| "non-utf8 string".to_string())?
            .to_string();
        self.i += 1;
        Ok(s)
    }
    fn value(&mut self, depth: u32) -> Result<Val, String> {
        if depth > 8 {
            return Err("nesting too deep".into());
        }
        self.ws();
        match self.b.get(self.i) {
            Some(b'"') => Ok(Val::Str(self.string()?)),
            Some(b'{') => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Val::Obj(v));
                }
                loop {
                    let k = self.string()?;
                    self.eat(b':')?;
                    v.push((k, self.value(depth + 1)?));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Val::Obj(v));
                        }
                        _ => return Err(format!("bad object at byte {}", self.i)),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Val::Arr(v));
                }
                loop {
                    v.push(self.value(depth + 1)?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Val::Arr(v));
                        }
                        _ => return Err(format!("bad array at byte {}", self.i)),
                    }
                }
            }
            Some(b't') if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Ok(Val::Bool(true))
            }
            Some(b'f') if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Ok(Val::Bool(false))
            }
            Some(c) if c.is_ascii_digit() => {
                let start = self.i;
                while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[start..self.i])
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(Val::Int)
                    .ok_or_else(|| "integer out of range".to_string())
            }
            _ => Err(format!("unexpected input at byte {}", self.i)),
        }
    }
}

/// Parse one JSON line holding an object.
///
/// # Errors
///
/// A description of the first problem.
pub fn parse_object(line: &[u8]) -> Result<Vec<(String, Val)>, String> {
    let mut p = P { b: line, i: 0 };
    match p.value(0)? {
        Val::Obj(o) => {
            p.ws();
            if p.i != line.len() {
                return Err("data after the object".into());
            }
            Ok(o)
        }
        _ => Err("not an object".into()),
    }
}

fn get<'a>(o: &'a [(String, Val)], k: &str) -> Option<&'a Val> {
    o.iter().find(|(n, _)| n == k).map(|(_, v)| v)
}

fn int(o: &[(String, Val)], k: &str) -> Result<u64, String> {
    match get(o, k) {
        Some(Val::Int(n)) => Ok(*n),
        _ => Err(format!("missing integer field {k}")),
    }
}

fn int32(o: &[(String, Val)], k: &str) -> Result<u32, String> {
    u32::try_from(int(o, k)?).map_err(|_| format!("field {k} exceeds 32 bits"))
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    b.as_chunks::<2>()
        .0
        .iter()
        .map(|p| Some(nib(p[0])? << 4 | nib(p[1])?))
        .collect()
}

/// Read an observer export from `r` into the analyzer. `label` names the source.
///
/// # Errors
///
/// Only I/O errors; every content problem becomes an anomaly.
pub fn read<R: BufRead>(an: &mut Analyzer, label: &str, mut r: R) -> std::io::Result<()> {
    an.begin_source(label, "observer-jsonl");
    let element_size = an.abi().driver.gsp_element_size_min();
    let mut line = Vec::new();
    let mut lineno = 0u64;
    let mut have_header = false;
    let mut footer: Option<Vec<(String, Val)>> = None;
    let mut records = 0u64;
    let mut source_bytes = 64u64; // the observer's 64-byte file header counts in source_bytes
    loop {
        line.clear();
        // Bounded read: a hostile or corrupt file cannot make one line unbounded.
        let n = (&mut r)
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        lineno += 1;
        if line.len() > MAX_LINE {
            an.anomaly(
                AnomalyKind::BadLine,
                format!(
                    "line {lineno}: longer than {MAX_LINE} bytes (skipped to the next newline)"
                ),
            );
            let mut skip = Vec::new();
            loop {
                skip.clear();
                let k = (&mut r).take(65_536).read_until(b'\n', &mut skip)?;
                if k == 0 || skip.ends_with(b"\n") {
                    break;
                }
            }
            continue;
        }
        let text = line.strip_suffix(b"\n").unwrap_or(&line);
        let obj = match parse_object(text) {
            Ok(o) => o,
            Err(e) => {
                an.anomaly(AnomalyKind::BadLine, format!("line {lineno}: {e}"));
                continue;
            }
        };
        if footer.is_some() {
            an.anomaly(
                AnomalyKind::FooterProblem,
                format!("line {lineno}: data after the footer"),
            );
        }
        let kind = match get(&obj, "kind") {
            Some(Val::Str(s)) => s.as_str(),
            _ => "",
        };
        match kind {
            "header" => {
                if lineno != 1 || have_header {
                    an.anomaly(
                        AnomalyKind::FooterProblem,
                        format!("line {lineno}: header not on line 1"),
                    );
                }
                have_header = true;
                let schema =
                    matches!(get(&obj, "schema"), Some(Val::Str(s)) if s == "kayfabe-gsp-text/1");
                if !schema {
                    an.anomaly(
                        AnomalyKind::FooterProblem,
                        "header schema is not kayfabe-gsp-text/1".to_string(),
                    );
                }
                if matches!(get(&obj, "kf_scrubbed"), Some(Val::Int(1))) {
                    an.mark_scrubbed();
                }
                for k in ["observer", "device"] {
                    if let Some(Val::Str(s)) = get(&obj, k) {
                        an.fact(k, s);
                    }
                }
            }
            "record" => {
                an.record_read();
                records += 1;
                let parsed = (|| -> Result<(In<'_>, Vec<u8>), String> {
                    let hex = match get(&obj, "payload_hex") {
                        Some(Val::Str(s)) => s,
                        _ => return Err("missing payload_hex".into()),
                    };
                    let bytes = unhex(hex).ok_or("payload_hex is not lowercase hex")?;
                    let payload_bytes = int32(&obj, "payload_bytes")?;
                    if bytes.len() != payload_bytes as usize {
                        return Err(format!(
                            "payload_hex has {} bytes, payload_bytes says {payload_bytes}",
                            bytes.len()
                        ));
                    }
                    let dir = match int(&obj, "direction")? {
                        0 => Dir::Cmd,
                        1 => Dir::Status,
                        d => return Err(format!("direction {d}")),
                    };
                    let flags = int32(&obj, "flags")?;
                    let m = In {
                        a: int(&obj, "generation")?,
                        b: int(&obj, "table_pa")?,
                        dir,
                        ordinal: lineno,
                        ts: int(&obj, "qpc")?,
                        bytes: &[],
                        element_size,
                        exact: true,
                        obs: Some(ObsMeta {
                            queue_sequence: int32(&obj, "queue_sequence")?,
                            rpc_sequence: int32(&obj, "rpc_sequence")?,
                            rpc_function: int32(&obj, "rpc_function")?,
                            rpc_result: int32(&obj, "rpc_result")?,
                            rpc_version: int32(&obj, "rpc_version")?,
                            payload_bytes,
                            missing_before: int32(&obj, "missing_before")?,
                            prefix_unknown: flags & 2 != 0,
                        }),
                    };
                    Ok((m, bytes))
                })();
                match parsed {
                    Ok((m, bytes)) => {
                        source_bytes += 64 + bytes.len() as u64;
                        an.message(In { bytes: &bytes, ..m });
                    }
                    Err(e) => an.anomaly(AnomalyKind::BadLine, format!("line {lineno}: {e}")),
                }
            }
            "footer" => footer = Some(obj),
            other => an.anomaly(
                AnomalyKind::BadLine,
                format!("line {lineno}: unknown kind {other:?}"),
            ),
        }
    }
    if !have_header {
        an.anomaly(AnomalyKind::FooterProblem, "no header line".to_string());
    }
    match footer {
        None => an.anomaly(
            AnomalyKind::FooterProblem,
            "no footer: the export was cut off (the observer writes it last)".to_string(),
        ),
        Some(f) => check_footer(an, &f, records, source_bytes),
    }
    Ok(())
}

fn check_footer(an: &mut Analyzer, f: &[(String, Val)], records: u64, source_bytes: u64) {
    match get(f, "file_export_complete") {
        Some(Val::Bool(true)) => {}
        _ => an.anomaly(
            AnomalyKind::FooterProblem,
            "file_export_complete is not true".to_string(),
        ),
    }
    for (k, want) in [("records", records), ("source_bytes", source_bytes)] {
        match get(f, k) {
            Some(Val::Int(n)) if *n == want => an.fact(k, n),
            other => an.anomaly(
                AnomalyKind::FooterProblem,
                format!("footer {k} is {other:?}, the file carried {want}"),
            ),
        }
    }
    if let Some(Val::Obj(stats)) = get(f, "driver_stats") {
        for (k, v) in stats {
            if let (Val::Bool(true), "limited") = (v, k.as_str()) {
                an.fact("stat.limited", true);
                an.anomaly(
                    AnomalyKind::ObserverLoss,
                    "observer driver_stats.limited = true: it stopped recording at its own cap, so the tail of the stream is missing".to_string(),
                );
            }
            if let Val::Int(n) = v {
                an.fact(&format!("stat.{k}"), n);
                if k == "dropped" && *n != 0 {
                    an.anomaly(
                        AnomalyKind::ObserverLoss,
                        format!("observer driver_stats.dropped = {n}"),
                    );
                }
            }
        }
    }
}

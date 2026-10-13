//! Census output: TSV tables and one JSON document. Only counts, sizes, ids and statuses are
//! written - never message bytes (the capture files are host-only; the census may be shared).

use crate::analyze::Report;
use crate::capture::Dir;
use std::collections::BTreeMap;
use std::fmt::Write as _;

fn hist(m: &BTreeMap<u32, u64>, hex: bool) -> String {
    if m.is_empty() {
        return "-".to_string();
    }
    m.iter()
        .map(|(k, n)| {
            if hex {
                format!("{k:#x}:{n}")
            } else {
                format!("{k}:{n}")
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn dir_name(d: Dir) -> &'static str {
    match d {
        Dir::Cmd => "cmd",
        Dir::Status => "status",
    }
}

/// Per-control census as TSV (one header line, one row per control id).
#[must_use]
pub fn controls_tsv(r: &Report) -> String {
    let mut s = String::from(
        "cmd\tname\trequests\treplies\tpaired\tunanswered\torphan_replies\techo\tserialized\treq_sizes\treply_sizes\tstatuses\trpc_results\tdistinct_replies\togkm\togkm_basis\n",
    );
    for c in &r.controls {
        let _ = writeln!(
            s,
            "{:#010x}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            c.cmd,
            c.name.as_deref().unwrap_or("-"),
            c.requests,
            c.replies,
            c.paired,
            c.unanswered,
            c.orphan_replies,
            c.echo,
            c.serialized,
            hist(&c.req_sizes, false),
            hist(&c.rep_sizes, false),
            hist(&c.statuses, true),
            hist(&c.rpc_results, true),
            c.distinct_replies,
            c.ogkm.word(),
            c.ogkm.basis(),
        );
    }
    s
}

/// Per-function census as TSV.
#[must_use]
pub fn functions_tsv(r: &Report) -> String {
    let mut s = String::from("dir\tfunction\tname\tcount\tbytes\trpc_results\n");
    for ((d, f), row) in &r.functions {
        let _ = writeln!(
            s,
            "{}\t{:#x}\t{}\t{}\t{}\t{}",
            dir_name(*d),
            f,
            row.name.as_deref().unwrap_or("UNKNOWN"),
            row.count,
            row.bytes,
            hist(&row.results, true),
        );
    }
    s
}

/// Oracle anomalies as TSV (count and first example per kind).
#[must_use]
pub fn anomalies_tsv(r: &Report) -> String {
    let mut s = String::from("kind\tcount\tfirst_example\n");
    for (k, a) in &r.anomalies {
        let _ = writeln!(
            s,
            "{}\t{}\t{}",
            k.name(),
            a.count,
            a.examples
                .first()
                .map_or("", String::as_str)
                .replace(['\t', '\n'], " ")
        );
    }
    s
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn jhist(m: &BTreeMap<u32, u64>) -> String {
    let items: Vec<String> = m
        .iter()
        .map(|(k, n)| format!("{{\"value\":{k},\"count\":{n}}}"))
        .collect();
    format!("[{}]", items.join(","))
}

/// The whole report as one JSON document (schema `kf-census/1`).
#[must_use]
pub fn json(r: &Report) -> String {
    let mut o = String::new();
    let _ = write!(
        o,
        "{{\"schema\":\"kf-census/1\",\"ogkm_tag\":\"{}.{}.{:02}\",\"oracle_ok\":{},\"anomaly_total\":{},\"messages\":{{\"cmd\":{},\"status\":{}}},\"missing_messages\":{},\"declared_loss_followups\":{},\"nonzero_padding\":{},\"rpc_sequence_nonzero\":{},\"large_joined\":{},\"unbalanced_controls\":{},",
        r.tag.major,
        r.tag.minor,
        r.tag.patch,
        r.oracle_ok(),
        r.anomaly_total(),
        r.messages[0],
        r.messages[1],
        r.missing_messages,
        r.declared_loss_followups,
        r.nonzero_padding,
        r.rpc_sequence_nonzero,
        r.large_joined,
        r.unbalanced_controls(),
    );
    o.push_str("\"sources\":[");
    for (i, s) in r.sources.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        let facts: Vec<String> = s
            .facts
            .iter()
            .map(|(k, v)| format!("{}:{}", esc(k), esc(v)))
            .collect();
        let _ = write!(
            o,
            "{{\"label\":{},\"format\":{},\"records\":{},\"messages\":{},\"facts\":{{{}}}}}",
            esc(&s.label),
            esc(&s.format),
            s.records,
            s.messages,
            facts.join(",")
        );
    }
    o.push_str("],\"anomalies\":{");
    for (i, (k, a)) in r.anomalies.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        let ex: Vec<String> = a.examples.iter().map(|e| esc(e)).collect();
        let _ = write!(
            o,
            "{}:{{\"count\":{},\"examples\":[{}]}}",
            esc(k.name()),
            a.count,
            ex.join(",")
        );
    }
    o.push_str("},\"functions\":[");
    for (i, ((d, f), row)) in r.functions.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        let _ = write!(
            o,
            "{{\"dir\":\"{}\",\"function\":{},\"name\":{},\"count\":{},\"bytes\":{},\"rpc_results\":{}}}",
            dir_name(*d),
            f,
            row.name.as_deref().map_or("null".to_string(), esc),
            row.count,
            row.bytes,
            jhist(&row.results)
        );
    }
    o.push_str("],\"controls\":[");
    for (i, c) in r.controls.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        let _ = write!(
            o,
            "{{\"cmd\":{},\"cmd_hex\":\"{:#010x}\",\"name\":{},\"requests\":{},\"replies\":{},\"paired\":{},\"unanswered\":{},\"orphan_replies\":{},\"echo\":{},\"serialized\":{},\"req_sizes\":{},\"reply_sizes\":{},\"statuses\":{},\"rpc_results\":{},\"distinct_replies\":{},\"ogkm\":\"{}\",\"ogkm_basis\":{}}}",
            c.cmd,
            c.cmd,
            c.name.as_deref().map_or("null".to_string(), esc),
            c.requests,
            c.replies,
            c.paired,
            c.unanswered,
            c.orphan_replies,
            c.echo,
            c.serialized,
            jhist(&c.req_sizes),
            jhist(&c.rep_sizes),
            jhist(&c.statuses),
            jhist(&c.rpc_results),
            c.distinct_replies,
            c.ogkm.word(),
            esc(&c.ogkm.basis()),
        );
    }
    o.push_str("]}\n");
    o
}

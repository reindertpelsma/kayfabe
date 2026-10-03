// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ `wire.rs` against the TEXT of the vendored `proto/nvkvm_broker_proto.h` (nvkvm-pv `368d2db`,
//! verbatim): every value it defines, both directions — a value on one side only fails — and
//! the record layouts, compiled from the header by the system C compiler.

use kf_broker::wire;
use std::collections::BTreeMap;
use std::process::Command;

fn header() -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("proto/nvkvm_broker_proto.h");
    std::fs::read_to_string(p).expect("the vendored header")
}

fn strip_comments(src: &str) -> String {
    let mut out = String::new();
    let mut rest = src;
    while let Some((before, after)) = rest.split_once("/*") {
        out.push_str(before);
        rest = after.split_once("*/").expect("an unterminated comment").1;
    }
    out.push_str(rest);
    out.lines()
        .map(|l| l.split_once("//").map_or(l, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `2u`, `0x20u`, `(1u << 9)`, or another name already defined.
fn eval(v: &str, known: &BTreeMap<String, u64>) -> Option<u64> {
    let v = v
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    if let Some((a, b)) = v.split_once("<<") {
        return Some(eval(a, known)? << eval(b, known)?);
    }
    let lit = v.trim_end_matches(['u', 'U', 'l', 'L']);
    if let Some(hex) = lit.strip_prefix("0x") {
        return u64::from_str_radix(hex, 16).ok();
    }
    lit.parse().ok().or_else(|| known.get(v).copied())
}

/// Every `#define NVKVM_BROKER_* <constant>` and every `NVKVM_BROKER_* = N` enum entry.
fn header_values() -> BTreeMap<String, u64> {
    let text = strip_comments(&header());
    let mut m = BTreeMap::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("#define ") {
            let mut it = rest.splitn(2, char::is_whitespace);
            let (Some(name), Some(val)) = (it.next(), it.next()) else {
                continue;
            };
            if !name.starts_with("NVKVM_BROKER_") || name.contains('(') {
                continue; // the include guard, and function-like macros
            }
            if let Some(v) = eval(val, &m) {
                m.insert(name.to_string(), v);
            }
        } else if let Some((name, val)) = l.trim_end_matches(',').split_once('=')
            && name.trim().starts_with("NVKVM_BROKER_")
        {
            let v = eval(val, &m).expect("an enum value");
            m.insert(name.trim().to_string(), v);
        }
    }
    m
}

#[test]
fn every_protocol_value_matches_the_header_text_both_ways() {
    use wire::*;
    let ours: BTreeMap<&str, u64> = [
        ("NVKVM_BROKER_PROTO_VERSION", u64::from(PROTO_VERSION)),
        ("NVKVM_BROKER_PKT_SIZE", PKT_SIZE as u64),
        ("NVKVM_BROKER_CMD_SIZE", CMD_SIZE as u64),
        ("NVKVM_BROKER_EV_HELLO", u64::from(EV_HELLO)),
        ("NVKVM_BROKER_EV_SURFACE", u64::from(EV_SURFACE)),
        ("NVKVM_BROKER_EV_FRAME", u64::from(EV_FRAME)),
        ("NVKVM_BROKER_EV_RELEASE", u64::from(EV_RELEASE)),
        ("NVKVM_BROKER_EV_KEY", u64::from(EV_KEY)),
        ("NVKVM_BROKER_EV_BTN", u64::from(EV_BTN)),
        ("NVKVM_BROKER_EV_ABS", u64::from(EV_ABS)),
        ("NVKVM_BROKER_EV_REL", u64::from(EV_REL)),
        ("NVKVM_BROKER_EV_WHEEL", u64::from(EV_WHEEL)),
        ("NVKVM_BROKER_EV_GRAB", u64::from(EV_GRAB)),
        ("NVKVM_BROKER_EV_FOCUS", u64::from(EV_FOCUS)),
        ("NVKVM_BROKER_EV_POINTER", u64::from(EV_POINTER)),
        ("NVKVM_BROKER_EV_BYE", u64::from(EV_BYE)),
        ("NVKVM_BROKER_EV_CLOSE", u64::from(EV_CLOSE)),
        ("NVKVM_BROKER_EV_CLIPBOARD", u64::from(EV_CLIPBOARD)),
        ("NVKVM_BROKER_EV_FORMAT", u64::from(EV_FORMAT)),
        ("NVKVM_BROKER_CLOSE_POWERDOWN", CLOSE_POWERDOWN as u64),
        ("NVKVM_BROKER_CLOSE_FORCE", CLOSE_FORCE as u64),
        ("NVKVM_BROKER_CAP_KEYBOARD", u64::from(CAP_KEYBOARD)),
        ("NVKVM_BROKER_CAP_ABS_POINTER", u64::from(CAP_ABS_POINTER)),
        ("NVKVM_BROKER_CAP_REL_POINTER", u64::from(CAP_REL_POINTER)),
        ("NVKVM_BROKER_CAP_POINTER_LOCK", u64::from(CAP_POINTER_LOCK)),
        ("NVKVM_BROKER_CAP_TOTAL_GRAB", u64::from(CAP_TOTAL_GRAB)),
        ("NVKVM_BROKER_CAP_FOCUS_EVENTS", u64::from(CAP_FOCUS_EVENTS)),
        ("NVKVM_BROKER_CAP_FULLSCREEN", u64::from(CAP_FULLSCREEN)),
        ("NVKVM_BROKER_CAP_DMABUF", u64::from(CAP_DMABUF)),
        ("NVKVM_BROKER_CAP_MODIFIERS", u64::from(CAP_MODIFIERS)),
        ("NVKVM_BROKER_CAP_RELEASE", u64::from(CAP_RELEASE)),
        ("NVKVM_BROKER_BYE_SHUTDOWN", BYE_SHUTDOWN as u64),
        ("NVKVM_BROKER_BYE_DISPLAY_LOST", BYE_DISPLAY_LOST as u64),
        ("NVKVM_BROKER_BYE_PROTOCOL", BYE_PROTOCOL as u64),
        ("NVKVM_BROKER_F_GRABBED", u64::from(F_GRABBED)),
        ("NVKVM_BROKER_F_FOCUSED", u64::from(F_FOCUSED)),
        ("NVKVM_BROKER_F_FULLSCREEN", u64::from(F_FULLSCREEN)),
        ("NVKVM_BROKER_CMD_ATTACH", u64::from(CMD_ATTACH)),
        ("NVKVM_BROKER_CMD_COMMIT", u64::from(CMD_COMMIT)),
        ("NVKVM_BROKER_CMD_WINDOW", u64::from(CMD_WINDOW)),
        ("NVKVM_BROKER_CMD_CLIPBOARD", u64::from(CMD_CLIPBOARD)),
        ("NVKVM_BROKER_CMD_CAPS", u64::from(CMD_CAPS)),
        ("NVKVM_BROKER_CMD_QUERY_FORMAT", u64::from(CMD_QUERY_FORMAT)),
        ("NVKVM_BROKER_CMD_F_SHM", u64::from(CMD_F_SHM)),
        ("NVKVM_BROKER_CMD_F_ALL", u64::from(CMD_F_SHM)),
        ("NVKVM_BROKER_CLIENT_CLIPBOARD", u64::from(CLIENT_CLIPBOARD)),
        ("NVKVM_BROKER_MAX_DIM", u64::from(MAX_DIM)),
    ]
    .into_iter()
    .collect();
    // ⊘ Deliberately NOT mirrored: the clipboard framing (this relay offers no clipboard —
    // CAPS bit 0 is clear — so it neither sends nor parses chunks).
    let not_mirrored = |n: &str| n.starts_with("NVKVM_BROKER_CLIP_");
    let theirs = header_values();
    assert!(
        theirs.len() >= 50,
        "the header census found too little ({}): {theirs:?}",
        theirs.len()
    );
    for (name, v) in &theirs {
        if not_mirrored(name) {
            continue;
        }
        assert_eq!(
            ours.get(name.as_str()),
            Some(v),
            "{name} = {v:#x} in the header: wire.rs must carry it with that value"
        );
    }
    for (name, v) in &ours {
        assert_eq!(
            theirs.get(*name),
            Some(v),
            "wire.rs carries {name} = {v:#x}, which the header does not define so"
        );
    }
}

#[test]
fn the_header_is_vendored_verbatim_with_its_own_licence() {
    let h = header();
    assert!(h.starts_with("/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */"));
    assert!(h.contains("#define NVKVM_BROKER_PROTO_VERSION 2u"));
}

/// ★ The record layouts, from the compiler: every field's offset as the header declares it, and
/// the Rust encoder placing each field at exactly that offset.
#[test]
fn the_record_layouts_match_the_compiled_header() {
    let dir = std::env::temp_dir().join(format!(
        "kfb-proto-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("layout.c");
    let bin = dir.join("layout");
    let pkt = ["type", "flags", "seq", "x", "y", "w0", "w1"];
    let cmd = [
        "type",
        "flags",
        "width",
        "height",
        "stride",
        "offset",
        "fourcc",
        "modifier",
        "seq",
        "reserved1",
    ];
    let mut prog = String::from(
        "#include <stdio.h>\n#include <stddef.h>\n#include \"nvkvm_broker_proto.h\"\nint main(void) {\n",
    );
    prog.push_str("printf(\"pkt %zu\\n\", sizeof(struct nvkvm_broker_pkt));\n");
    prog.push_str("printf(\"cmd %zu\\n\", sizeof(struct nvkvm_broker_cmd));\n");
    for f in pkt {
        prog.push_str(&format!(
            "printf(\"pkt.{f} %zu\\n\", offsetof(struct nvkvm_broker_pkt, {f}));\n"
        ));
    }
    for f in cmd {
        prog.push_str(&format!(
            "printf(\"cmd.{f} %zu\\n\", offsetof(struct nvkvm_broker_cmd, {f}));\n"
        ));
    }
    prog.push_str("return 0; }\n");
    std::fs::write(&src, prog).unwrap();
    let build = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Werror", "-I"])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("proto"))
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("a C compiler (cc)");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&bin).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let got: BTreeMap<String, usize> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| {
            let (k, v) = l.split_once(' ').unwrap();
            (k.to_string(), v.parse().unwrap())
        })
        .collect();
    let want: BTreeMap<String, usize> = [
        ("pkt", wire::PKT_SIZE),
        ("cmd", wire::CMD_SIZE),
        ("pkt.type", 0),
        ("pkt.flags", 2),
        ("pkt.seq", 4),
        ("pkt.x", 8),
        ("pkt.y", 12),
        ("pkt.w0", 16),
        ("pkt.w1", 20),
        ("cmd.type", 0),
        ("cmd.flags", 2),
        ("cmd.width", 4),
        ("cmd.height", 8),
        ("cmd.stride", 12),
        ("cmd.offset", 16),
        ("cmd.fourcc", 20),
        ("cmd.modifier", 24),
        ("cmd.seq", 32),
        ("cmd.reserved1", 36),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    assert_eq!(got, want, "the C layout");
    // and the encoder writes each field there: one distinct byte per field
    let c = wire::Cmd {
        ty: 0x0101,
        flags: 0x0202,
        width: 0x0303_0303,
        height: 0x0404_0404,
        stride: 0x0505_0505,
        offset: 0x0606_0606,
        fourcc: 0x0707_0707,
        modifier: 0x0808_0808_0808_0808,
        seq: 0x0909_0909,
    }
    .encode();
    for (i, f) in cmd.iter().enumerate().take(9) {
        let at = want[&format!("cmd.{f}")];
        assert_eq!(c[at], i as u8 + 1, "cmd.{f} at {at}");
    }
    let p = wire::Pkt {
        ty: 0x0101,
        flags: 0x0202,
        seq: 0x0303_0303,
        x: 0x0404_0404,
        y: 0x0505_0505,
        w0: 0x0606_0606,
        w1: 0x0707_0707,
    }
    .encode();
    for (i, f) in pkt.iter().enumerate() {
        let at = want[&format!("pkt.{f}")];
        assert_eq!(p[at], i as u8 + 1, "pkt.{f} at {at}");
    }
}

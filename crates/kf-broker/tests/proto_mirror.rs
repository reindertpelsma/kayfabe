// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ `wire.rs` against the TEXT of the broker's own header, `broker/common/nvkvm_broker_proto.h` —
//! the ONE copy of the protocol (the crate's vendored `proto/` copy, nvkvm-pv `368d2db`, 175 lines
//! behind it, is gone): every value it defines, both directions — a value on one side only fails —
//! and the record layouts (the command record and the cursor record), compiled from the header by
//! the system C compiler.

use kf_broker::wire;
use std::collections::BTreeMap;
use std::process::Command;

/// The directory of the broker's header (`broker/common`, two levels up from the crate).
fn header_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../broker/common")
}

fn header() -> String {
    std::fs::read_to_string(header_dir().join("nvkvm_broker_proto.h")).expect("the broker's header")
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
    if let Some((a, b)) = v.split_once('*') {
        return eval(a, known)?.checked_mul(eval(b, known)?);
    }
    let lit = v.trim_end_matches(['u', 'U', 'l', 'L']);
    if let Some(hex) = lit.strip_prefix("0x") {
        return u64::from_str_radix(hex, 16).ok();
    }
    lit.parse().ok().or_else(|| known.get(v).copied())
}

/// Every `#define NVKVM_BROKER_* <constant>` and every `NVKVM_BROKER_* = N` enum entry of the
/// broker's header.
fn header_values() -> BTreeMap<String, u64> {
    values_of(&header())
}

/// The same census over any header text.
fn values_of(src: &str) -> BTreeMap<String, u64> {
    let text = strip_comments(src);
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
            // ⊘ an identifier, not an expression: the newer header's `_Static_assert(... *
            // NVKVM_BROKER_CURSOR_MAX_DIM <= 0x7fffffffull, ...)` is no enum entry (found
            // 2026-10-04, the first run of the check below against the COMMITTED `badf2d7` header)
            && name
                .trim()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
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
        ("NVKVM_BROKER_EV_DEVICE", u64::from(EV_DEVICE)),
        ("NVKVM_BROKER_DEVICE_F_KNOWN", DEVICE_F_KNOWN as u64),
        ("NVKVM_BROKER_DEVICE_F_RENDER", DEVICE_F_RENDER as u64),
        ("NVKVM_BROKER_CAP_CURSOR", u64::from(CAP_CURSOR)),
        ("NVKVM_BROKER_CAP_DEVICE", u64::from(CAP_DEVICE)),
        ("NVKVM_BROKER_CAP_CURSOR_STREAM", u64::from(CAP_CURSOR_STREAM)),
        ("NVKVM_BROKER_CURSOR_MOVE", u64::from(CURSOR_MOVE)),
        ("NVKVM_BROKER_CURSOR_F_ABS", u64::from(CURSOR_F_ABS)),
        ("NVKVM_BROKER_CURSOR_MOVE_LIMIT", CURSOR_MOVE_LIMIT as u64),
        ("NVKVM_BROKER_CMD_CURSOR", u64::from(CMD_CURSOR)),
        ("NVKVM_BROKER_CURSOR_SET", u64::from(CURSOR_SET)),
        ("NVKVM_BROKER_CURSOR_HIDE", u64::from(CURSOR_HIDE)),
        ("NVKVM_BROKER_CURSOR_SHOW", u64::from(CURSOR_SHOW)),
        ("NVKVM_BROKER_CURSOR_F_XOR", u64::from(CURSOR_F_XOR)),
        ("NVKVM_BROKER_CURSOR_MAX_DIM", u64::from(CURSOR_MAX_DIM)),
        ("NVKVM_BROKER_CURSOR_MAX_STRIDE", u64::from(CURSOR_MAX_STRIDE)),
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
        // phase 2 (docs/design/V3_DISPLAY_PACING_20261017.md section 11.2)
        ("NVKVM_BROKER_CAP_TICK", u64::from(CAP_TICK)),
        ("NVKVM_BROKER_CLIENT_TICK", u64::from(CLIENT_TICK)),
        ("NVKVM_BROKER_EV_TICK", u64::from(EV_TICK)),
        ("NVKVM_BROKER_CMD_NO_CHANGE", u64::from(CMD_NO_CHANGE)),
        ("NVKVM_BROKER_TICK_REFRESH_MASK", u64::from(TICK_REFRESH_MASK)),
        ("NVKVM_BROKER_TICK_REFRESH_MIN_NS", u64::from(TICK_REFRESH_MIN_NS)),
        ("NVKVM_BROKER_TICK_F_WANT_FRAME", u64::from(TICK_F_WANT_FRAME)),
        ("NVKVM_BROKER_TICK_F_CLOCK_ONLY", u64::from(TICK_F_CLOCK_ONLY)),
        ("NVKVM_BROKER_TICK_F_RESERVED", u64::from(TICK_F_RESERVED)),
        ("NVKVM_BROKER_TICK_REQ_TIMEOUT_NS", u64::from(TICK_REQ_TIMEOUT_NS)),
        ("NVKVM_BROKER_GUEST_MAX_HZ", u64::from(GUEST_MAX_HZ)),
        ("NVKVM_BROKER_GUEST_MIN_HZ", u64::from(GUEST_MIN_HZ)),
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

/// ★ The cursor record's layout, from the compiler: every field's offset as the header declares it,
/// and `CursorCmd::encode` writing each field there (flags included: `CURSOR_F_XOR` rides in it).
#[test]
fn the_cursor_record_layout_matches_the_compiled_header() {
    let dir = std::env::temp_dir().join(format!("kfb-proto-cursor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(header_dir().join("nvkvm_broker_proto.h"), dir.join("nvkvm_broker_proto.h")).unwrap();
    let fields = [
        "type",
        "flags",
        "width",
        "height",
        "stride",
        "offset",
        "fourcc",
        "hot_x",
        "hot_y",
        "op",
        "reserved1",
    ];
    let mut prog = String::from(
        "#include <stdio.h>\n#include <stddef.h>\n#include \"nvkvm_broker_proto.h\"\nint main(void) {\n",
    );
    prog.push_str("printf(\"size %zu\\n\", sizeof(struct nvkvm_broker_cursor_cmd));\n");
    for f in fields {
        prog.push_str(&format!(
            "printf(\"{f} %zu\\n\", offsetof(struct nvkvm_broker_cursor_cmd, {f}));\n"
        ));
    }
    prog.push_str("return 0; }\n");
    std::fs::write(dir.join("c.c"), prog).unwrap();
    let out = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Werror", "-o"])
        .arg(dir.join("c"))
        .arg(dir.join("c.c"))
        .arg("-I")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(dir.join("c")).output().unwrap();
    let got: BTreeMap<String, usize> = String::from_utf8(run.stdout)
        .unwrap()
        .lines()
        .map(|l| {
            let (k, v) = l.split_once(' ').unwrap();
            (k.to_string(), v.parse().unwrap())
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got["size"], wire::CMD_SIZE);
    // plant a distinct value per field and find it where the compiled header says it lives
    let c = wire::CursorCmd {
        flags: 0x0a0a,
        width: 0x1111_1111,
        height: 0x2222_2222,
        stride: 0x3333_3333,
        offset: 0x4444_4444,
        fourcc: 0x5555_5555,
        hot_x: 0x6666_6666,
        hot_y: 0x7777_7777,
        op: 0x0808_0808,
    };
    let b = c.encode();
    assert_eq!(&b[got["flags"]..got["flags"] + 2], &[0x0a, 0x0a], "flags");
    for (f, v) in [
        ("width", 0x11u8),
        ("height", 0x22),
        ("stride", 0x33),
        ("offset", 0x44),
        ("fourcc", 0x55),
        ("hot_x", 0x66),
        ("hot_y", 0x77),
        ("op", 0x08),
    ] {
        let at = got[f];
        assert_eq!(b[at..at + 4], [v; 4], "{f} at +{at}");
    }
    assert_eq!(
        &b[got["type"]..got["type"] + 2],
        &wire::CMD_CURSOR.to_le_bytes()
    );
    assert_eq!(&b[got["reserved1"]..got["reserved1"] + 4], &[0; 4]);
}

/// ★ The XOR flag as a command: the encoder writes `CURSOR_F_XOR` at the header's `flags`, a plain SET
/// has none, and HIDE/SHOW never carry one.
#[test]
fn the_xor_flag_is_in_flags_on_a_set_only() {
    let set = wire::CursorCmd::set(32, 32, 128, 0, (3, 4), 4096).unwrap();
    assert_eq!(set.flags, 0);
    assert_eq!(&set.encode()[2..4], &[0, 0]);
    let x = set.xor();
    assert_eq!(x.flags, wire::CURSOR_F_XOR);
    assert_eq!(&x.encode()[2..4], &wire::CURSOR_F_XOR.to_le_bytes());
    assert_eq!(wire::CursorCmd::hide().flags, 0);
    assert_eq!(wire::CursorCmd::show().flags, 0);
    // the rest of the record is the SET's
    let (a, b) = (set.encode(), x.encode());
    assert_eq!(a[..2], b[..2]);
    assert_eq!(a[4..], b[4..]);
}

#[test]
fn the_header_is_the_brokers_own_with_its_licence() {
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
        .arg(header_dir())
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

/// ★ Phase 2: the `EV_TICK` overlay's layout from the compiler — every field at the offset
/// `Pkt::tick_pkt` writes it (the doc's table: tick_seq@8, refresh word@12, present_ns@16).
#[test]
fn the_tick_event_layout_matches_the_compiled_header() {
    let dir = std::env::temp_dir().join(format!("kfb-proto-tick-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(header_dir().join("nvkvm_broker_proto.h"), dir.join("nvkvm_broker_proto.h")).unwrap();
    let fields = ["type", "flags", "seq", "tick_seq", "refresh_word", "present_ns"];
    let mut prog = String::from(
        "#include <stdio.h>\n#include <stddef.h>\n#include \"nvkvm_broker_proto.h\"\nint main(void) {\n",
    );
    prog.push_str("printf(\"size %zu\\n\", sizeof(struct nvkvm_broker_tick_pkt));\n");
    for f in fields {
        prog.push_str(&format!(
            "printf(\"{f} %zu\\n\", offsetof(struct nvkvm_broker_tick_pkt, {f}));\n"
        ));
    }
    prog.push_str("return 0; }\n");
    std::fs::write(dir.join("c.c"), prog).unwrap();
    let out = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Werror", "-o"])
        .arg(dir.join("c"))
        .arg(dir.join("c.c"))
        .arg("-I")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let run = Command::new(dir.join("c")).output().unwrap();
    let got: BTreeMap<String, usize> = String::from_utf8(run.stdout)
        .unwrap()
        .lines()
        .map(|l| {
            let (k, v) = l.split_once(' ').unwrap();
            (k.to_string(), v.parse().unwrap())
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got["size"], wire::PKT_SIZE);
    let t = wire::TickPkt {
        tick_seq: 0x1111_1111,
        refresh_ns: 0x0222_2222,
        want: true,
        clock_only: false,
        present_ns: 0x4444_4444_3333_3333,
    };
    let b = wire::Pkt::tick_pkt(0x5555_5555, 0x0707, &t).encode();
    let at = |name: &str, len: usize| &b[got[name]..got[name] + len];
    assert_eq!(at("type", 2), &wire::EV_TICK.to_le_bytes());
    assert_eq!(at("flags", 2), &0x0707u16.to_le_bytes());
    assert_eq!(at("seq", 4), &0x5555_5555u32.to_le_bytes());
    assert_eq!(at("tick_seq", 4), &0x1111_1111u32.to_le_bytes());
    assert_eq!(
        at("refresh_word", 4),
        &(0x0222_2222u32 | wire::TICK_F_WANT_FRAME).to_le_bytes()
    );
    assert_eq!(at("present_ns", 8), &0x4444_4444_3333_3333u64.to_le_bytes());
}

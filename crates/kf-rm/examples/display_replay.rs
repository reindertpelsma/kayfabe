//! ★ GPU-free diagnostic (2026-10-08, `traces/display_reply_diff_20261008/`): replay display
//! requests through kf's display link and print kf's reply BODIES — the side of a reply diff that
//! no kf3 trace carries (`KF3_RPC_TRACE` logs only the status).
//!
//! Input (one request per line, whitespace-separated, `#` comments):
//!   `ctrl  N CMD CLIENT OBJECT PARAMS_HEX`     a `GSP_RM_CONTROL`
//!   `alloc N CLASS CLIENT PARENT HANDLE PARAMS_HEX`   a display-class `GSP_RM_ALLOC` (recorded in the model)
//! Output: `N CMD who status=0x.. REPLY_HEX`, where `who` is the link that would answer it in a kf3
//! boot with the runs' flags: `probe` (`KF3_DISPLAY_CTRL_PROBE`'s echo), `private`
//! (`KF3_DISPLAY_PRIVATE_PROBE`), `model` (the display link),
//! `gss` (the `0x20808159` identity), or `none` (no display link claims it: the unserviced ledger's
//! `NV_ERR_NOT_SUPPORTED`). The environment's `KF3_DISPLAY_*` flags apply as in a boot.
//!
//! Usage: `cargo run -p kf-rm --example display_replay -- 580.65.06 < requests.txt`
//! Input lines are data from a capture; every field is parsed, never executed.

use std::io::BufRead;

fn hex(s: &str) -> Option<Vec<u8>> {
    if s == "-" {
        return Some(Vec::new());
    }
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn num(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}

fn main() {
    let ver = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "580.65.06".into());
    let v: Vec<u16> = ver.split('.').filter_map(|x| x.parse().ok()).collect();
    let [major, minor, patch] = v[..] else {
        eprintln!("usage: display_replay MAJOR.MINOR.PATCH < requests");
        std::process::exit(2);
    };
    let driver = *kf_abi::versions::table_for(kf_abi::DriverVersion {
        major,
        minor,
        patch,
    })
    .expect("a measured driver version");
    let mut link = kf_rm::display::DisplayPolicy::new(driver, &kf_chip::display::ADA)
        .offering_display_sw(false);
    let probe = kf_rm::display_ctrl_probe::enabled();
    let private = kf_rm::display_ctrl_probe::private_enabled();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["alloc", n, class, client, parent, handle, params] => {
                let (Some(class), Some(client), Some(parent), Some(handle), Some(p)) = (
                    num(class),
                    num(client),
                    num(parent),
                    num(handle),
                    hex(params),
                ) else {
                    println!("{n} bad-line");
                    continue;
                };
                let ok = link.model().is_some_and(|m| {
                    m.lock()
                        .is_ok_and(|mut g| g.alloc(client, parent, handle, class, &p))
                });
                println!("{n} alloc:{class:#06x} recorded={ok}");
            }
            ["ctrl", n, cmd, client, obj, params] => {
                let (Some(cmd), Some(client), Some(obj), Some(p)) =
                    (num(cmd), num(client), num(obj), hex(params))
                else {
                    println!("{n} bad-line");
                    continue;
                };
                let echoed = kf_rm::display_ctrl_probe::ECHOED
                    .iter()
                    .any(|&(c, s)| c == cmd && s == p.len());
                let private_reply = private
                    .then(|| kf_rm::display_ctrl_probe::private_answer(cmd, &p))
                    .flatten();
                let (who, r) = if probe && echoed {
                    ("probe", Ok(p.clone()))
                } else if let Some(out) = private_reply {
                    ("private", Ok(out))
                } else if link.claims(cmd) {
                    (
                        "model",
                        link.answer_on(client, obj, cmd, &p).unwrap_or(Err(0x56)),
                    )
                } else if cmd == kf_abi::gsslegacy::GSS_LEGACY_0X8159
                    && p.len() == kf_abi::gsslegacy::GSS_LEGACY_0X8159_PARAMS_SIZE
                {
                    ("gss", Ok(p.clone()))
                } else {
                    ("none", Err(0x56))
                };
                match r {
                    Ok(b) => {
                        let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
                        println!("{n} {cmd:#010x} {who} status=0x0 {h}");
                    }
                    Err(st) => println!("{n} {cmd:#010x} {who} status={st:#x} -"),
                }
            }
            _ => {}
        }
    }
}

//! Test helper: build and drive the fn-1 negotiation oracle (`tests/oracle/fn1_negotiation_oracle.c`),
//! NVIDIA's own guest-side `RmRpcSetGuestSystemInfo` compiled from the ogkm 595.84 tree.
//!
//! Included by `#[path]` from both `kf-abi/tests/fn1_negotiation_oracle.rs` and
//! `kf-rm/tests/fn1_negotiation_oracle.rs` (the policy under test lives in `kf-rm`, which `kf-abi`
//! cannot depend on). Nothing here decides anything: it builds the program and speaks its line
//! protocol.
//!
//! # The skip is loud
//!
//! Tree absent -> [`oracle`] returns `None` and prints `FN1-ORACLE-GATE: SKIPPED <test> — <reason>`
//! (and `RAN` otherwise), so a CI floor can count both. A tree that is present but does not build, or
//! whose function moved, panics: that is a rotted oracle, not a machine without one.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Where the pinned ogkm tree is expected; `KAYFABE_OGKM_595` relocates it.
pub const DEFAULT_OGKM_595: &str = "/workspace/ogkm-tars/open-gpu-kernel-modules-595.84";

/// `sizeof(rpc_set_guest_system_info_v03_00)`; the oracle reports its own in `consts` and the tests
/// compare.
pub const BODY_LEN: usize = 792;

fn ogkm_root() -> PathBuf {
    std::env::var_os("KAYFABE_OGKM_595")
        .map_or_else(|| PathBuf::from(DEFAULT_OGKM_595), PathBuf::from)
}

static BUILT: OnceLock<Result<PathBuf, String>> = OnceLock::new();

/// The built oracle binary, or `None` (after printing the named skip reason).
pub fn oracle(test: &str) -> Option<PathBuf> {
    let built = BUILT.get_or_init(|| build());
    match built {
        Ok(p) => {
            eprintln!("FN1-ORACLE-GATE: RAN {test}");
            Some(p.clone())
        }
        Err(reason) if reason.starts_with("SKIP") => {
            eprintln!("FN1-ORACLE-GATE: SKIPPED {test} — {reason}");
            None
        }
        Err(e) => panic!("the fn-1 oracle is present but did not build: {e}"),
    }
}

fn build() -> Result<PathBuf, String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = manifest.join("../kf-abi/tests/oracle/build_fn1_oracle.sh");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "fn1-oracle-{}",
        manifest.file_name().and_then(|n| n.to_str()).unwrap_or("x")
    ));
    let root = ogkm_root();
    let r = Command::new("bash")
        .arg(&script)
        .arg(&root)
        .arg(&out)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", script.display()))?;
    match r.status.code() {
        Some(0) => Ok(out.join("fn1_negotiation_oracle")),
        Some(77) => Err(String::from_utf8_lossy(&r.stdout).trim().to_owned()),
        _ => Err(format!(
            "{}\n{}",
            String::from_utf8_lossy(&r.stdout),
            String::from_utf8_lossy(&r.stderr)
        )),
    }
}

/// `consts`: the constants the ogkm tree itself defines, as `key -> values`.
pub fn consts(bin: &Path) -> BTreeMap<String, Vec<String>> {
    let out = Command::new(bin).arg("consts").output().expect("run oracle");
    assert!(out.status.success());
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for l in String::from_utf8(out.stdout).unwrap().lines() {
        let (k, v) = l.split_once('=').expect("key=value");
        m.entry(k.to_owned()).or_default().push(v.to_owned());
    }
    m
}

/// `0x2d` -> 45.
pub fn hexnum(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).expect("hex")
}

/// What the real guest code decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    /// `RmRpcSetGuestSystemInfo`'s return value.
    pub status: u32,
    /// How many times the request went out (1 = no retry).
    pub issued: u32,
    /// The pair the guest stored (`rpcVgxVersion`), `None` if it stored none.
    pub agreed: Option<(u32, u32)>,
    /// The `ipVersion` it handed to `rpcSetIpVersion`, if it called it.
    pub ip_version: Option<u32>,
    /// Times the fn-64 tail call was made.
    pub ext_calls: u32,
    /// The "Retrying with the VGX version requested by the host" log was written.
    pub retry_log: bool,
    /// The "host version is too old" log was written.
    pub too_old_log: bool,
}

impl Done {
    /// The guest ended up with a usable handshake.
    pub fn accepted(&self) -> bool {
        self.status == 0 && self.agreed.is_some()
    }

    fn parse(line: &str) -> Done {
        let mut kv = BTreeMap::new();
        for w in line.split_whitespace().skip(1) {
            if let Some((k, v)) = w.split_once('=') {
                kv.insert(k, v);
            }
        }
        let num = |k: &str| {
            let v = kv[k];
            v.strip_prefix("0x")
                .map_or_else(|| v.parse().unwrap(), |h| u32::from_str_radix(h, 16).unwrap())
        };
        let agreed = kv["agreed"].split_once('.').map(|(a, b)| {
            (
                u32::from_str_radix(a, 16).unwrap(),
                u32::from_str_radix(b, 16).unwrap(),
            )
        });
        Done {
            status: num("status"),
            issued: num("issued"),
            agreed: agreed.filter(|p| *p != (0, 0)),
            ip_version: (num("set_ip_version") != 0).then(|| num("ip_version")),
            ext_calls: num("ext_calls"),
            retry_log: num("retry_log") != 0,
            too_old_log: num("too_old_log") != 0,
        }
    }
}

/// One `GRID` row: the guest's decision against a built-in host model.
#[derive(Debug, Clone)]
pub struct GridRow {
    /// The host model's name.
    pub model: String,
    /// The guest's pair.
    pub guest: (u32, u32),
    /// The host's pair.
    pub host: (u32, u32),
    /// The decision.
    pub done: Done,
}

fn pair(s: &str) -> (u32, u32) {
    let (a, b) = s.split_once('.').unwrap();
    (
        u32::from_str_radix(a, 16).unwrap(),
        u32::from_str_radix(b, 16).unwrap(),
    )
}

/// `grid`: every (model, guest pair, host pair) the oracle's pair universe spans.
pub fn grid(bin: &Path) -> Vec<GridRow> {
    let out = Command::new(bin).arg("grid").output().expect("run oracle");
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| {
            let f = |k: &str| {
                l.split_whitespace()
                    .find_map(|w| w.strip_prefix(k))
                    .unwrap()
                    .to_owned()
            };
            GridRow {
                model: f("model="),
                guest: pair(&f("guest=")),
                host: pair(&f("host=")),
                done: Done::parse(l),
            }
        })
        .collect()
}

/// The host's answer to one request: envelope `rpc_result`, `rpc_result_private`, and the body the
/// guest finds in its (shared) message buffer afterwards.
pub struct HostReply {
    /// Envelope `rpc_result`.
    pub rpc_result: u32,
    /// Envelope `rpc_result_private`.
    pub rpc_result_private: u32,
    /// The reply body (the whole struct: it overwrites the guest's buffer).
    pub body: Vec<u8>,
}

/// The identity a guest sends.
pub struct Guest<'a> {
    /// `VGX_MAJOR_VERSION_NUMBER`.
    pub major: u32,
    /// `VGX_MINOR_VERSION_NUMBER`.
    pub minor: u32,
    /// `NV_VERSION_STRING`.
    pub driver: &'a str,
    /// `NV_BUILD_BRANCH_VERSION`.
    pub branch: &'a str,
    /// `NV_BUILD_CHANGELIST_NUM`.
    pub cl: u32,
}

/// Run the real guest against `host`: every request the guest issues is handed to `host` (the
/// request body in, the reply out). Returns the decision and the requests seen.
pub fn drive(
    bin: &Path,
    g: &Guest<'_>,
    mut host: impl FnMut(&[u8]) -> HostReply,
) -> (Done, Vec<Vec<u8>>) {
    let mut child = Command::new(bin)
        .args([
            "drive",
            &g.major.to_string(),
            &g.minor.to_string(),
            g.driver,
            g.branch,
            &g.cl.to_string(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn oracle");
    let mut stdin = child.stdin.take().unwrap();
    let mut rd = BufReader::new(child.stdout.take().unwrap());
    let mut seen = Vec::new();
    let done = loop {
        let mut line = String::new();
        assert!(rd.read_line(&mut line).unwrap() > 0, "oracle closed early");
        if let Some(hex) = line.trim().strip_prefix("REQ ") {
            let req: Vec<u8> = (0..hex.len() / 2)
                .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
                .collect();
            let r = host(&req);
            assert_eq!(r.body.len(), BODY_LEN, "a reply is the whole struct");
            let hex: String = r.body.iter().map(|b| format!("{b:02x}")).collect();
            writeln!(
                stdin,
                "REPLY {} {} {hex}",
                r.rpc_result, r.rpc_result_private
            )
            .unwrap();
            stdin.flush().unwrap();
            seen.push(req);
        } else if line.starts_with("DONE") {
            break Done::parse(&line);
        } else {
            panic!("unexpected oracle line: {line}");
        }
    };
    drop(stdin);
    child.wait().unwrap();
    (done, seen)
}

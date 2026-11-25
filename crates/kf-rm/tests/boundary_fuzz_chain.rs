//! (boundary fuzzing, `docs/design/V3_BOUNDARY_FUZZING.md`): the WHOLE served chain
//! (`kf_rm::served_policy`) against hostile RPC commands, GPU-free.
//!
//! The guest's RM is untrusted (guest root), and most of what it sends on `GSP_RM_CONTROL` and
//! `GSP_RM_ALLOC` is a parameter block an unprivileged guest process chose. So every link of the
//! chain is a decoder of guest bytes. The properties, for every command:
//!
//! - **no panic** (a panic here is a panic on the register drainer: the device dies);
//! - **bounded reply**: the reply body is at most [`REPLY_CAP`] bytes;
//! - **bounded emit**: at most [`EMIT_CAP`] statements reach the channel or memory plane per command
//!   (each is an authored host act; a guest-steered unbounded emit is a security bug);
//! - **bounded time**: no command takes longer than [`SLOW_CAP`];
//! - **determinism**: the same session on a fresh chain gives byte-identical replies.
//!
//! The generator is structure-aware: a session starts with a plausible client/device/subdevice, then
//! draws controls, allocs, frees, dups and other functions with handles from the live pool, ids
//! and classes from a dictionary read out of this crate's and `kf-abi`'s own sources, and parameter
//! blocks whose declared `paramsSize` is honest, off by one, zero, `u32::MAX` or random.
//!
//! Fast run: `cargo test -p kf-rm --test boundary_fuzz_chain`. Campaign (several million commands):
//! `KF_FUZZ_SESSIONS=20000 cargo test --release -p kf-rm --test boundary_fuzz_chain -- --ignored --nocapture`.

#[path = "support/ga106.rs"]
mod ga106;
#[path = "support/rpcwire.rs"]
mod rpcwire;
#[path = "support/windows_fn1.rs"]
mod support;

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kf_gsp::{CommandPolicy, RpcCommand};

/// The largest reply body any link may build (the largest legitimate one, GSP static info and the
/// init-table blobs, is a few tens of KiB).
const REPLY_CAP: usize = 1 << 20;
/// Host statements one guest command may cause.
const EMIT_CAP: usize = 16;
/// Wall time one command may take (debug build, shared CI runner: generous on purpose).
const SLOW_CAP: Duration = Duration::from_secs(2);

/// Deterministic xorshift64* (the repo's pattern: no proptest dependency).
#[derive(Clone)]
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn u32(&mut self) -> u32 {
        self.next() as u32
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len() as u64) as usize]
    }
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

/// Every hex literal in the named source trees: `> 0xffff` are control-id candidates, the rest
/// class candidates. Read at test time from the crates' own `src/` (never ignored), sorted.
fn dictionary() -> (Vec<u32>, Vec<u32>) {
    fn walk(dir: &std::path::Path, out: &mut BTreeSet<u32>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                let b = text.as_bytes();
                let mut i = 0;
                while i + 2 < b.len() {
                    if b[i] == b'0' && (b[i + 1] == b'x' || b[i + 1] == b'X') {
                        let mut j = i + 2;
                        let mut s = String::new();
                        while j < b.len() && (b[j].is_ascii_hexdigit() || b[j] == b'_') {
                            if b[j] != b'_' {
                                s.push(b[j] as char);
                            }
                            j += 1;
                        }
                        if (2..=8).contains(&s.len())
                            && let Ok(v) = u32::from_str_radix(&s, 16)
                        {
                            out.insert(v);
                        }
                        i = j;
                    } else {
                        i += 1;
                    }
                }
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut all = BTreeSet::new();
    walk(&root.join("src"), &mut all);
    walk(&root.join("../kf-abi/src"), &mut all);
    let ctrls: Vec<u32> = all.iter().copied().filter(|v| *v > 0xffff).collect();
    let classes: Vec<u32> = all
        .iter()
        .copied()
        .filter(|v| (0x10..=0xffff).contains(v))
        .collect();
    (ctrls, classes)
}

/// The generator's dictionary: ids and classes read from the sources, and (campaign only) the ids
/// and classes some link CLAIMS, each with the parameter sizes at which its answer changes.
#[derive(Default)]
struct Dict {
    ctrls: Vec<u32>,
    classes: Vec<u32>,
    claimed: Vec<u32>,
    claimed_classes: Vec<u32>,
    /// control id, or `class | 1 << 31`, to the sizes whose answer differs from the id's usual one.
    sizes: std::collections::BTreeMap<u32, Vec<usize>>,
}

fn plain_dict() -> Dict {
    let (ctrls, classes) = dictionary();
    Dict {
        ctrls,
        classes,
        ..Default::default()
    }
}

/// Probe every dictionary id and class at every candidate size against one chain (both arms) and
/// keep the ids some link answers and the sizes at which the answer changes: a cheap coverage
/// signal without instrumentation.
fn discovered_dict() -> Dict {
    let mut d = plain_dict();
    let mut cand: Vec<usize> = (0..=512).step_by(4).collect();
    cand.extend((520..=8192).step_by(8));
    for windows in [false, true] {
        let planes = Arc::new(Planes::default());
        let mut c = chain(windows, &planes);
        let world = session(0, 0, &d);
        for w in &world {
            c.respond(w);
        }
        let (client, device, subdev) = (
            u32::from_le_bytes(world[0].payload[0..4].try_into().unwrap()),
            u32::from_le_bytes(world[1].payload[8..12].try_into().unwrap()),
            u32::from_le_bytes(world[2].payload[8..12].try_into().unwrap()),
        );
        for &id in &d.ctrls {
            let object = match id >> 16 {
                0x0000 => client,
                0x0080 => device,
                _ => subdev,
            };
            let mut sigs = std::collections::BTreeMap::<String, Vec<usize>>::new();
            for &n in &cand {
                let body = rpcwire::control_body(client, object, id, n as u32, 0, &vec![0u8; n]);
                let cmd = RpcCommand {
                    function: kf_gsp::RpcFunction::RmControl,
                    code: kf_rm::abi::FUNCTIONS.gsp_rm_control,
                    sequence: 9,
                    payload: body,
                    elements: 1,
                    delivered: Vec::new(),
                };
                let sig = match catch_unwind(AssertUnwindSafe(|| c.respond(&cmd))) {
                    Ok(Some(r)) => format!(
                        "{:x}/{:?}/{}",
                        r.rpc_result,
                        r.rpc_result_private,
                        r.body.len() as i64 - cmd.payload.len() as i64
                    ),
                    Ok(None) => "none".into(),
                    Err(_) => "panic".into(),
                };
                sigs.entry(sig).or_default().push(n);
            }
            if sigs.len() > 1 || !sigs.contains_key("none") {
                if !d.claimed.contains(&id) {
                    d.claimed.push(id);
                }
                let common = sigs.values().map(Vec::len).max().unwrap_or(0);
                let e = d.sizes.entry(id).or_default();
                for v in sigs
                    .values()
                    .filter(|v| v.len() < common || sigs.len() == 1)
                {
                    e.extend(v.iter().take(16));
                }
                e.sort_unstable();
                e.dedup();
            }
        }
        let mut h = 0xbeef_0000u32;
        for &class in &d.classes {
            let mut sigs = std::collections::BTreeMap::<String, Vec<usize>>::new();
            for &n in cand.iter().step_by(2) {
                h += 1;
                let parent = match class {
                    0x0080 => client,
                    0x2080 => device,
                    _ => subdev,
                };
                let body =
                    rpcwire::alloc_body(client, parent, h, class, n as u32, 0, &vec![0u8; n]);
                let cmd = RpcCommand {
                    function: kf_gsp::RpcFunction::RmAlloc,
                    code: kf_rm::abi::FUNCTIONS.gsp_rm_alloc,
                    sequence: 9,
                    payload: body,
                    elements: 1,
                    delivered: Vec::new(),
                };
                let sig = match catch_unwind(AssertUnwindSafe(|| c.respond(&cmd))) {
                    Ok(Some(r)) => format!("{:x}", r.rpc_result),
                    Ok(None) => "none".into(),
                    Err(_) => "panic".into(),
                };
                sigs.entry(sig).or_default().push(n);
            }
            if sigs.len() > 1 || sigs.contains_key("0") {
                if !d.claimed_classes.contains(&class) {
                    d.claimed_classes.push(class);
                }
                let common = sigs.values().map(Vec::len).max().unwrap_or(0);
                let e = d.sizes.entry(class | 0x8000_0000).or_default();
                for v in sigs
                    .values()
                    .filter(|v| v.len() < common || sigs.len() == 1)
                {
                    e.extend(v.iter().take(16));
                }
                e.sort_unstable();
                e.dedup();
            }
        }
    }
    eprintln!(
        "discovery: {} of {} ids claimed, {} of {} classes, {} sized",
        d.claimed.len(),
        d.ctrls.len(),
        d.claimed_classes.len(),
        d.classes.len(),
        d.sizes.len()
    );
    d
}

/// What the planes were told, per command.
#[derive(Default)]
struct Planes {
    chan: Mutex<Vec<String>>,
    mem: Mutex<Vec<String>>,
}

fn chain(windows: bool, planes: &Arc<Planes>) -> Box<dyn CommandPolicy> {
    let driver = if windows {
        support::abi()
    } else {
        *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench driver")
    };
    let p1 = planes.clone();
    let p2 = planes.clone();
    let links = kf_rm::ObjectLinks {
        objects: Some(kf_rm::rmrpc::ObjectPolicy::over(
            &driver,
            if windows {
                kf_abi::GuestOs::Windows
            } else {
                kf_abi::GuestOs::Linux
            },
            Box::new(kf_rm::rmrpc::GraphObjects::new(kf_chip::Family::Ampere)),
            Default::default(),
        )),
        memory: Some(kf_rm::MemoryLink {
            sink: Arc::new(move |s| p2.mem.lock().unwrap().push(format!("{s:?}"))),
            guest_os: if windows {
                kf_abi::GuestOs::Windows
            } else {
                kf_abi::GuestOs::Linux
            },
        }),
        channels: Some(Arc::new(move |s| {
            let mut v = p1.chan.lock().unwrap();
            v.push(format!("{s:?}"));
            // A token for every statement: the plane accepted it (the deepest path through the link).
            kf_rm::chanlink::ChanAnswer::Token(v.len() as u32)
        })),
        display: Some((&kf_chip::display::AMPERE).into()),
        console: None,
        windows: kf_rm::winprofile::WindowsProfile::on(),
        utilization: None,
    };
    let mut c = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver,
        Default::default(),
        Default::default(),
        links,
    );
    let fn1 = if windows {
        support::windows(&driver)
    } else {
        support::linux(&driver)
    };
    c.respond(&fn1);
    c
}

/// A parameter block: honest, short, long, saturated or random, with a declared size that may lie.
fn params(r: &mut Rng, known: &[usize]) -> (Vec<u8>, u32) {
    let len = match r.below(if known.is_empty() { 8 } else { 12 }) {
        8..=11 => {
            let k = r.pick(known);
            match r.below(8) {
                0 => k.saturating_sub(1),
                1 => k + 1,
                2 => k.saturating_sub(4),
                3 => k + 4,
                _ => k,
            }
        }
        0 => 0,
        1 => r.below(16) as usize,
        2 | 3 => r.below(256) as usize,
        4 => r.below(4096) as usize,
        5 => 4 * r.below(1024) as usize,
        6 => r.pick(&[
            8usize, 16, 32, 40, 48, 56, 64, 112, 128, 264, 512, 1024, 2048, 4096,
        ]),
        _ => r.below(9000) as usize,
    };
    let mut p = vec![0u8; len];
    match r.below(5) {
        0 => {}
        1 => p.fill(0xff),
        2 => p.iter_mut().for_each(|b| *b = r.next() as u8),
        3 => {
            // small integers in every word: counts, indices and enum values that pass a first check
            for w in p.chunks_mut(4) {
                let v = r.pick(&[
                    0u32,
                    1,
                    2,
                    3,
                    4,
                    7,
                    8,
                    15,
                    16,
                    31,
                    32,
                    63,
                    64,
                    255,
                    256,
                    0x7fff_ffff,
                    0xffff_ffff,
                    0x8000_0000,
                ]);
                let b = v.to_le_bytes();
                w.copy_from_slice(&b[..w.len()]);
            }
        }
        _ => {
            for w in p.chunks_mut(4) {
                let v = if r.chance(1, 3) {
                    r.u32()
                } else {
                    r.below(64) as u32
                };
                let b = v.to_le_bytes();
                w.copy_from_slice(&b[..w.len()]);
            }
        }
    }
    let n = len as u32;
    let declared = match r.below(10) {
        0 => n.wrapping_add(1),
        1 => n.wrapping_sub(1),
        2 => 0,
        3 => u32::MAX,
        4 => r.u32(),
        5 => n.wrapping_mul(2),
        _ => n,
    };
    (p, declared)
}

/// One session: a world, then `n` hostile commands.
fn session(seed: u64, n: usize, dict: &Dict) -> Vec<RpcCommand> {
    let mut r = Rng::new(seed);
    let client = 0xc1d0_0000 | (r.below(0x100) as u32);
    let device = 0xff01_0000 | r.below(0x100) as u32;
    let subdev = 0xff02_0000 | r.below(0x100) as u32;
    let mut handles = vec![
        client,
        device,
        subdev,
        0,
        0xffff_ffff,
        0xc1d0_0002,
        0xff03_0000,
    ];
    // (handle, class) of every object the session tried to create: a control is mostly aimed at
    // an object of the class its id names (`id >> 16`).
    let mut objects: Vec<(u32, u32)> = vec![(client, 0x0000), (device, 0x0080), (subdev, 0x2080)];
    let mut out = Vec::with_capacity(n + 3);
    let mk = |function, code: u32, payload: Vec<u8>, seq: u32| RpcCommand {
        function,
        code,
        sequence: seq,
        payload,
        elements: 1,
        delivered: Vec::new(),
    };
    let alloc = kf_rm::abi::FUNCTIONS.gsp_rm_alloc;
    let control = kf_rm::abi::FUNCTIONS.gsp_rm_control;
    let cls = &kf_rm::abi::FUNCTIONS;
    use kf_gsp::RpcFunction as F;
    out.push(mk(
        F::RmAlloc,
        alloc,
        rpcwire::client_root_alloc_body(0x41, client, 1234),
        0,
    ));
    out.push(mk(
        F::RmAlloc,
        alloc,
        rpcwire::alloc_body(
            client,
            client,
            device,
            0x80,
            56,
            0,
            &rpcwire::device_params(0, 0, 0),
        ),
        1,
    ));
    out.push(mk(
        F::RmAlloc,
        alloc,
        rpcwire::alloc_body(client, device, subdev, 0x2080, 4, 0, &[0; 4]),
        2,
    ));
    for i in 0..n {
        let seq = 3 + i as u32;
        let h = |r: &mut Rng, hs: &Vec<u32>| if r.chance(7, 8) { r.pick(hs) } else { r.u32() };
        let cmd = match r.below(20) {
            0..=10 => {
                let id = if !dict.claimed.is_empty() && r.chance(3, 4) {
                    r.pick(&dict.claimed)
                } else if r.chance(15, 16) {
                    r.pick(&dict.ctrls)
                } else {
                    r.u32()
                };
                let known = dict.sizes.get(&id).map_or(&[][..], |v| &v[..]);
                let (p, decl) = params(&mut r, known);
                let wild = r.u32();
                let flags = r.pick(&[0u32, 0, 0, 0, 0, 0, 1, 2, 3, wild]);
                let target: Vec<u32> = objects
                    .iter()
                    .filter(|o| o.1 == id >> 16)
                    .map(|o| o.0)
                    .collect();
                let object = if !target.is_empty() && r.chance(7, 8) {
                    r.pick(&target)
                } else {
                    h(&mut r, &handles)
                };
                let owner = if r.chance(15, 16) {
                    client
                } else {
                    h(&mut r, &handles)
                };
                let mut body = rpcwire::control_body(owner, object, id, decl, flags, &p);
                if r.chance(1, 16) {
                    let cut = r.below(body.len() as u64 + 1) as usize;
                    body.truncate(cut);
                }
                mk(F::RmControl, control, body, seq)
            }
            11..=15 => {
                let class = if !dict.claimed_classes.is_empty() && r.chance(3, 4) {
                    r.pick(&dict.claimed_classes)
                } else if r.chance(15, 16) {
                    r.pick(&dict.classes)
                } else {
                    r.u32()
                };
                let known = dict
                    .sizes
                    .get(&(class | 0x8000_0000))
                    .map_or(&[][..], |v| &v[..]);
                let (p, decl) = params(&mut r, known);
                let new = if r.chance(3, 4) {
                    0xcafe_0000 | r.below(0x40) as u32
                } else {
                    h(&mut r, &handles)
                };
                let parent = if r.chance(3, 4) {
                    r.pick(&objects).0
                } else {
                    h(&mut r, &handles)
                };
                let wild = r.u32();
                let aflags = r.pick(&[0u32, 0, 1, wild]);
                let owner = if r.chance(15, 16) {
                    client
                } else {
                    h(&mut r, &handles)
                };
                objects.push((new, class));
                let mut body = rpcwire::alloc_body(owner, parent, new, class, decl, aflags, &p);
                if r.chance(1, 16) {
                    let cut = r.below(body.len() as u64 + 1) as usize;
                    body.truncate(cut);
                }
                handles.push(new);
                mk(F::RmAlloc, alloc, body, seq)
            }
            16 => mk(
                F::Free,
                cls.free,
                rpcwire::free_body(
                    h(&mut r, &handles),
                    h(&mut r, &handles),
                    h(&mut r, &handles),
                ),
                seq,
            ),
            17 => mk(
                F::DupObject,
                cls.dup_object,
                rpcwire::dup_body(
                    h(&mut r, &handles),
                    h(&mut r, &handles),
                    0xd000_0000 | r.below(64) as u32,
                    h(&mut r, &handles),
                    h(&mut r, &handles),
                    r.below(4) as u32,
                ),
                seq,
            ),
            _ => {
                // any other function code, with any body
                let code = r.below(128) as u32;
                let (p, _) = params(&mut r, &[]);
                mk(cls.classify(code), code, p, seq)
            }
        };
        let mut cmd = cmd;
        // The DELIVERED body (alloc tail the guest wrote past paramsSize), sometimes.
        if r.chance(1, 8) {
            let extra = r.below(512) as usize;
            let mut d = cmd.payload.clone();
            d.extend((0..extra).map(|_| r.next() as u8));
            cmd.delivered = d;
            cmd.elements = 1 + r.below(16) as u32;
        }
        out.push(cmd);
    }
    out
}

static STATS: Mutex<Vec<(String, u32, Option<u32>)>> = Mutex::new(Vec::new());

/// One finding: what broke, on which seed and step.
// The fields are read through `Debug`, in the failure message.
#[allow(dead_code)]
#[derive(Debug)]
struct Finding {
    windows: bool,
    seed: u64,
    step: usize,
    what: String,
    cmd: String,
}

fn hexdump(c: &RpcCommand) -> String {
    let head: String = c
        .payload
        .iter()
        .take(96)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!(
        "{:?} code={} seq={} len={} delivered={} payload[..96]={head}",
        c.function,
        c.code,
        c.sequence,
        c.payload.len(),
        c.delivered.len()
    )
}

/// Run one session; the reply transcript (for determinism) or the first finding.
fn run(windows: bool, seed: u64, cmds: &[RpcCommand]) -> Result<Vec<String>, Finding> {
    let planes = Arc::new(Planes::default());
    let mut c = chain(windows, &planes);
    let mut transcript = Vec::with_capacity(cmds.len());
    for (step, cmd) in cmds.iter().enumerate() {
        let (c0, m0) = (
            planes.chan.lock().unwrap().len(),
            planes.mem.lock().unwrap().len(),
        );
        let t = Instant::now();
        let got = catch_unwind(AssertUnwindSafe(|| {
            let reply = c.respond(cmd);
            let held = c.holds_for_refresh(cmd);
            (reply, held)
        }));
        let dt = t.elapsed();
        let finding = |what: String| Finding {
            windows,
            seed,
            step,
            what,
            cmd: hexdump(cmd),
        };
        let (reply, held) = match got {
            Ok(v) => v,
            Err(e) => {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                return Err(finding(format!("PANIC: {msg}")));
            }
        };
        if dt > SLOW_CAP {
            return Err(finding(format!("SLOW: {dt:?}")));
        }
        if let Some(r) = &reply
            && r.body.len() > REPLY_CAP
        {
            return Err(finding(format!("REPLY of {} bytes", r.body.len())));
        }
        let (c1, m1) = (
            planes.chan.lock().unwrap().len(),
            planes.mem.lock().unwrap().len(),
        );
        if c1 - c0 > EMIT_CAP || m1 - m0 > EMIT_CAP {
            return Err(finding(format!(
                "EMIT: {} channel, {} memory statements",
                c1 - c0,
                m1 - m0
            )));
        }
        if std::env::var_os("KF_FUZZ_STATS").is_some() {
            let id = if cmd.function == kf_gsp::RpcFunction::RmControl && cmd.payload.len() >= 12 {
                u32::from_le_bytes(cmd.payload[8..12].try_into().unwrap())
            } else if cmd.function == kf_gsp::RpcFunction::RmAlloc && cmd.payload.len() >= 16 {
                u32::from_le_bytes(cmd.payload[12..16].try_into().unwrap())
            } else {
                cmd.code
            };
            let status = reply.as_ref().map(|r| r.rpc_result);
            STATS
                .lock()
                .unwrap()
                .push((format!("{:?}", cmd.function), id, status));
        }
        transcript.push(format!(
            "{:?} held={held} chan={} mem={}",
            reply.map(|r| (r.rpc_result, r.rpc_result_private, r.body)),
            c1 - c0,
            m1 - m0
        ));
    }
    Ok(transcript)
}

fn campaign(sessions: u64, per: usize, base: u64, dict: &Dict) -> Vec<Finding> {
    assert!(
        dict.ctrls.len() > 500 && dict.classes.len() > 100,
        "dictionary: {} ids, {} classes",
        dict.ctrls.len(),
        dict.classes.len()
    );
    let mut findings = Vec::new();
    for s in 0..sessions {
        let seed = base + s;
        let windows = s % 2 == 1;
        let cmds = session(seed, per, dict);
        match run(windows, seed, &cmds) {
            Ok(t) => {
                if s % 16 == 0 {
                    let again = run(windows, seed, &cmds).map_err(|f| format!("{f:?}"));
                    if again.as_ref() != Ok(&t) {
                        findings.push(Finding {
                            windows,
                            seed,
                            step: 0,
                            what: "NONDETERMINISTIC transcript".into(),
                            cmd: String::new(),
                        });
                    }
                }
            }
            Err(f) => findings.push(f),
        }
    }
    findings
}

#[test]
fn the_served_chain_survives_hostile_commands() {
    let findings = campaign(48, 120, 0x5eed_0000, &plain_dict());
    assert!(
        findings.is_empty(),
        "{} findings, first: {:#?}",
        findings.len(),
        findings.first()
    );
}

#[test]
#[ignore = "campaign: KF_FUZZ_SESSIONS=<n> KF_FUZZ_BASE=<seed> [KF_FUZZ_SECONDS=<s>] cargo test --release -p kf-rm --test boundary_fuzz_chain -- --ignored --nocapture"]
fn campaign_served_chain() {
    let n: u64 = std::env::var("KF_FUZZ_SESSIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    let base: u64 = std::env::var("KF_FUZZ_BASE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let per: usize = std::env::var("KF_FUZZ_PER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let dict = if std::env::var_os("KF_FUZZ_NO_DISCOVERY").is_some() {
        plain_dict()
    } else {
        discovered_dict()
    };
    // ★ Time mode: KF_FUZZ_SECONDS=<s> runs batches of `n` sessions from `base` on until the deadline.
    let secs: u64 = std::env::var("KF_FUZZ_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut findings = campaign(n, per, base, &dict);
    let mut next = base + n;
    while findings.is_empty() && std::time::Instant::now() < deadline {
        findings = campaign(n, per, next, &dict);
        next += n;
    }
    let n = next - base;
    let mut kinds = std::collections::BTreeMap::<String, (usize, String)>::new();
    for f in &findings {
        let key: String = f.what.chars().take(120).collect();
        let e = kinds.entry(key).or_insert((0, format!("{f:?}")));
        e.0 += 1;
    }
    for (k, (n, first)) in &kinds {
        eprintln!("FINDING x{n}: {k}\n  first: {first}");
    }
    let st = STATS.lock().unwrap();
    if !st.is_empty() {
        let mut by = std::collections::BTreeMap::<(String, Option<u32>), usize>::new();
        let mut ok_ids = BTreeSet::new();
        for (f, id, s) in st.iter() {
            *by.entry((f.clone(), *s)).or_default() += 1;
            if *s == Some(0) {
                ok_ids.insert((f.clone(), *id));
            }
        }
        for ((f, s), c) in &by {
            eprintln!("STAT {f} {s:x?}: {c}");
        }
        eprintln!(
            "STAT distinct (function,id) answered NV_OK: {}",
            ok_ids.len()
        );
    }
    eprintln!(
        "campaign: {n} sessions x {per} commands, {} findings",
        findings.len()
    );
    assert!(findings.is_empty());
}

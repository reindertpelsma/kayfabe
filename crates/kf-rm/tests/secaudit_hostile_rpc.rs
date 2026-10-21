//! SECURITY AUDIT 2026-10-21 (scratch harness): hostile GSP RPC bodies against the production served chain.
//!
//! The guest controls every byte of an RPC body. This drives `kf_rm::served_policy` (the chain `kf-qemu`
//! builds) with structured and mutated RPCs and records every panic with its location. A panic on the
//! drainer thread aborts the VMM (`kf-qemu/src/failclosed.rs`), so any panic found here is a guest-triggered
//! VM stop. The generator is deterministic.
#![allow(clippy::unusual_byte_groupings, dead_code, missing_docs)]

#[path = "support/ga106.rs"]
mod ga106;
#[path = "support/rpcwire.rs"]
mod rpcwire;

use kf_abi::GuestOs;
use kf_gsp::RpcCommand;
use kf_rm::rmrpc::{GraphObjects, ObjectPolicy, ReasmLimits};
use rpcwire as w;
use std::collections::BTreeMap;
use std::sync::Mutex;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u64) as usize]
    }
    fn u32(&mut self) -> u32 {
        // biased towards boundary values
        match self.below(10) {
            0 => 0,
            1 => u32::MAX,
            2 => 0x8000_0000,
            3 => u32::MAX - self.below(64) as u32,
            4 => self.below(16) as u32,
            5 => 0xFFFF,
            _ => self.next() as u32,
        }
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

/// Every 8-hex-digit literal in the kf-rm and kf-abi sources: a corpus of control ids the chain has arms for.
fn corpus() -> Vec<u32> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut ids = std::collections::BTreeSet::new();
    for dir in ["kf-rm/src", "kf-abi/src", "kf-abi/src/generated"] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }
            let Ok(t) = std::fs::read_to_string(&p) else {
                continue;
            };
            let b = t.as_bytes();
            let mut i = 0;
            while i + 10 <= b.len() {
                if b[i] == b'0' && b[i + 1] == b'x' {
                    let mut j = i + 2;
                    let mut v: u64 = 0;
                    let mut n = 0;
                    while j < b.len() && (b[j].is_ascii_hexdigit() || b[j] == b'_') {
                        if b[j] != b'_' {
                            v = (v << 4) | u64::from((b[j] as char).to_digit(16).unwrap());
                            n += 1;
                        }
                        j += 1;
                        if n > 16 {
                            break;
                        }
                    }
                    if n == 8 && v <= u64::from(u32::MAX) {
                        ids.insert(v as u32);
                    }
                    i = j;
                } else {
                    i += 1;
                }
            }
        }
    }
    ids.into_iter().collect()
}

static PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn fastrand_pick<T: std::fmt::Debug>(st: &T) -> u32 {
    // a deterministic hash of the statement's Debug text
    let t = format!("{st:?}");
    let mut h = 1469598103934665603u64;
    for b in t.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(1099511628211);
    }
    (h % 3) as u32
}

fn chain() -> Box<dyn kf_gsp::CommandPolicy> {
    let abi = kf_rm::abi::gsp_abi_for(kf_abi::versions::BENCH_DRIVER).expect("bench table");
    let objects = ObjectPolicy::over(
        &abi.driver,
        GuestOs::Linux,
        Box::new(GraphObjects::new(kf_chip::Family::Ampere)),
        ReasmLimits::default(),
    );
    kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        abi.driver,
        kf_rm::ChainLogs::default(),
        kf_rm::census::ControlCensusLog::new(),
        kf_rm::ObjectLinks {
            objects: Some(objects),
            memory: Some(kf_rm::MemoryLink {
                sink: std::sync::Arc::new(|_st| {}),
                guest_os: GuestOs::Linux,
            }),
            channels: Some(std::sync::Arc::new(|st| {
                // every other statement is accepted; Token keeps the chain moving
                match fastrand_pick(&st) {
                    0 => kf_rm::chanlink::ChanAnswer::Done,
                    1 => kf_rm::chanlink::ChanAnswer::Token(0x1234),
                    _ => kf_rm::chanlink::ChanAnswer::NotOurs,
                }
            })),
            display: kf_chip::display::display_for(kf_chip::arch::GA100, 0x6)
                .map(kf_rm::DisplaySeat::from),
            console: None,
        },
    )
}

fn build(rng: &mut Rng, ids: &[u32]) -> RpcCommand {
    let funcs = [
        w::fn_id::SET_GUEST_SYSTEM_INFO,
        w::fn_id::FREE,
        w::fn_id::DUP_OBJECT,
        w::fn_id::UNLOADING_GUEST_DRIVER,
        w::fn_id::GET_GSP_STATIC_INFO,
        w::fn_id::CONTINUATION_RECORD,
        w::fn_id::GSP_SET_SYSTEM_INFO,
        w::fn_id::SET_REGISTRY,
        w::fn_id::GSP_RM_CONTROL,
        w::fn_id::GSP_RM_CONTROL,
        w::fn_id::GSP_RM_CONTROL,
        w::fn_id::GSP_RM_ALLOC,
        w::fn_id::GSP_RM_ALLOC,
    ];
    let code = if rng.below(12) == 0 {
        rng.u32()
    } else {
        rng.pick(&funcs)
    };
    let hc = 0xc1d0_0000 | rng.below(4) as u32;
    let ho = 0x5c00_0000 | rng.below(8) as u32;
    let mut payload = match code {
        w::fn_id::GSP_RM_CONTROL => {
            let cmd = if rng.below(4) == 0 {
                rng.u32()
            } else {
                rng.pick(ids)
            };
            let plen = match rng.below(5) {
                0 => 0,
                1 => rng.below(64) as usize,
                2 => rng.below(600) as usize,
                3 => rng.below(4300) as usize,
                _ => rng.below(70_000) as usize,
            };
            let mut params = rng.bytes(plen.min(70_000));
            if rng.below(3) == 0 {
                params.iter_mut().for_each(|b| *b = 0);
            }
            let declared = match rng.below(6) {
                0 => plen as u32,
                1 => plen as u32 + rng.below(8) as u32,
                2 => (plen as u32).saturating_sub(rng.below(8) as u32),
                3 => rng.u32(),
                _ => plen as u32,
            };
            w::control_body(hc, ho, cmd, declared, rng.pick(&[0, 1, 2, 3, 0xff]), &params)
        }
        w::fn_id::GSP_RM_ALLOC if rng.below(3) == 0 => {
            // structured: the objects a driver really allocates, with hostile parents and mutated params
            let (class, params) = match rng.below(6) {
                0 => (w::NV01_ROOT_CLIENT, w::client_root_params(hc, rng.u32())),
                1 => (w::NV01_DEVICE_0, w::device_params(rng.below(2) as u32, hc, rng.u32())),
                2 => (w::KEPLER_CHANNEL_GROUP_A, w::tsg_params(0x5c00_0000 | rng.below(8) as u32, rng.u32())),
                3 => (w::FERMI_CONTEXT_SHARE_A, w::ctxshare_params(0x5c00_0000 | rng.below(8) as u32, rng.u32(), rng.u32())),
                4 => (w::AMPERE_CHANNEL_GPFIFO_A, w::channel_params(rng.u32(), 0x5c00_0000 | rng.below(8) as u32, 0x5c00_0000 | rng.below(8) as u32)),
                _ => {
                    let n = rng.below(80) as usize;
                    (w::FERMI_VASPACE_A, rng.bytes(n))
                }
            };
            let mut params = params;
            if !params.is_empty() && rng.below(2) == 0 {
                let i = rng.below(params.len() as u64) as usize;
                params[i] = rng.next() as u8;
            }
            let declared = if rng.below(4) == 0 { rng.u32() } else { params.len() as u32 };
            let pp = 0x5c00_0000 | rng.below(8) as u32;
            let parent = rng.pick(&[0, hc, ho, pp]);
            let obj = 0x5c00_0000 | rng.below(8) as u32;
            w::alloc_body(hc, parent, obj, class, declared, rng.u32(), &params)
        }
        w::fn_id::GSP_RM_ALLOC => {
            let class = if rng.below(2) == 0 {
                rng.pick(&[
                    w::NV01_ROOT,
                    w::NV01_ROOT_CLIENT,
                    w::NV01_DEVICE_0,
                    w::FERMI_VASPACE_A,
                    w::KEPLER_CHANNEL_GROUP_A,
                    w::FERMI_CONTEXT_SHARE_A,
                    w::AMPERE_CHANNEL_GPFIFO_A,
                    w::AMPERE_COMPUTE_B,
                    w::AMPERE_DMA_COPY_B,
                    w::AMPERE_B,
                    w::GP100_UVM_SW,
                    w::NV01_MEMORY_SYSTEM,
                    0x2080,
                    0x2081,
                    0x0000_0073,
                    0x5070,
                    0xc67d,
                    0xc67e,
                    0xc67b,
                    0x0000_9170,
                    0xc57d,
                ])
            } else {
                rng.u32()
            };
            let plen = rng.below(700) as usize;
            let params = rng.bytes(plen);
            let declared = match rng.below(5) {
                0 => plen as u32,
                1 => rng.u32(),
                2 => plen as u32 + 1,
                _ => plen as u32,
            };
            let parent = if rng.below(3) == 0 { 0 } else { hc };
            let obj = if rng.below(3) == 0 { 0 } else { ho };
            w::alloc_body(hc, parent, obj, class, declared, rng.u32(), &params)
        }
        w::fn_id::FREE => {
            let r = rng.u32();
            let a = rng.pick(&[0, hc, ho]);
            w::free_body(hc, a, rng.pick(&[hc, ho, r]))
        }
        w::fn_id::DUP_OBJECT => {
            w::dup_body(hc, rng.pick(&[0, hc, ho]), ho, rng.pick(&[hc, 0xc1d0_0003]), rng.pick(&[ho, 0x5c00_0001]), rng.below(4) as u32)
        }
        _ => {
            let n = rng.below(1500) as usize;
            rng.bytes(n)
        }
    };
    // mutate
    match rng.below(6) {
        0 if !payload.is_empty() => {
            let n = rng.below(payload.len() as u64 + 1) as usize;
            payload.truncate(n);
        }
        1 if !payload.is_empty() => {
            for _ in 0..1 + rng.below(4) {
                let i = rng.below(payload.len() as u64) as usize;
                payload[i] = rng.next() as u8;
            }
        }
        2 if payload.len() >= 4 => {
            let i = rng.below((payload.len() / 4) as u64) as usize * 4;
            payload[i..i + 4].copy_from_slice(&rng.u32().to_le_bytes());
        }
        _ => {}
    }
    let delivered = if rng.below(8) == 0 {
        let n = rng.below(300) as usize;
        rng.bytes(n)
    } else {
        Vec::new()
    };
    RpcCommand {
        function: kf_rm::abi::FUNCTIONS.classify(code),
        code,
        sequence: rng.u32(),
        payload,
        elements: 1 + rng.below(3) as u32,
        delivered,
    }
}

#[test]
fn hostile_rpcs_do_not_panic_the_served_chain() {
    let ids = corpus();
    assert!(ids.len() > 200, "corpus too small: {}", ids.len());
    std::panic::set_hook(Box::new(|info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| info.payload().downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_default();
        if let Ok(mut p) = PANICS.lock() {
            p.push(format!("{loc}: {msg}"));
        }
    }));
    let seed: u64 = std::env::var("SECAUDIT_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5EC_A0D1_7);
    let iters: usize = std::env::var("SECAUDIT_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);
    let mut rng = Rng(seed | 1);
    let mut policy = chain();
    let mut first: BTreeMap<String, String> = BTreeMap::new();
    let mut n_panics = 0usize;
    let mut stat = [0usize; 3];
    for it in 0..iters {
        if it % 5000 == 0 {
            policy = chain(); // fresh graph now and then
        }
        let cmd = build(&mut rng, &ids);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let r = policy.respond(&cmd);
            match r {
                None => 0usize,
                Some(rep) if rep.rpc_result == 0 => 1,
                Some(_) => 2,
            }
        }));
        if let Ok(k) = &r {
            stat[*k] += 1;
        }
        if r.is_err() {
            n_panics += 1;
            let loc = PANICS.lock().ok().and_then(|mut p| p.pop()).unwrap_or_default();
            let key = loc.split(": ").next().unwrap_or("").to_string();
            first.entry(key).or_insert_with(|| {
                format!(
                    "{loc} | fn={} code={} payload_len={} head={:02x?}",
                    cmd.code,
                    cmd.code,
                    cmd.payload.len(),
                    &cmd.payload[..cmd.payload.len().min(48)]
                )
            });
            policy = chain(); // a panicked policy may be mid-update
        }
    }
    eprintln!("SECAUDIT iterations={iters} none={} ok={} refused={} panics={n_panics} distinct_sites={}", stat[0], stat[1], stat[2], first.len());
    for (k, v) in &first {
        eprintln!("SECAUDIT PANIC SITE {k}\n    {v}");
    }
    assert_eq!(n_panics, 0, "guest-reachable panics: {first:#?}");
}

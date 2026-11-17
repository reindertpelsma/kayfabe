//! ★ 2026-11-17 (`docs/design/V3_UTILIZATION_FLAG.md`, device property `report-utilization`): GPU utilisation
//! through the WHOLE served chain, flag ON and flag OFF, with a hostile guest.
//!
//! The claims pinned here, each by name:
//! - OFF is today's behaviour: the three controls reach the ledger, are refused and counted
//!   (`off_*`);
//! - ON answers GPUMON from the cache with a reply that is a function of the cache alone (`on_*`), and the RUSD
//!   page is published through the three-step seqlock at the generated offset (`rusd_*`);
//! - every hostile shape is refused, not answered (`hostile_*`);
//! - the derived-row rule: a guest version with no row gets no link, and the siblings of the three ids stay
//!   refused (`derived_rows_*`, `siblings_*`).
//!
//! No GPU, no host: the chain has no handle to a host at all (the sampler lives in `kf-qemu`), which is the
//! structural proof that a guest request cannot cause a host call.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::DriverVersion;
use kf_abi::utilization::{self as ku, GuestSample, Layout, Percents};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::ChainLogs;
use kf_rm::utilization::{
    Clock, GPUMON_REPLIES_PER_SEC, PublishOutcome, UtilizationSeat, UtilizationState,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const NV_OK: u32 = 0;

#[derive(Debug, Default)]
struct FakeClock(AtomicU64);

impl Clock for FakeClock {
    fn now_ns(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

fn v(major: u16, minor: u16, patch: u16) -> DriverVersion {
    DriverVersion {
        major,
        minor,
        patch,
    }
}

fn abi(version: DriverVersion) -> kf_abi::versions::DriverAbiTable {
    *kf_abi::versions::table_for(version).expect("a measured wire table")
}

struct Rig {
    chain: Box<dyn CommandPolicy>,
    logs: ChainLogs,
    state: Option<Arc<UtilizationState>>,
    clock: Arc<FakeClock>,
    abi: kf_abi::versions::DriverAbiTable,
    layout: Option<&'static Layout>,
}

/// The real served chain at guest driver `version`, with the property on or off.
fn rig(version: DriverVersion, on: bool) -> Rig {
    let abi = abi(version);
    let clock = Arc::new(FakeClock(AtomicU64::new(1_000_000_000)));
    let state = on.then(|| UtilizationState::new(clock.clone()));
    let logs = ChainLogs::default();
    let chain = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        abi,
        logs.clone(),
        Default::default(),
        kf_rm::ObjectLinks {
            utilization: state.clone().map(|state| UtilizationSeat { state }),
            ..Default::default()
        },
    );
    Rig {
        chain,
        logs,
        state,
        clock,
        abi,
        layout: ku::layout(version),
    }
}

impl Rig {
    fn control(&self, cmd: u32, params: &[u8], flags: u32) -> RpcCommand {
        let w = self.abi.rm_control_wire();
        let mut payload = vec![0u8; w.params_off + params.len()];
        payload[0..4].copy_from_slice(&0xc1d0_0002u32.to_le_bytes());
        payload[4..8].copy_from_slice(&0xff03_0000u32.to_le_bytes());
        payload[8..12].copy_from_slice(&cmd.to_le_bytes());
        payload[w.status_off..w.status_off + 4].copy_from_slice(&0x56u32.to_le_bytes());
        payload[w.params_size_off..w.params_size_off + 4]
            .copy_from_slice(&u32::try_from(params.len()).unwrap().to_le_bytes());
        payload[w.rpc_flags_off..w.rpc_flags_off + 4].copy_from_slice(&flags.to_le_bytes());
        payload[w.params_off..].copy_from_slice(params);
        RpcCommand {
            function: RpcFunction::RmControl,
            code: 0x4c,
            sequence: 1,
            payload,
            elements: 1,
            delivered: Vec::new(),
        }
    }

    fn l(&self) -> &'static Layout {
        self.layout.expect("a probed row")
    }

    /// A guest GPUMON request: `type`/`bufSize` as the guest's NVML sets them, and `junk` in every guest-owned
    /// byte (count, tracker, the whole sample buffer).
    fn gpumon_request(&self, junk: u8) -> RpcCommand {
        let l = self.l();
        let mut p = vec![junk; l.params_len()];
        p[0] = 2;
        p[4..8].copy_from_slice(&u32::try_from(l.buffer_size()).unwrap().to_le_bytes());
        self.control(ku::GPUMON_CMD, &p, 0)
    }

    fn init(&self, binit: u8, phys: u64) -> RpcCommand {
        let l = self.l();
        let mut p = vec![0u8; 16];
        p[usize::try_from(l.init_off_binit).unwrap()] = binit;
        p[usize::try_from(l.init_off_phys).unwrap()..][..8].copy_from_slice(&phys.to_le_bytes());
        self.control(l.rusd_init_cmd as u32, &p, 0)
    }

    fn poll(&self, mask: u64) -> RpcCommand {
        let l = self.l();
        let mut p = vec![0u8; 16];
        p[..8].copy_from_slice(&mask.to_le_bytes());
        p[8..12].copy_from_slice(&1000u32.to_le_bytes());
        self.control(l.rusd_poll_cmd as u32, &p, 0)
    }

    fn push(&self, gpu: u8, at: u64) {
        self.clock.0.store(at, Ordering::Relaxed);
        self.state.as_ref().unwrap().push(GuestSample {
            timestamp_ns: at,
            percents: Percents {
                gpu,
                memory: 11,
                nvenc: 0,
                nvdec: 3,
                nvjpg: 0,
                nvofa: 0,
            },
        });
    }

    fn reply_params(&self, r: &kf_gsp::Reply, size: usize) -> Vec<u8> {
        let at = self.abi.rm_control_wire().params_off;
        r.body[at..at + size].to_vec()
    }
}

/// Guest versions a layout row exists for, one per distinct RUSD layout the pinned oracles and the Windows twin cover.
const VERSIONS: [(u16, u16, u16); 3] = [(580, 65, 6), (580, 159, 4), (595, 84, 0)];

// ---------------------------------------------------------------- OFF

/// ★ `report-utilization=off`: the three controls are refused and counted exactly like any control nobody built.
#[test]
fn off_refuses_all_three_and_counts_them_unserviced() {
    for (a, b, c) in VERSIONS {
        let mut r = rig(v(a, b, c), false);
        let l = r.l();
        let reqs = [
            ("GPUMON", r.gpumon_request(0)),
            ("INIT", r.init(1, 0x10_0000)),
            ("POLL", r.poll(u64::MAX)),
        ];
        for (name, cmd) in &reqs {
            assert!(
                r.chain.respond(cmd).is_none(),
                "{a}.{b}.{c}: {name} must be refused with the property off"
            );
        }
        assert_eq!(r.logs.unserviced.total(), 3, "{a}.{b}.{c}");
        let seen: Vec<_> = r
            .logs
            .unserviced
            .sample()
            .iter()
            .filter_map(|u| u.cmd)
            .collect();
        assert_eq!(
            seen,
            vec![
                ku::GPUMON_CMD,
                l.rusd_init_cmd as u32,
                l.rusd_poll_cmd as u32
            ],
            "{a}.{b}.{c}"
        );
    }
}

/// ★ With the property off, the chain cannot even see the state: a link that was never seated cannot answer, so
/// the refusal is the same bytes (None) as for an unrelated control nobody built.
#[test]
fn off_is_indistinguishable_from_an_unbuilt_control() {
    let mut r = rig(v(580, 159, 4), false);
    let unrelated = r.control(0x2080_a099, &[0u8; 64], 0);
    assert!(r.chain.respond(&unrelated).is_none());
    assert!(r.chain.respond(&r.gpumon_request(0)).is_none());
    assert_eq!(r.logs.unserviced.total(), 2);
}

// ---------------------------------------------------------------- ON, GPUMON

/// ★ ON: the reply is a function of the cache only. A guest that fills every byte it owns with junk gets the
/// SAME reply as one that sends zeros, and it is byte-identical to the authored encoding.
#[test]
fn on_gpumon_reply_is_the_cache_and_none_of_the_guests_bytes() {
    for (a, b, c) in VERSIONS {
        let mut r = rig(v(a, b, c), true);
        r.push(42, 2_000_000_000);
        let l = r.l();
        let expected =
            ku::encode_gpumon_reply(l, &r.state.as_ref().unwrap().fresh_samples().unwrap())
                .unwrap();
        let mut seen = Vec::new();
        for junk in [0x00u8, 0xa5, 0xff] {
            let reply = r
                .chain
                .respond(&r.gpumon_request(junk))
                .unwrap_or_else(|| panic!("{a}.{b}.{c}: not answered"));
            assert_eq!(reply.rpc_result, NV_OK);
            let w = r.abi.rm_control_wire();
            assert_eq!(
                u32::from_le_bytes(
                    reply.body[w.status_off..w.status_off + 4]
                        .try_into()
                        .unwrap()
                ),
                NV_OK,
                "the control's own status word"
            );
            let params = r.reply_params(&reply, l.params_len());
            assert_eq!(
                params, expected,
                "{a}.{b}.{c}: junk {junk:#x} reached the reply"
            );
            seen.push(reply.body.len());
        }
        assert!(seen.iter().all(|n| *n == seen[0]), "a fixed size");
        assert_eq!(r.state.as_ref().unwrap().counters().gpumon_served, 3);
    }
}

#[test]
fn on_gpumon_carries_the_percentages_and_no_process() {
    let mut r = rig(v(580, 159, 4), true);
    r.push(42, 2_000_000_000);
    let l = r.l();
    let reply = r.chain.respond(&r.gpumon_request(0)).unwrap();
    let p = r.reply_params(&reply, l.params_len());
    let u32_at = |o: u64| u32::from_le_bytes(p[o as usize..o as usize + 4].try_into().unwrap());
    let s = l.off_samples;
    assert_eq!(u32_at(8), 1, "count");
    assert_eq!(
        u32_at(s + l.off_gr + l.eng_off_util),
        4_200,
        "gpu 42 % in pct x 100"
    );
    assert_eq!(u32_at(s + l.off_fb + l.eng_off_util), 1_100);
    assert_eq!(u32_at(s + l.off_nvdec + l.eng_off_util), 300);
    for eng in [
        l.off_gr,
        l.off_fb,
        l.off_nvenc,
        l.off_nvdec,
        l.off_nvjpg,
        l.off_nvofa,
    ] {
        assert_eq!(
            u32_at(s + eng + l.eng_off_proc_id),
            u32::MAX,
            "no process id"
        );
        assert_eq!(u32_at(s + eng + l.eng_off_sub_proc_id), u32::MAX);
    }
}

/// ★ Unavailable is not zero: no fresh sample, no answer; the refusal is the one the guest has today.
#[test]
fn on_without_a_fresh_sample_gpumon_is_refused_as_before() {
    let mut r = rig(v(580, 159, 4), true);
    assert!(
        r.chain.respond(&r.gpumon_request(0)).is_none(),
        "empty cache"
    );
    r.push(10, 2_000_000_000);
    r.clock
        .0
        .store(2_000_000_000 + 4_000_000_000, Ordering::Relaxed);
    assert!(
        r.chain.respond(&r.gpumon_request(0)).is_none(),
        "a 4 s old cache is not served"
    );
    let c = r.state.as_ref().unwrap().counters();
    assert_eq!((c.gpumon_served, c.gpumon_no_sample), (0, 2));
    assert_eq!(r.logs.unserviced.total(), 2, "both reached the ledger");
}

/// ★ Flood: a guest asking as fast as it can is served the bucket's burst and nothing more, with no host call
/// possible (the chain holds no host handle).
#[test]
fn hostile_flood_is_rate_limited() {
    let mut r = rig(v(580, 159, 4), true);
    r.push(10, 2_000_000_000);
    let req = r.gpumon_request(0);
    let served = (0..1000)
        .filter(|_| r.chain.respond(&req).is_some())
        .count() as u64;
    assert_eq!(served, GPUMON_REPLIES_PER_SEC);
    let c = r.state.as_ref().unwrap().counters();
    assert_eq!(c.gpumon_rate_limited, 1000 - GPUMON_REPLIES_PER_SEC);
    // a second later the bucket is full again
    r.clock.0.fetch_add(1_000_000_000, Ordering::Relaxed);
    r.state.as_ref().unwrap().push(GuestSample {
        timestamp_ns: r.clock.0.load(Ordering::Relaxed),
        percents: Percents::default(),
    });
    assert!(r.chain.respond(&req).is_some());
}

// ---------------------------------------------------------------- hostile shapes

#[test]
fn hostile_gpumon_shapes_are_refused_not_answered() {
    let mut r = rig(v(580, 159, 4), true);
    r.push(10, 2_000_000_000);
    let l = r.l();
    let good = |r: &Rig| {
        let mut p = vec![0u8; l.params_len()];
        p[0] = 2;
        p[4..8].copy_from_slice(&u32::try_from(l.buffer_size()).unwrap().to_le_bytes());
        let _ = r;
        p
    };
    let mut cases: Vec<(&str, RpcCommand)> = Vec::new();
    let mut p = good(&r);
    p[0] = 1; // PWR_MONITOR_STATUS
    cases.push(("another sample type", r.control(ku::GPUMON_CMD, &p, 0)));
    let mut p = good(&r);
    p[4..8].copy_from_slice(&(u32::try_from(l.buffer_size()).unwrap() - 1).to_le_bytes());
    cases.push(("a short bufSize", r.control(ku::GPUMON_CMD, &p, 0)));
    let mut p = good(&r);
    p[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    cases.push(("a huge bufSize", r.control(ku::GPUMON_CMD, &p, 0)));
    let p = good(&r);
    cases.push((
        "truncated params",
        r.control(ku::GPUMON_CMD, &p[..p.len() - 1], 0),
    ));
    let mut long = good(&r);
    long.push(0);
    cases.push(("oversized params", r.control(ku::GPUMON_CMD, &long, 0)));
    cases.push(("empty params", r.control(ku::GPUMON_CMD, &[], 0)));
    cases.push((
        "a serialized envelope",
        r.control(ku::GPUMON_CMD, &good(&r), 1 << 1),
    ));
    let mut not_control = r.control(ku::GPUMON_CMD, &good(&r), 0);
    not_control.function = RpcFunction::RmAlloc;
    cases.push(("not GSP_RM_CONTROL", not_control));
    for (name, cmd) in cases {
        assert!(
            r.chain.respond(&cmd).is_none(),
            "{name} must not be answered"
        );
    }
    assert_eq!(r.state.as_ref().unwrap().counters().gpumon_served, 0);
}

#[test]
fn hostile_rusd_init_and_poll_shapes_are_refused_and_change_nothing() {
    let mut r = rig(v(580, 159, 4), true);
    let state = r.state.clone().unwrap();
    for (name, phys) in [
        ("a zero address", 0u64),
        ("an unaligned address", 0x10_0001),
        ("an address at 2^52", 1 << 52),
        ("the top of the address space", 0xffff_ffff_ffff_f000),
    ] {
        assert!(r.chain.respond(&r.init(1, phys)).is_none(), "{name}");
        assert_eq!(state.rusd_page(), None, "{name} must not be remembered");
    }
    assert!(r.chain.respond(&r.init(2, 0x10_0000)).is_none(), "bInit 2");
    assert!(
        r.chain.respond(&r.init(0xff, 0x10_0000)).is_none(),
        "bInit 255"
    );
    let l = r.l();
    let short = r.control(l.rusd_init_cmd as u32, &[1u8; 15], 0);
    assert!(r.chain.respond(&short).is_none());
    let long = r.control(l.rusd_poll_cmd as u32, &[0u8; 17], 0);
    assert!(r.chain.respond(&long).is_none());
    let ser = r.control(l.rusd_poll_cmd as u32, &[0u8; 16], 1 << 1);
    assert!(r.chain.respond(&ser).is_none());
    assert_eq!(state.rusd_page(), None);
    assert!(!state.rusd_wants_utilization());
    let c = state.counters();
    assert_eq!((c.init_linked, c.poll_seen), (0, 0));
    assert!(c.init_refused >= 6);
}

// ---------------------------------------------------------------- RUSD

/// A model of the guest's RUSD page: a RAM that refuses writes outside `[base, base+len)`.
struct Ram {
    base: u64,
    mem: Vec<u8>,
    writes: Vec<(u64, usize)>,
}

impl Ram {
    fn write(&mut self, gpa: u64, b: &[u8]) -> bool {
        let Some(rel) = gpa.checked_sub(self.base) else {
            return false;
        };
        let rel = rel as usize;
        if rel + b.len() > self.mem.len() {
            return false;
        }
        self.mem[rel..rel + b.len()].copy_from_slice(b);
        self.writes.push((gpa, b.len()));
        true
    }
}

/// The reader's side of `cl00de.h`'s seqlock: timestamp, copy, timestamp.
fn read_record(page: &[u8], l: &Layout) -> Option<(u64, u32, u32, [u32; 4])> {
    let at = l.rusd_off_util as usize;
    let ts = |p: &[u8]| u64::from_le_bytes(p[at..at + 8].try_into().unwrap());
    let t0 = ts(page);
    if t0 == l.ts_invalid || t0 == l.ts_in_progress {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes(page[at + o..at + o + 4].try_into().unwrap());
    let info = l.util_off_info as usize;
    let gpu = u32_at(info + l.info_off_gpu as usize);
    let mem = u32_at(info + l.info_off_mem as usize);
    let mut eng = [0u32; 4];
    for (i, e) in eng.iter_mut().enumerate() {
        *e = u32_at(
            info + l.info_off_eng as usize
                + i * l.eng_util_size as usize
                + l.eng_util_off_busy as usize,
        );
    }
    (ts(page) == t0).then_some((t0, gpu, mem, eng))
}

/// ★ The whole RUSD path through the chain: INIT links the page, POLL asks for the record, the drainer's publish
/// writes it, an NVML-style reader reads a consistent record, and an unlink stops it.
#[test]
fn rusd_link_poll_publish_read_unlink() {
    for (a, b, c) in VERSIONS {
        let mut r = rig(v(a, b, c), true);
        let l = r.l();
        let state = r.state.clone().unwrap();
        let base = 0x10_0000u64;
        let mut ram = Ram {
            base,
            mem: vec![0u8; 4096],
            writes: Vec::new(),
        };
        r.push(64, 2_000_000_000);

        // Before INIT and POLL nothing is written and the record is invalid (timestamp 0).
        assert_eq!(
            state.publish(&mut |g, b| ram.write(g, b)),
            PublishOutcome::NotWanted
        );
        assert!(read_record(&ram.mem, l).is_none());

        assert!(
            r.chain
                .respond(&r.init(1, base))
                .is_some_and(|x| x.rpc_result == NV_OK)
        );
        assert_eq!(state.rusd_page(), Some(base));
        // linked but not polled for the record: still nothing
        assert_eq!(
            state.publish(&mut |g, b| ram.write(g, b)),
            PublishOutcome::NotWanted
        );
        // a poll for CLOCK only is accepted and publishes nothing
        assert!(
            r.chain
                .respond(&r.poll(1))
                .is_some_and(|x| x.rpc_result == NV_OK)
        );
        assert_eq!(
            state.publish(&mut |g, b| ram.write(g, b)),
            PublishOutcome::NotWanted
        );
        assert!(ram.writes.is_empty());
        // a poll with every bit set asks for it
        assert!(
            r.chain
                .respond(&r.poll(u64::MAX))
                .is_some_and(|x| x.rpc_result == NV_OK)
        );
        assert_eq!(
            state.publish(&mut |g, b| ram.write(g, b)),
            PublishOutcome::Written,
            "{a}.{b}.{c}"
        );
        let (ts, gpu, mem, eng) = read_record(&ram.mem, l).expect("a consistent record");
        assert_ne!(ts, 0);
        assert_eq!((gpu, mem, eng), (64, 11, [0, 3, 0, 0]), "{a}.{b}.{c}");
        // only perfDevUtil was touched, only inside the page
        let lo = base + l.rusd_off_util;
        let hi = lo + l.util_size;
        assert!(
            ram.writes
                .iter()
                .all(|(g, n)| *g >= lo && g + *n as u64 <= hi),
            "{a}.{b}.{c}: {:?}",
            ram.writes
        );
        let touched: usize = ram.mem.iter().filter(|b| **b != 0).count();
        assert!(touched <= l.util_size as usize, "{touched} non-zero bytes");

        // unlink
        assert!(
            r.chain
                .respond(&r.init(0, 0))
                .is_some_and(|x| x.rpc_result == NV_OK)
        );
        assert_eq!(state.rusd_page(), None);
        let n = ram.writes.len();
        assert_eq!(
            state.publish(&mut |g, b| ram.write(g, b)),
            PublishOutcome::NotWanted
        );
        assert_eq!(ram.writes.len(), n);
    }
}

/// A page the guest named that is not registered RAM makes the writer refuse; nothing else happens.
#[test]
fn rusd_a_page_outside_guest_ram_is_refused_by_the_writer() {
    let mut r = rig(v(580, 159, 4), true);
    let state = r.state.clone().unwrap();
    r.push(5, 2_000_000_000);
    assert!(r.chain.respond(&r.init(1, 0xfee0_0000)).is_some());
    assert!(r.chain.respond(&r.poll(2)).is_some());
    let mut ram = Ram {
        base: 0x10_0000,
        mem: vec![0u8; 4096],
        writes: Vec::new(),
    };
    assert_eq!(
        state.publish(&mut |g, b| ram.write(g, b)),
        PublishOutcome::Refused
    );
    assert!(ram.writes.is_empty());
    assert_eq!(state.counters().publish_refused, 1);
}

/// A new chain is a new guest life: the RUSD link and mask of the old one are gone.
#[test]
fn rusd_a_rebuilt_chain_forgets_the_old_link() {
    let first = rig(v(580, 159, 4), true);
    let state = first.state.clone().unwrap();
    let mut first = first;
    assert!(first.chain.respond(&first.init(1, 0x10_0000)).is_some());
    assert!(first.chain.respond(&first.poll(2)).is_some());
    assert_eq!(state.rusd_page(), Some(0x10_0000));
    // the device rebuilds the chain over the same state (reset, fn-1 re-selection)
    let abi = first.abi;
    let _second = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        abi,
        ChainLogs::default(),
        Default::default(),
        kf_rm::ObjectLinks {
            utilization: Some(UtilizationSeat {
                state: state.clone(),
            }),
            ..Default::default()
        },
    );
    assert_eq!(state.rusd_page(), None);
    assert!(!state.rusd_wants_utilization());
}

// ---------------------------------------------------------------- derived rows, siblings

/// ★ A guest version with no generated row has no link: the controls are refused, even with the property on.
#[test]
fn derived_rows_a_version_without_a_row_gets_no_link() {
    // 575.51.2 has a wire table and a GPUMON/RUSD row would be needed: use a version the table knows and
    // the probe did not (570.86.15: an older INIT struct, recorded as unprobeable).
    let version = v(570, 86, 15);
    assert!(ku::layout(version).is_none());
    let abi = abi(version);
    let state = UtilizationState::new(Arc::new(FakeClock(AtomicU64::new(1_000_000_000))));
    assert!(kf_rm::utilization::UtilizationPolicy::new(abi, &UtilizationSeat { state }).is_none());
    // and through the chain: any shape, refused
    let mut r = rig(version, true);
    let mut p = vec![0u8; 55_888];
    p[0] = 2;
    p[4..8].copy_from_slice(&55_872u32.to_le_bytes());
    assert!(r.chain.respond(&r.control(ku::GPUMON_CMD, &p, 0)).is_none());
    assert!(
        r.chain
            .respond(&r.control(0x2080_0afe, &[0u8; 16], 0))
            .is_none()
    );
    assert!(
        r.chain
            .respond(&r.control(0x2080_0aff, &[0u8; 16], 0))
            .is_none()
    );
    assert_eq!(r.logs.unserviced.total(), 3);
}

/// ★ The derived-row rule (W349REFUSE): the neighbours of the three ids have no derived row and stay refused with
/// the property ON.
#[test]
fn siblings_without_a_derived_row_stay_refused() {
    let mut r = rig(v(580, 159, 4), true);
    r.push(50, 2_000_000_000);
    for (name, cmd, size) in [
        ("PERF_GET_VID_ENG_PERFMON_SAMPLE", 0x2080_2087u32, 64usize),
        ("the GPUMON v1 id", 0x2080_2083, 64),
        ("GSS-legacy 0x2080a097", 0x2080_a097, 64),
        ("BINAPI 0x20810108", 0x2081_0108, 992),
        ("brand caps 0x00800294", 0x0080_0294, 4),
        ("NV00DE REQUEST_DATA_POLL on the GSP", 0x00de_0001, 16),
        ("another INTERNAL control", 0x2080_0a9a, 16),
    ] {
        let c = r.control(cmd, &vec![0u8; size], 0);
        assert!(r.chain.respond(&c).is_none(), "{name} must stay refused");
    }
    assert_eq!(r.logs.unserviced.total(), 7);
    let c = r.state.as_ref().unwrap().counters();
    assert_eq!((c.gpumon_served, c.poll_seen, c.init_linked), (0, 0, 0));
}

/// ★ The matrix agrees with the derivation at every version the chain claims: a link is only built where two
/// independent measurements say the same size.
#[test]
fn derived_rows_the_link_exists_exactly_where_both_measurements_agree() {
    for (versions, l) in ku::ROWS {
        for ver in *versions {
            let state = UtilizationState::new(Arc::new(FakeClock(AtomicU64::new(1))));
            let Ok(table) = kf_abi::versions::table_for(*ver) else {
                continue;
            };
            let link =
                kf_rm::utilization::UtilizationPolicy::new(*table, &UtilizationSeat { state });
            assert_eq!(
                link.is_some(),
                ku::gpumon_at(*ver).is_some() || ku::rusd_at(*ver).is_some()
            );
            assert!(l.matrix_agrees(*ver), "{ver}");
        }
    }
}

// ---------------------------------------------------------------- the weak-mutant additions

/// The cache never holds more than the control's 72 samples, and the reply shows the newest 72 with the tracker
/// at the oldest.
#[test]
fn on_gpumon_after_100_samples_replies_72_with_the_tracker_at_the_oldest() {
    let mut r = rig(v(580, 159, 4), true);
    for i in 0..100u64 {
        r.push(u8::try_from(i % 100).unwrap(), 2_000_000_000 + i);
    }
    let l = r.l();
    let reply = r.chain.respond(&r.gpumon_request(0)).unwrap();
    let p = r.reply_params(&reply, l.params_len());
    let u32_at = |o: u64| u32::from_le_bytes(p[o as usize..o as usize + 4].try_into().unwrap());
    let u64_at = |o: u64| u64::from_le_bytes(p[o as usize..o as usize + 8].try_into().unwrap());
    assert_eq!(u32_at(8), 72, "count");
    assert_eq!(u32_at(12), 0, "tracker 0: the oldest is index 0");
    assert_eq!(
        u64_at(l.off_samples + l.off_timestamp),
        2_000_000_028,
        "the 29th of 100 is the oldest kept"
    );
    assert_eq!(
        u64_at(l.off_samples + 71 * l.sample_size + l.off_timestamp),
        2_000_000_099,
        "the newest is last"
    );
}

/// Two publications at one clock value still carry strictly increasing timestamps (a reader that compares the
/// timestamp before and after a copy must see a change).
#[test]
fn rusd_two_publishes_at_one_clock_value_have_increasing_timestamps() {
    let mut r = rig(v(580, 159, 4), true);
    let l = r.l();
    let state = r.state.clone().unwrap();
    r.push(10, 2_000_000_000);
    let mut ram = Ram {
        base: 0x10_0000,
        mem: vec![0u8; 4096],
        writes: Vec::new(),
    };
    assert!(r.chain.respond(&r.init(1, 0x10_0000)).is_some());
    assert!(r.chain.respond(&r.poll(2)).is_some());
    assert_eq!(
        state.publish(&mut |g, b| ram.write(g, b)),
        PublishOutcome::Written
    );
    let t1 = read_record(&ram.mem, l).unwrap().0;
    assert!(
        r.chain.respond(&r.poll(2)).is_some(),
        "a new poll makes a publication due at once"
    );
    assert_eq!(
        state.publish(&mut |g, b| ram.write(g, b)),
        PublishOutcome::Written
    );
    let t2 = read_record(&ram.mem, l).unwrap().0;
    assert!(t2 > t1, "{t2} must exceed {t1}");
}

/// A later poll that no longer asks for the utilisation record withdraws the wish: nothing is published after it.
#[test]
fn rusd_a_later_poll_without_the_bit_withdraws_the_wish() {
    let mut r = rig(v(580, 159, 4), true);
    let state = r.state.clone().unwrap();
    r.push(10, 2_000_000_000);
    assert!(r.chain.respond(&r.init(1, 0x10_0000)).is_some());
    assert!(r.chain.respond(&r.poll(2)).is_some());
    assert!(state.rusd_wants_utilization());
    assert!(r.chain.respond(&r.poll(0)).is_some());
    assert!(!state.rusd_wants_utilization(), "mask 0 withdraws it");
    assert_eq!(
        state.publish(&mut |_, _| panic!("no write")),
        PublishOutcome::NotWanted
    );
    assert!(r.chain.respond(&r.poll(2)).is_some());
    assert!(r.chain.respond(&r.poll(!2)).is_some());
    assert!(
        !state.rusd_wants_utilization(),
        "a mask without the PERF bit withdraws it too"
    );
}

/// A new chain (reset, fn-1 re-selection) forgets a wish and a publication that was already owed.
#[test]
fn rusd_a_rebuilt_chain_forgets_a_wish_and_an_owed_publication() {
    let mut first = rig(v(580, 159, 4), true);
    let state = first.state.clone().unwrap();
    first.push(10, 2_000_000_000);
    assert!(first.chain.respond(&first.init(1, 0x10_0000)).is_some());
    assert!(first.chain.respond(&first.poll(2)).is_some());
    let abi = first.abi;
    let _second = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        abi,
        ChainLogs::default(),
        Default::default(),
        kf_rm::ObjectLinks {
            utilization: Some(UtilizationSeat {
                state: state.clone(),
            }),
            ..Default::default()
        },
    );
    assert!(!state.rusd_wants_utilization());
    assert_eq!(
        state.publish(&mut |_, _| panic!("no write for a dead guest life")),
        PublishOutcome::NotWanted
    );
}

/// The bucket is per second of the state's clock: a burst, then nothing until the clock moves.
#[test]
fn hostile_a_frozen_clock_serves_one_burst_and_a_moving_one_the_rate() {
    let mut r = rig(v(595, 84, 0), true);
    r.push(10, 2_000_000_000);
    let req = r.gpumon_request(0);
    let first = (0..64).filter(|_| r.chain.respond(&req).is_some()).count() as u64;
    assert_eq!(first, GPUMON_REPLIES_PER_SEC);
    // 100 ms later: one more token at most (16 per second)
    r.clock.0.fetch_add(100_000_000, Ordering::Relaxed);
    r.state.as_ref().unwrap().push(GuestSample {
        timestamp_ns: r.clock.0.load(Ordering::Relaxed),
        percents: Percents::default(),
    });
    let next = (0..64).filter(|_| r.chain.respond(&req).is_some()).count() as u64;
    assert_eq!(next, 1, "100 ms at 16/s is one token");
}

// ---------------------------------------------------------------- security review of PR #459 (F1, F3)

/// ★ F1, replayed: boot A seats the link, then the device resets and rebuilds the chain for a driver version that
/// has a table but NO layout row. The rebuild must leave no link: no write is ever issued into the memory of the
/// next guest life, whatever is sampled.
#[test]
fn rusd_a_reset_into_a_version_without_a_row_leaves_no_link_and_issues_no_write() {
    let mut a = rig(v(580, 159, 4), true);
    let state = a.state.clone().unwrap();
    a.push(70, 2_000_000_000);
    assert!(a.chain.respond(&a.init(1, 0x10_0000)).is_some());
    assert!(a.chain.respond(&a.poll(2)).is_some());
    assert_eq!(state.rusd_page(), Some(0x10_0000));
    // the reset: a fresh chain for a version the probe has no row for (570.86.15: an older INIT struct)
    let no_row = v(570, 86, 15);
    assert!(ku::layout(no_row).is_none());
    let _boot_b = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        abi(no_row),
        ChainLogs::default(),
        Default::default(),
        kf_rm::ObjectLinks {
            utilization: Some(UtilizationSeat {
                state: state.clone(),
            }),
            ..Default::default()
        },
    );
    assert_eq!(state.rusd_page(), None, "the link is gone");
    assert!(!state.rusd_wants_utilization());
    for step in 0..4u64 {
        a.clock.0.fetch_add(300_000_000, Ordering::Relaxed);
        state.push(GuestSample {
            timestamp_ns: a.clock.0.load(Ordering::Relaxed),
            percents: Percents::default(),
        });
        assert_eq!(
            state.publish(&mut |g, _| panic!(
                "a write into the next guest life at {g:#x} (step {step})"
            )),
            PublishOutcome::NotWanted
        );
    }
}

/// ★ F3: when the host stops answering, the guest's record does not keep a valid timestamp and a frozen number:
/// it is invalidated (timestamp 0, the header's "not available"), once, and the reader sees no record.
#[test]
fn rusd_a_host_that_stops_answering_ages_the_record_out_visibly() {
    let mut r = rig(v(580, 159, 4), true);
    let l = r.l();
    let state = r.state.clone().unwrap();
    let mut ram = Ram {
        base: 0x10_0000,
        mem: vec![0u8; 4096],
        writes: Vec::new(),
    };
    r.push(97, 2_000_000_000);
    assert!(r.chain.respond(&r.init(1, 0x10_0000)).is_some());
    assert!(r.chain.respond(&r.poll(2)).is_some());
    assert_eq!(
        state.publish(&mut |g, b| ram.write(g, b)),
        PublishOutcome::Written
    );
    assert_eq!(read_record(&ram.mem, l).unwrap().1, 97);
    // the host goes quiet: no new sample for longer than the freshness window
    r.clock.0.fetch_add(4_000_000_000, Ordering::Relaxed);
    assert_eq!(
        state.publish(&mut |g, b| ram.write(g, b)),
        PublishOutcome::Invalidated
    );
    assert!(
        read_record(&ram.mem, l).is_none(),
        "the frozen 97 % is no longer readable"
    );
    // and it stays quiet: nothing more is written
    r.clock.0.fetch_add(1_000_000_000, Ordering::Relaxed);
    let n = ram.writes.len();
    assert_eq!(
        state.publish(&mut |g, b| ram.write(g, b)),
        PublishOutcome::NoSample
    );
    assert_eq!(ram.writes.len(), n);
    // when samples return, the record returns
    r.push(12, 8_000_000_000);
    assert_eq!(
        state.publish(&mut |g, b| ram.write(g, b)),
        PublishOutcome::Written
    );
    assert_eq!(read_record(&ram.mem, l).unwrap().1, 12);
}

/// Forgetting the guest (what the device does at a reset) stops the publication of a linked, polled page.
#[test]
fn rusd_forgetting_the_guest_stops_the_publication() {
    let mut r = rig(v(595, 84, 0), true);
    let state = r.state.clone().unwrap();
    r.push(33, 2_000_000_000);
    assert!(r.chain.respond(&r.init(1, 0x10_0000)).is_some());
    assert!(r.chain.respond(&r.poll(2)).is_some());
    state.forget_guest();
    assert_eq!(state.rusd_page(), None);
    assert_eq!(
        state.publish(&mut |_, _| panic!("no write")),
        PublishOutcome::NotWanted
    );
}

// ---------------------------------------------------------------- security review 2 of PR #459 (R1)

/// The device's reset, as `reset_drainer_state` does it: a fresh chain for the realize-time table over the same
/// state, THEN `forget_guest` (the order the device has).
fn reset_to(state: &Arc<UtilizationState>, version: DriverVersion) -> Box<dyn CommandPolicy> {
    let chain = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        abi(version),
        ChainLogs::default(),
        Default::default(),
        kf_rm::ObjectLinks {
            utilization: Some(UtilizationSeat {
                state: state.clone(),
            }),
            ..Default::default()
        },
    );
    state.forget_guest();
    chain
}

/// ★ R1, replayed: boot -> record written; reboot (the rebuilt chain is for the SAME table, which is every
/// `guest-driver`-declared VM and every VM whose guest driver equals the host's) -> the record is written again,
/// every time; reboot into a driver with no layout row -> no link and no write; reboot back -> written again.
#[test]
fn rusd_reboots_keep_publishing_and_an_unknown_driver_publishes_nothing() {
    for (a, b, c) in [(580, 159, 4), (595, 84, 0)] {
        let version = v(a, b, c);
        let mut life = rig(version, true);
        let l = life.l();
        let state = life.state.clone().unwrap();
        let mut ram = Ram {
            base: 0x10_0000,
            mem: vec![0u8; 4096],
            writes: Vec::new(),
        };
        let mut serve =
            |life: &mut Rig, state: &Arc<UtilizationState>, ram: &mut Ram, gpu: u8, at: u64| {
                life.push(gpu, at);
                assert!(
                    life.chain
                        .respond(&life.init(1, 0x10_0000))
                        .is_some_and(|r| r.rpc_result == NV_OK)
                );
                assert!(
                    life.chain
                        .respond(&life.poll(2))
                        .is_some_and(|r| r.rpc_result == NV_OK)
                );
                assert_eq!(
                    state.publish(&mut |g, d| ram.write(g, d)),
                    PublishOutcome::Written,
                    "{a}.{b}.{c}"
                );
                assert_eq!(read_record(&ram.mem, l).unwrap().1, u32::from(gpu));
            };
        // boot A
        serve(&mut life, &state, &mut ram, 41, 2_000_000_000);
        // three reboots at the same table
        for (i, gpu) in [52u8, 63, 74].into_iter().enumerate() {
            ram.mem.fill(0);
            life.chain = reset_to(&state, version);
            assert_eq!(
                state.rusd_page(),
                None,
                "the dead life's link is gone after reboot {i}"
            );
            serve(
                &mut life,
                &state,
                &mut ram,
                gpu,
                3_000_000_000 + i as u64 * 1_000_000_000,
            );
        }
        // a reboot into a driver with no layout row: no link, no write, whatever is sampled
        let no_row = v(570, 86, 15);
        assert!(ku::layout(no_row).is_none());
        let _unknown = reset_to(&state, no_row);
        assert_eq!(state.rusd_page(), None);
        for k in 0..3u64 {
            life.clock.0.fetch_add(300_000_000, Ordering::Relaxed);
            state.push(GuestSample {
                timestamp_ns: life.clock.0.load(Ordering::Relaxed),
                percents: Percents::default(),
            });
            assert_eq!(
                state.publish(&mut |g, _| panic!(
                    "write at {g:#x} into an unknown-driver life (step {k})"
                )),
                PublishOutcome::NotWanted
            );
        }
        // and back to a known driver: the record returns
        ram.mem.fill(0);
        life.chain = reset_to(&state, version);
        serve(&mut life, &state, &mut ram, 85, 9_000_000_000);
    }
}

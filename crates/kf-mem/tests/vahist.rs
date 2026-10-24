//! ★ DIAGNOSTIC ONLY — `KF3_DIAG_VAHIST` (`kf_mem::vahist`): bounds and format, GPU-free.
//!
//! What these pin (enscape-08 diagnostic, 2026-10-24):
//! - the trigger parser takes the host's own Xid text (and a `va=… chid=…` line), and nothing
//!   else; hostile input never panics and is read only up to [`TRIGGER_BYTES`];
//! - the per-VAS rings are bounded whatever the guest does ([`LAST_RUNS`], [`ABSENCE_EVENTS`]);
//!   VA-contiguous runs of one walk coalesce (a 4144-run map is ONE entry);
//! - the lookup's verdicts: PRESENT / UNMAPPED / REFUSED / WITHHELD / NOT_NAMED, and the previous
//!   process's retired space (`RETIRED_MAPPED`) for the host-channel-reuse question;
//! - every emitted line is prefixed, at most [`LINE_MAX`] bytes, and the dumps are capped
//!   ([`MANUAL_DUMPS`], [`AUTO_DUMPS`], [`CHAN_LINES_MAX`]).

use kf_mem::apply::DiffRun;
use kf_mem::vahist::{
    ABSENCE_EVENTS, AUTO_DUMPS, CHAN_LINES_MAX, ChanBirth, LAST_RUNS, LINE_MAX, MANUAL_DUMPS,
    Recorder, SpaceView, TRIGGER_BYTES, TRIGGER_POLL_MS, Trigger, flag_on, parse_trigger,
    probe_pte,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const ACK_FAILED: u8 = 0;
const ACK_APPLIED: u8 = 1;
const ACK_HELD: u8 = 2;

fn run(unmap: bool, va: u64, len: u64) -> DiffRun {
    DiffRun {
        unmap,
        va,
        len,
        at: 0x1000_0000 + va,
        ap: 0,
        held: false,
        kind: 0,
        perm: kf_host::MapPerm::default(),
        privileged: false,
        leaf: 0x1000,
    }
}

/// A view with fixed rows per key (`(start, len)`), no guest tables.
#[derive(Default)]
struct View {
    rows: HashMap<u64, Vec<(u64, u64)>>,
}

impl SpaceView for View {
    fn live_keys(&self) -> Vec<u64> {
        let mut k: Vec<u64> = self.rows.keys().copied().collect();
        k.sort_unstable();
        k
    }
    fn row_at(&self, key: u64, va: u64) -> Option<(u64, u64, bool, u64)> {
        self.rows
            .get(&key)?
            .iter()
            .find(|(s, l)| va >= *s && va < s + l)
            .map(|&(s, l)| (s, l, true, 0x4000))
    }
    fn guest_pte(&self, _key: u64, _va: u64) -> Option<String> {
        None
    }
}

const K07: u64 = (0xc1d0_0276u64 << 32) | 0xff00_0850;
const K08: u64 = (0xc1d0_028cu64 << 32) | 0xff00_0850;

fn birth(r: &Recorder, client: u32, handle: u32, host_token: u32, vas: u64) -> Option<String> {
    r.chan_birth(ChanBirth {
        client,
        handle,
        guest_token: 0x3d,
        host_token,
        pid: Some(0x325c),
        engine: 1,
        gp_entries: 32768,
        gpfifo_va: 0x722b000,
        vas,
    })
}

fn all_prefixed(lines: &[String]) {
    for l in lines {
        assert!(
            l.starts_with("kf3: VAHIST ") || l.starts_with("kf3: CHAN_LIFE "),
            "{l}"
        );
        assert!(
            l.len() <= LINE_MAX,
            "line over {LINE_MAX} bytes: {}",
            l.len()
        );
    }
}

#[test]
fn the_flag_is_on_only_for_exactly_one() {
    assert!(flag_on(Some("1")));
    for v in [
        None,
        Some(""),
        Some("0"),
        Some("yes"),
        Some("true"),
        Some(" 1"),
        Some("11"),
    ] {
        assert!(!flag_on(v), "{v:?}");
    }
}

#[test]
fn the_trigger_parser_takes_the_hosts_own_xid_text() {
    let xid = b"2026-10-24T09:00:46.9+02:00 claude kernel: NVRM: Xid (PCI:0000:01:00): 31, pid=3716136, name=qemu-system-x86, channel 0x00000067, MMU Fault: ENGINE CE0 HUBCLIENT_CE1 faulted @ 0xb_5a225000. Fault is of type FAULT_PTE ACCESS_TYPE_VIRT_WRITE";
    assert_eq!(
        parse_trigger(xid),
        Some(Trigger {
            va: 0xb_5a22_5000,
            chid: Some(0x67)
        })
    );
    assert_eq!(
        parse_trigger(b"va=0x2_f9e87000 chid=0x6b\n"),
        Some(Trigger {
            va: 0x2_f9e8_7000,
            chid: Some(0x6b)
        })
    );
    assert_eq!(
        parse_trigger(b"va=0xa2fbda000"),
        Some(Trigger {
            va: 0xa_2fbd_a000,
            chid: None
        })
    );
}

#[test]
fn the_trigger_parser_refuses_hostile_or_partial_input_without_panicking() {
    assert_eq!(parse_trigger(b""), None);
    assert_eq!(parse_trigger(b"channel 0x67 but no address"), None);
    assert_eq!(parse_trigger(b"va=0x"), None);
    assert_eq!(parse_trigger(b"va=0xzz"), None);
    // More than 64 bits of hex is refused, never wrapped.
    assert_eq!(parse_trigger(b"va=0x1_0000_0000_0000_0000"), None);
    // Only the first TRIGGER_BYTES are looked at: an address past them is not found.
    let mut long = vec![b'x'; TRIGGER_BYTES];
    long.extend_from_slice(b" va=0x1000");
    assert_eq!(parse_trigger(&long), None);
    // Arbitrary bytes.
    let junk: Vec<u8> = (0..4096u32).map(|i| (i * 131 % 251) as u8).collect();
    let _ = parse_trigger(&junk);
}

#[test]
fn contiguous_runs_of_one_walk_coalesce_into_one_entry() {
    let r = Recorder::new();
    r.note_space(K08, 0xcaff_4b20, true);
    let runs: Vec<DiffRun> = (0..4144u64)
        .map(|i| run(false, 0xa_0000_0000 + i * 0x1000, 0x1000))
        .collect();
    let codes = vec![ACK_APPLIED; runs.len()];
    assert!(!r.record_apply(K08, 7, &runs, &codes, true, 0));
    let s = r.space_stats(K08).expect("tracked");
    assert_eq!(s.last_len, 1, "one coalesced MAP entry");
    let unmaps: Vec<DiffRun> = runs.iter().map(|d| run(true, d.va, d.len)).collect();
    assert!(!r.record_apply(K08, 8, &unmaps, &codes, true, 0));
    let s = r.space_stats(K08).expect("tracked");
    assert_eq!(s.last_len, 2);
    assert_eq!(s.absent_len, 1, "one coalesced UNMAP entry");
}

#[test]
fn the_rings_are_bounded_whatever_the_guest_maps() {
    let r = Recorder::new();
    r.note_space(K08, 1, false);
    // 100 000 scattered runs (never contiguous), unmapped one by one.
    for chunk in 0..100u64 {
        let runs: Vec<DiffRun> = (0..1000u64)
            .map(|i| run(true, (chunk * 1000 + i) * 0x3000, 0x1000))
            .collect();
        let codes = vec![ACK_APPLIED; runs.len()];
        r.record_apply(K08, chunk, &runs, &codes, true, 0);
    }
    let s = r.space_stats(K08).expect("tracked");
    assert_eq!(s.last_len, LAST_RUNS);
    assert_eq!(s.absent_len, ABSENCE_EVENTS);
    assert_eq!(s.absent_evicted, 100_000 - ABSENCE_EVENTS as u64);
}

#[test]
fn the_lookup_says_present_when_our_row_covers_the_va() {
    let r = Recorder::new();
    r.note_space(K08, 0xcaff_4b20, true);
    r.record_apply(
        K08,
        3,
        &[run(false, 0xb_5a20_0000, 0x40000)],
        &[ACK_APPLIED],
        true,
        0,
    );
    assert!(birth(&r, 0xc1d0_029b, 0xff04_0039, 0x67, K08).is_some());
    let mut v = View::default();
    v.rows.insert(K08, vec![(0xb_5a20_0000, 0x40000)]);
    let out = r.lookup(&parse_trigger(b"va=0xb5a225000 chid=0x67").unwrap(), &v);
    all_prefixed(&out);
    let vas = out
        .iter()
        .find(|l| l.contains(" VAS "))
        .expect("a VAS line");
    assert!(vas.contains("verdict=PRESENT"), "{vas}");
    assert!(vas.contains("role=chan"), "{vas}");
    assert!(
        out.iter()
            .any(|l| l.contains(" CHAN ") && l.contains("host_token=0x67"))
    );
    assert!(
        out.iter()
            .any(|l| l.contains(" EV ") && l.contains("op=MAP"))
    );
    assert!(out.first().unwrap().contains(" LOOKUP #1 "));
    assert!(out.last().unwrap().contains(" END #1 "));
}

#[test]
fn the_lookup_says_unmapped_refused_or_withheld_from_the_last_event_covering_the_va() {
    let r = Recorder::new();
    r.note_space(K08, 2, true);
    let base = 0xa_2fbd_0000u64;
    // mapped, then unmapped (the guest's own UNMAP, acked)
    r.record_apply(
        K08,
        1,
        &[run(false, base, 0x10000)],
        &[ACK_APPLIED],
        true,
        0,
    );
    r.record_apply(K08, 2, &[run(true, base, 0x10000)], &[ACK_APPLIED], true, 0);
    // a refused MAP elsewhere, a withheld privileged MAP elsewhere, a HELD one
    assert!(r.record_apply(
        K08,
        3,
        &[run(false, base + 0x100000, 0x1000)],
        &[ACK_FAILED],
        true,
        0
    ));
    let mut p = run(false, base + 0x200000, 0x1000);
    p.privileged = true;
    assert!(
        !r.record_apply(K08, 4, &[p], &[ACK_FAILED], true, 0),
        "withheld is not a refusal"
    );
    r.record_apply(
        K08,
        5,
        &[run(false, base + 0x300000, 0x1000)],
        &[ACK_HELD],
        true,
        0,
    );
    let v = View::default();
    let verdict = |va: u64| {
        let out = r.lookup(&Trigger { va, chid: None }, &v);
        all_prefixed(&out);
        out.iter()
            .find(|l| l.contains(" VAS "))
            .cloned()
            .unwrap_or_default()
    };
    assert!(verdict(base + 0xa000).contains("verdict=UNMAPPED"));
    assert!(verdict(base + 0x100000).contains("verdict=REFUSED"));
    assert!(verdict(base + 0x200000).contains("verdict=WITHHELD"));
    assert!(verdict(base + 0x300000).contains("verdict=HELD"));
    // a VA nobody named in any tracked space: one line saying so (a fresh recorder: the four
    // lookups above used this one's MANUAL_DUMPS)
    let r = Recorder::new();
    r.note_space(K08, 2, true);
    r.record_apply(
        K08,
        1,
        &[run(false, base, 0x10000)],
        &[ACK_APPLIED],
        true,
        0,
    );
    let out = r.lookup(
        &Trigger {
            va: 0x5_0000_0000,
            chid: None,
        },
        &v,
    );
    assert!(
        out.iter().any(|l| l.contains("verdict=NOT_NAMED")),
        "{out:?}"
    );
}

#[test]
fn the_lookup_names_the_previous_owner_of_a_reused_host_channel_and_its_retired_space() {
    let r = Recorder::new();
    // process 07: its channel on host token 0x67, its space maps the VA, then everything goes
    r.note_space(K07, 0xcaff_4b20, true);
    assert!(birth(&r, 0xc1d0_0284, 0xff04_0039, 0x67, K07).is_some());
    r.chan_class(0xc1d0_0284, 0xff04_0039, 0xc7b5);
    r.record_apply(
        K07,
        1,
        &[run(false, 0xb_5a00_0000, 0x400000)],
        &[ACK_APPLIED],
        true,
        0,
    );
    let free = r
        .chan_free(0xc1d0_0284, 0xff04_0039, true)
        .expect("a free line");
    assert!(free.contains("classes=[0xc7b5]"), "{free}");
    r.retire(K07, vec![(0xb_5a00_0000, 0x400000)], 0);
    // process 08: same host token, recycled host space, the VA never named
    r.note_space(K08, 0xcaff_4b20, true);
    assert!(birth(&r, 0xc1d0_029b, 0xff04_0039, 0x67, K08).is_some());
    let mut v = View::default();
    v.rows.insert(K08, vec![]);
    let out = r.lookup(
        &Trigger {
            va: 0xb_5a22_5000,
            chid: Some(0x67),
        },
        &v,
    );
    all_prefixed(&out);
    let chans: Vec<&String> = out.iter().filter(|l| l.contains(" CHAN ")).collect();
    assert_eq!(chans.len(), 2, "{out:?}");
    assert!(chans[0].contains("(0xc1d0029b,0xff040039)") && chans[0].contains("alive"));
    assert!(chans[1].contains("(0xc1d00284,0xff040039)") && chans[1].contains("freed_ms="));
    let vas: Vec<&String> = out.iter().filter(|l| l.contains(" VAS ")).collect();
    assert!(
        vas[0].contains("verdict=NOT_NAMED") && vas[0].contains("role=chan"),
        "{vas:?}"
    );
    assert!(
        vas.iter()
            .any(|l| l.contains("verdict=RETIRED_MAPPED") && l.contains("(0xc1d00276,0xff000850)")),
        "{vas:?}"
    );
}

#[test]
fn manual_dumps_are_capped_and_each_is_bounded() {
    let r = Recorder::new();
    for k in 0..300u64 {
        let key = (0xc1d0_1000u64 + k) << 32 | 0xff00_0850;
        r.note_space(key, k, false);
        let runs: Vec<DiffRun> = (0..200u64)
            .map(|i| run(i % 2 == 0, 0x1_0000_0000 + i * 0x2000, 0x1000))
            .collect();
        r.record_apply(key, k, &runs, &[ACK_APPLIED; 200], true, 0);
        assert!(birth(&r, 0xc1d0_1000 + k as u32, 0xff04_0039, 0x67, key).is_some());
    }
    let v = View::default();
    let t = Trigger {
        va: 0x1_0000_0000,
        chid: Some(0x67),
    };
    let mut n = 0;
    for _ in 0..MANUAL_DUMPS {
        let out = r.lookup(&t, &v);
        all_prefixed(&out);
        assert!(out.len() <= 1 + 8 + 8 * 10 + LAST_RUNS + 1, "{}", out.len());
        n += 1;
    }
    assert_eq!(n, MANUAL_DUMPS);
    let over = r.lookup(&t, &v);
    assert_eq!(over.len(), 1, "a capped dump says so in one line: {over:?}");
    assert!(over[0].contains("capped"));
}

#[test]
fn an_auto_dump_fires_once_per_space_on_a_refusal_and_is_capped() {
    let r = Recorder::new();
    let mut dumped = 0;
    for k in 0..(AUTO_DUMPS as u64 + 10) {
        let key = (0xc1d0_2000u64 + k) << 32 | 0xff00_0850;
        r.note_space(key, k, false);
        assert!(r.record_apply(
            key,
            1,
            &[run(false, 0x1000, 0x1000)],
            &[ACK_FAILED],
            true,
            0
        ));
        let out = r.auto_dump(key, "refused map");
        all_prefixed(&out);
        if !out.is_empty() {
            dumped += 1;
            assert!(out[0].contains(" AUTO "), "{out:?}");
            assert!(out.iter().any(|l| l.contains("op=REFUSED")));
            assert!(r.auto_dump(key, "again").is_empty(), "once per space");
        }
    }
    assert_eq!(dumped, AUTO_DUMPS);
}

#[test]
fn a_walk_refusal_is_recorded_without_a_va() {
    let r = Recorder::new();
    r.note_space(K08, 1, false);
    assert!(r.record_apply(K08, 9, &[], &[], true, 0x400));
    let out = r.auto_dump(K08, "walk refused");
    assert!(
        out.iter()
            .any(|l| l.contains("op=WALK_REFUSED") && l.contains("mask=0x400")),
        "{out:?}"
    );
}

#[test]
fn channel_lines_have_one_fixed_format_and_are_capped() {
    let r = Recorder::new();
    let b = birth(&r, 0xc1d0_029b, 0xff04_0039, 0x67, K08).expect("birth line");
    assert!(b.starts_with("kf3: CHAN_LIFE birth #1 t_ms="), "{b}");
    for f in [
        "key=(0xc1d0029b,0xff040039)",
        "guest_tok=0x3d",
        "host_token=0x67",
        "pid=0x325c",
        "engine=0x1",
        "gp=32768",
        "gpfifo=0x722b000",
        "vas=(0xc1d0028c,0xff000850)",
    ] {
        assert!(b.contains(f), "{f} in {b}");
    }
    for c in [
        0xc7b5, 0xa140, 0x902d, 0xc997, 0xc9c0, 0xc7b5, 1, 2, 3, 4, 5, 6,
    ] {
        r.chan_class(0xc1d0_029b, 0xff04_0039, c);
    }
    let f = r
        .chan_free(0xc1d0_029b, 0xff04_0039, true)
        .expect("free line");
    assert!(f.starts_with("kf3: CHAN_LIFE free #2 t_ms="), "{f}");
    assert!(f.contains("lived_ms=") && f.contains("ok=true") && f.contains("host_token=0x67"));
    assert!(
        f.contains("classes=[0xc7b5,0xa140,0x902d,0xc997,0xc9c0,0x1,0x2,0x3]"),
        "deduplicated, at most 8: {f}"
    );
    assert!(
        r.chan_free(0xc1d0_029b, 0xff04_0039, true).is_none(),
        "one free per birth"
    );
    assert!(
        r.chan_free(0x1, 0x2, true).is_none(),
        "no line for a channel never recorded"
    );
    let mut lines = 2usize;
    for i in 0..(CHAN_LINES_MAX as u32) {
        if birth(&r, i, 1, 0x67, K08).is_some() {
            lines += 1;
        }
        if r.chan_free(i, 1, true).is_some() {
            lines += 1;
        }
    }
    assert_eq!(lines, CHAN_LINES_MAX, "births and frees share one cap");
}

#[test]
fn the_trigger_file_fires_once_per_new_content_rate_limited() {
    let dir = std::env::temp_dir().join(format!("kf-vahist-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("trigger");
    let _ = std::fs::remove_file(&path);
    let r = Recorder::new();
    let t0 = Instant::now();
    assert!(r.poll_trigger(&path, t0).is_none(), "no file");
    std::fs::write(&path, b"va=0x1000 chid=0x67").unwrap();
    assert!(
        r.poll_trigger(&path, t0 + Duration::from_millis(1))
            .is_none(),
        "rate limited"
    );
    let step = Duration::from_millis(TRIGGER_POLL_MS + 1);
    let got = r.poll_trigger(&path, t0 + step).expect("fires");
    assert_eq!(
        got,
        Ok(Trigger {
            va: 0x1000,
            chid: Some(0x67)
        })
    );
    assert!(
        r.poll_trigger(&path, t0 + step * 2).is_none(),
        "same content"
    );
    std::fs::write(&path, b"garbage").unwrap();
    assert!(matches!(r.poll_trigger(&path, t0 + step * 3), Some(Err(_))));
    std::fs::write(&path, b"va=0x2000").unwrap();
    assert!(matches!(r.poll_trigger(&path, t0 + step * 4), Some(Ok(_))));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A small VER2 space in a fake store: PD3 → PD2 → PD1 → PD0 (dual) → small PT, one valid 4 KiB
/// PTE at `va`; the probe prints the raw words on the path and decodes the leaf.
#[test]
fn the_guest_pte_probe_walks_the_descriptor_and_prints_raw_words() {
    let fmt = kf_cuda::abi::kf_format_ver2();
    let va: u64 = 0xb_5a22_5000;
    let mut mem: HashMap<u64, u64> = HashMap::new();
    // vidmem PDE: aperture 1 (VID) at bits 2:1, address >> 12 at bits 8.. (addr_local lo=8)
    let pde = |addr: u64| (1u64 << 1) | ((addr >> 12) << 8);
    let (pd3, pd2, pd1, pd0, pt) = (0x10_0000u64, 0x20_0000, 0x30_0000, 0x40_0000, 0x50_0000);
    let idx = |k: usize| (va >> fmt.dir[k].va_lo) & (u64::from(fmt.dir[k].entries) - 1);
    mem.insert(pd3 + idx(1) * 8, pde(pd2));
    mem.insert(pd2 + idx(2) * 8, pde(pd1));
    mem.insert(pd1 + idx(3) * 8, pde(pd0));
    // dual entry: big half invalid (0), small half a vidmem PDE to `pt`
    mem.insert(pd0 + idx(4) * 16, 0);
    mem.insert(pd0 + idx(4) * 16 + 8, pde(pt));
    let si = (va >> fmt.small_va_lo) & (u64::from(fmt.small_entries) - 1);
    let pte = 1u64 | ((0x1234_5000u64 >> 12) << 8);
    mem.insert(pt + si * 8, pte);
    let mut reads = 0;
    let mut read8 = |a: u64| {
        reads += 1;
        Some(mem.get(&a).copied().unwrap_or(0))
    };
    let s = probe_pte(&fmt, pd3, va, &mut read8);
    assert!(reads <= 9, "bounded reads: {reads}");
    assert!(
        s.contains(&format!("small@{:#x}={pte:#x}", pt + si * 8)),
        "{s}"
    );
    assert!(s.contains("small_valid=1"), "{s}");
    assert!(s.contains("big_valid=none"), "{s}");
    // An invalid PDE ends the path, by name.
    let mut empty = |_a: u64| Some(0u64);
    let s = probe_pte(&fmt, pd3, va, &mut empty);
    assert!(s.contains("path ends"), "{s}");
    // A refused read ends it too.
    let mut refused = |_a: u64| None;
    assert!(probe_pte(&fmt, pd3, va, &mut refused).contains("read refused"));
}

/// ★ Found on the first hardware run (5220, 2026-10-24): a user space's privileged leaves are
/// re-emitted WITHHELD by every walk, and they flooded the last-runs ring. A re-emitted refused or
/// withheld run must not push real history out: it is counted in its de-duplicated ring only.
#[test]
fn re_emitted_withheld_and_refused_runs_do_not_flood_the_last_runs() {
    let r = Recorder::new();
    r.note_space(K08, 1, true);
    r.record_apply(
        K08,
        1,
        &[run(false, 0xb_5a20_0000, 0x40000)],
        &[ACK_APPLIED],
        true,
        0,
    );
    let mut p = run(false, 0x1_2400_0000, 0x25f_0000);
    p.privileged = true;
    for w in 2..500u64 {
        let refused = run(false, 0x7_0000_0000, 0x1000);
        r.record_apply(K08, w, &[p, refused], &[ACK_FAILED, ACK_FAILED], true, 0);
    }
    let s = r.space_stats(K08).expect("tracked");
    assert_eq!(
        s.last_len, 3,
        "MAP + first WITHHELD + first REFUSED only: {s:?}"
    );
    let out = r.auto_dump(K08, "test");
    assert!(
        out.iter()
            .any(|l| l.contains(" LAST ") && l.contains("op=MAP") && l.contains("va=0xb5a200000")),
        "{out:?}"
    );
    assert!(
        out.iter()
            .any(|l| l.contains("op=REFUSED") && l.contains("seen=498")),
        "{out:?}"
    );
}

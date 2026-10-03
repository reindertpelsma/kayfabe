// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ The boot display in fn 65 (`docs/design/V3_DISPLAY.md` §4.11.4 rows 1–2): the guest's
//! `consoleMemSize`, kept from fn 72 and decoded at fn 65 with the table that serves fn 65, becomes
//! region 0 — and with no console, or with `gop=off`, the reply is byte-identical to the one this
//! policy served before the seat existed.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::DriverVersion;
use kf_abi::guestsysinfo::SET_GUEST_SYSTEM_INFO_SIZE;
use kf_abi::matrix::Resolved;
use kf_abi::versions::{BENCH_DRIVER, DriverAbiTable, pre_fn1_surface_differs, table_for};
use kf_chip::bar0::{ConsoleRefused, fb_layout_with_console};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction, SystemInfoCell};
use kf_rm::staticinfo::{ConsoleRefusal, ConsoleSeat, StaticInfoPolicy, console_mem_size};
use kf_rm::{GuestDriverSource, ReselectAtFn1, Reselection};

/// 1920x1080 XRGB8888 at a 256-byte pitch, rounded to 64 KiB (`kf_oprom::Geometry::for_mode`).
const C_1080P: u64 = 0x7F_0000;
/// The guest BAR1 aperture the fixture board advertises (`ga106::pci_bars`).
const BAR1: u64 = 256 << 20;

fn table(v: DriverVersion) -> DriverAbiTable {
    *table_for(v).expect("a version in the matrix")
}

fn v(major: u16, minor: u16, patch: u16) -> DriverVersion {
    DriverVersion {
        major,
        minor,
        patch,
    }
}

/// A `GspSystemInfo` body at `version`'s size in the driver matrix, `consoleMemSize = console` at its
/// offset there, and a marker everywhere else (so a read at another version's offset is not zero).
fn fn72_body(version: DriverVersion, console: u64) -> Vec<u8> {
    let l =
        Resolved::of(&kf_abi::generated::matrix::GSPSYSTEMINFO, version).expect("in the matrix");
    let mut b = vec![0x5Au8; l.size()];
    let at = l.need("consoleMemSize").expect("the field").off();
    b[at..at + 8].copy_from_slice(&console.to_le_bytes());
    b
}

fn seat(cell: &SystemInfoCell, gop: bool) -> ConsoleSeat {
    ConsoleSeat {
        system_info: cell.clone(),
        bar1_bytes: BAR1,
        boot_fb: gop.then_some(C_1080P),
    }
}

fn fn65(version: DriverVersion) -> RpcCommand {
    let size = Resolved::of(&kf_abi::generated::matrix::GSPSTATICCONFIGINFO, version)
        .expect("in the matrix")
        .size();
    RpcCommand {
        function: RpcFunction::GetGspStaticInfo,
        code: 65,
        sequence: 2,
        payload: vec![0; size],
        elements: 1,
        delivered: Vec::new(),
    }
}

fn answer(p: &mut dyn CommandPolicy, version: DriverVersion) -> Reply {
    p.respond(&fn65(version)).expect("fn 65 is always answered")
}

fn plain(version: DriverVersion) -> StaticInfoPolicy {
    StaticInfoPolicy::new(ga106::board(), table(version))
}

/// What fn 65 must serve with a console of `c` bytes on the fixture board.
fn carved(version: DriverVersion, c: u64) -> Vec<u8> {
    let l = fb_layout_with_console(ga106::board().fb_length, c).expect("carvable");
    plain(version)
        .body_measured_with(&l.regions)
        .expect("encodes")
}

/// ⊘ The default must not move: no seat, no fn 72 kept, `C = 0`, and `gop=off` with any `C` all
/// serve the reply this policy served before the boot display existed, byte for byte.
#[test]
fn without_a_console_or_with_gop_off_the_reply_is_byte_identical() {
    let today = answer(&mut plain(BENCH_DRIVER), BENCH_DRIVER);
    assert_eq!(today.rpc_result, 0);
    let cell = SystemInfoCell::new();
    // a seat, but no fn 72 seen yet
    for gop in [false, true] {
        let mut p = plain(BENCH_DRIVER).with_console(seat(&cell, gop));
        assert_eq!(answer(&mut p, BENCH_DRIVER), today, "no fn 72, gop={gop}");
    }
    // C = 0, either posture
    cell.store(&fn72_body(BENCH_DRIVER, 0), 1);
    for gop in [false, true] {
        let mut p = plain(BENCH_DRIVER).with_console(seat(&cell, gop));
        assert_eq!(answer(&mut p, BENCH_DRIVER), today, "C = 0, gop={gop}");
    }
    // gop=off: a console is read and logged, never acted on — and nothing it says can refuse
    for c in [C_1080P, 0x1234, BAR1 * 2] {
        cell.store(&fn72_body(BENCH_DRIVER, c), 2);
        let mut p = plain(BENCH_DRIVER).with_console(seat(&cell, false));
        assert_eq!(answer(&mut p, BENCH_DRIVER), today, "gop=off, C = {c:#x}");
    }
    // gop=off and a body of another version's size: still today's reply
    cell.store(&[0u8; 100], 3);
    let mut p = plain(BENCH_DRIVER).with_console(seat(&cell, false));
    assert_eq!(answer(&mut p, BENCH_DRIVER), today);
}

#[test]
fn gop_on_with_a_console_serves_it_as_region_zero() {
    let cell = SystemInfoCell::new();
    cell.store(&fn72_body(BENCH_DRIVER, C_1080P), 1);
    let mut p = plain(BENCH_DRIVER).with_console(seat(&cell, true));
    let r = answer(&mut p, BENCH_DRIVER);
    assert_eq!(r.rpc_result, 0);
    assert_eq!(r.body, carved(BENCH_DRIVER, C_1080P));
    assert_ne!(r.body, answer(&mut plain(BENCH_DRIVER), BENCH_DRIVER).body);
    // `numFBRegions` (offset 344 at the bench layout, `tests/gsp_static_info.rs`): console, heap,
    // carve-out
    assert_eq!(r.body[344..348], 3u32.to_le_bytes());
    let regions = p.fb_regions_now().expect("carved");
    assert_eq!(
        (regions[0].base, regions[0].limit, regions[0].reserved),
        (0, C_1080P - 1, C_1080P)
    );
    assert_eq!(regions[1].base, C_1080P);
}

#[test]
fn gop_on_refuses_a_console_it_cannot_serve_by_name() {
    let refused = |body: Vec<u8>| {
        let cell = SystemInfoCell::new();
        cell.store(&body, 1);
        let mut p = plain(BENCH_DRIVER).with_console(seat(&cell, true));
        let why = p.fb_regions_now().expect_err("refused");
        let r = answer(&mut p, BENCH_DRIVER);
        assert_eq!(
            (r.rpc_result, r.body.len()),
            (kf_abi::NV_ERR_NOT_SUPPORTED, 0),
            "refused in the envelope: {why}"
        );
        why
    };
    assert_eq!(
        refused(fn72_body(BENCH_DRIVER, C_1080P + 0x1000)),
        ConsoleRefusal::Layout(ConsoleRefused::Unaligned {
            console: C_1080P + 0x1000
        })
    );
    assert_eq!(
        refused(fn72_body(BENCH_DRIVER, BAR1 + 0x1_0000)),
        ConsoleRefusal::LargerThanBar1 {
            console: BAR1 + 0x1_0000,
            bar1_bytes: BAR1
        }
    );
    let matrix = Resolved::of(&kf_abi::generated::matrix::GSPSYSTEMINFO, BENCH_DRIVER)
        .unwrap()
        .size();
    assert_eq!(
        refused(vec![0u8; matrix - 8]),
        ConsoleRefusal::WrongSize {
            version: BENCH_DRIVER,
            declared: matrix - 8,
            matrix
        }
    );
    // a board whose table is not kf-chip's layout (the captured five-region shape) is never carved
    let mut board = ga106::board_at(ga106::FB_SIZE_MB);
    board.fb_regions.rotate_left(1);
    let cell = SystemInfoCell::new();
    cell.store(&fn72_body(BENCH_DRIVER, C_1080P), 1);
    let p = StaticInfoPolicy::new(std::sync::Arc::new(board), table(BENCH_DRIVER))
        .with_console(seat(&cell, true));
    assert_eq!(p.fb_regions_now(), Err(ConsoleRefusal::NotOurLayout));
}

#[test]
fn console_mem_size_reads_each_versions_own_offset() {
    let mut offsets = std::collections::BTreeSet::new();
    for &ver in kf_abi::generated::matrix::MEASURED {
        let Ok(l) = Resolved::of(&kf_abi::generated::matrix::GSPSYSTEMINFO, ver) else {
            continue;
        };
        offsets.insert(l.need("consoleMemSize").unwrap().off());
        let body = fn72_body(ver, C_1080P);
        let s = {
            let c = SystemInfoCell::new();
            c.store(&body, 1);
            c.latest().unwrap()
        };
        assert_eq!(console_mem_size(ver, &s), Ok(C_1080P), "{ver}");
    }
    assert!(
        offsets.len() >= 2,
        "the field moves across the matrix's versions ({offsets:?}) — the reason it is read late"
    );
}

/// ★★★ The case the lazy decode exists for: a DEFAULTED device re-selects its tables at fn 1, and
/// fn 72 came BEFORE fn 1. The kept body is the guest's own version's struct; the provisional table
/// cannot read it, the re-selected one can — and fn 65 is answered by the re-selected one.
#[test]
fn fn72_before_a_reselect_at_fn1_is_decoded_with_the_table_that_serves_fn65() {
    let provisional = BENCH_DRIVER;
    let size = |ver| {
        Resolved::of(&kf_abi::generated::matrix::GSPSYSTEMINFO, ver)
            .unwrap()
            .size()
    };
    // ⊘ A known positive, not a skip: a matrix version the bench re-selects to whose
    // GspSystemInfo differs from the bench's.
    let guest = kf_abi::generated::matrix::MEASURED
        .iter()
        .copied()
        .find(|&g| {
            g != provisional
                && table_for(g).is_ok()
                && pre_fn1_surface_differs(&table(provisional), &table(g)).is_none()
                && size(g) != size(provisional)
        })
        .expect("a re-selectable version whose GspSystemInfo differs (570.148.08 at this matrix)");

    let cell = SystemInfoCell::new();
    // fn 72 arrives first, in the GUEST's layout
    cell.store(&fn72_body(guest, C_1080P), 1);
    let recipe_cell = cell.clone();
    let board = ga106::board();
    let mut chain = ReselectAtFn1::new(
        table(provisional),
        GuestDriverSource::Defaulted,
        Box::new(move |t: DriverAbiTable| {
            Box::new(StaticInfoPolicy::new(board.clone(), t).with_console(seat(&recipe_cell, true)))
                as Box<dyn CommandPolicy>
        }),
    );
    // the provisional table cannot read it
    let bench = plain(provisional).with_console(seat(&cell, true));
    assert!(matches!(
        bench.fb_regions_now(),
        Err(ConsoleRefusal::WrongSize { .. })
    ));
    // fn 1 states the guest's version: re-selected
    let mut payload = vec![0u8; SET_GUEST_SYSTEM_INFO_SIZE];
    let said = guest.to_string();
    payload[24..24 + said.len()].copy_from_slice(said.as_bytes());
    assert_eq!(
        chain.on_fn1(&payload),
        Reselection::Reselected {
            from: provisional,
            to: guest
        }
    );
    // fn 65: the re-selected chain decodes the kept body at the guest's offsets
    let r = answer(&mut chain, guest);
    assert_eq!(r.rpc_result, 0, "{guest}");
    assert_eq!(r.body, carved(guest, C_1080P));
}

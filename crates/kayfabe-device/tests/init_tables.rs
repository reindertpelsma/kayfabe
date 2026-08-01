//! ★★★ The two init tables, pinned against the bytes a real driver actually accepted.
//!
//! The hex below was not written by hand and was not read off this port's own output. It
//! was extracted from the committed C capture
//! `nvidia-gpu-passthrough/traces/mode2_c_reference/cap1b_coldboot_hermetic_d6.rec.zst`
//! (360 725 records, hermetic, `n_errors=0`) — the `GuestWrite` status-queue elements
//! carrying the single `fn=76` reply for each of the two control commands, decoded with
//! the element layout `scripts/mode2_diag/rec_replydiff.py` documents. Those are the
//! bytes a stock 580.159.04 guest read and proceeded on.
//!
//! ## ★★ What this test can and cannot settle
//!
//! It settles **layout**: strides, field order, the `NvU16`-then-`NvU32` hole in an
//! interrupt entry, the eight-byte alignment hole before `subtreeMap`, and the `[OUT]`
//! sizes. A single wrong offset moves every later byte and the comparison fails.
//!
//! It does **not** settle whether six engines are enough — no trace can, because the C
//! advertised ten and this port advertises six on purpose. That question is answered on
//! hardware, and the omission is argued at `kayfabe_device::ga10x::GA106_ENGINES`.

use kayfabe_abi::inittables::{
    DEVICE_ENTRY_SIZE, DEVICE_INFO_PARAMS_SIZE, INTR_PARAMS_SIZE,
    NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE, NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE,
    encode_device_info_table, encode_intr_kernel_table, engine_info_type,
};
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// The six kept entries of the C's `cmd=0x20801112` reply, in wire order — 600 bytes.
const ORACLE_ENGINE_ENTRIES: &str = concat!(
    "00df34d300000000010000000000000040000000330000000c00000000000000",
    "5400000000000000000000000000c00001000000000000000000c20000000000",
    "0000000001000000200000002100000002000000475230000000000000000000",
    "0000000000eb3c790100000009000000000000000f0000001700000002000000",
    "000000000f00000013000000000000000000c00001000000010000000000c200",
    "0000000000000000010000002000000021000000010000004345300000000000",
    "000000000000000001eb3c79020000000a000000000000001000000018000000",
    "03000000000130821000000013000000010000000000c0000100000002000000",
    "0000c20000000000010000000100000020000000210000000100000043453100",
    "00000000000000000000000002eb3c79030000000b0000000100000011000000",
    "19000000040000008f05f2771100000013000000020000000004c00001000000",
    "000000000020c200000000000500000001000000220000002100000001000000",
    "4345320000000000000000000000000003eb3c79040000000c00000002000000",
    "120000001a0000000500000002018e011200000013000000030000000008c000",
    "01000000000000000040c2000000000006000000010000002300000021000000",
    "010000004345330000000000000000000000000000f5a695ffffffff2d000000",
    "07000000ffffffff00000000100830828f05f2770f008e010000000000013082",
    "8f05f27700000000000000001007308200000000080000000100000000013082",
    "8f05f27701000000534f4654574152450000000000000000",
);

/// `tableLen` plus all 24 entries of the C's `cmd=0x20800a5c` reply — 388 bytes.
const ORACLE_INTR_PREFIX: &str = concat!(
    "180000003b0000000000000040000000ffffffff3e0000000000000083000000",
    "ffffffff3c0000000000000048000000ffffffff490000000000000081000000",
    "ffffffff9c00000000000000ffffffffffffffff9d00000000000000ffffffff",
    "ffffffff9e00000000000000ffffffffffffffff9f00000000000000ffffffff",
    "ffffffffa000000000000000ffffffffffffffffa100000000000000ffffffff",
    "ffffffffa200000000000000ffffffffffffffffa300000000000000ffffffff",
    "ffffffff32000000000000009b000000ffffffff02000000000000009a000000",
    "ffffffff5400000000000000ffffffff000000002f00000000000000ffffffff",
    "020000002600000000000000ffffffff010000004100000000000000ffffffff",
    "030000000f00000000000000ffffffffffffffff1000000000000000ffffffff",
    "ffffffff1100000000000000ffffffff070000001200000000000000ffffffff",
    "080000001300000000000000ffffffff0a0000005100000000000000ffffffff",
    "09000000",
);

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

fn chip() -> &'static kayfabe_device::ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("the chip row this port ships")
}

#[test]
fn the_engine_entries_are_the_bytes_the_oracle_put_on_the_wire() {
    let page = encode_device_info_table(chip().engines, 0).expect("encodes");
    // Literals, for the reason the interrupt-table test states.
    assert_eq!(page.params.len(), 3212);
    assert_eq!(DEVICE_INFO_PARAMS_SIZE, 3212);
    assert_eq!(DEVICE_ENTRY_SIZE, 100);
    assert_eq!(page.num_entries, 6);
    assert!(!page.more);
    let want = unhex(ORACLE_ENGINE_ENTRIES);
    assert_eq!(want.len(), 6 * DEVICE_ENTRY_SIZE);
    assert_eq!(
        &page.params[12..12 + want.len()],
        &want[..],
        "the encoded entries diverge from the C's cap1b reply"
    );
    // Everything past the populated entries is zero, because RM copies out the whole
    // struct and reads `numEntries` to know where to stop.
    assert!(page.params[12 + want.len()..].iter().all(|b| *b == 0));
}

#[test]
fn the_interrupt_table_is_the_bytes_the_oracle_put_on_the_wire() {
    let p = encode_intr_kernel_table(chip().intr_table, &chip().intr_subtree_map).expect("encodes");
    // ★★ Every number below is a LITERAL, deliberately. Reading the map back at
    // `INTR_SUBTREE_MAP_OFF` would make this test agree with the constant under test
    // instead of with the wire. Induced 2026-07-31 on this branch: with the constant moved
    // to the packed offset 2052, all six tests stayed green until these literals went in.
    assert_eq!(p.len(), 2112);
    assert_eq!(INTR_PARAMS_SIZE, 2112);
    let want = unhex(ORACLE_INTR_PREFIX);
    assert_eq!(want.len(), 4 + 24 * 16);
    assert_eq!(&p[..want.len()], &want[..]);
    // The tail between the last entry and the map is the alignment hole, and the map is
    // the part the C synthesised rather than captured.
    assert!(p[want.len()..2056].iter().all(|b| *b == 0));
    let map: Vec<u64> = (0..7)
        .map(|i| {
            let o = 2056 + i * 8;
            u64::from_le_bytes(p[o..o + 8].try_into().unwrap())
        })
        .collect();
    assert_eq!(map, vec![0x0, 0x8, 0x1, 0x0, 0x0, 0x2, 0x4]);
}

#[test]
fn only_engines_this_port_can_serve_are_advertised() {
    let names: Vec<&str> = chip().engines.iter().map(|e| e.name).collect();
    assert_eq!(names, vec!["GR0", "CE0", "CE1", "CE2", "CE3", "SOFTWARE"]);
    // ⊘ The four the C advertised and this port does not. Named individually so that
    // adding one back is a deliberate edit to a test that says why it was out.
    for absent in ["SEC2", "NVENC0", "NVDEC0", "OFA", "CE4"] {
        assert!(
            !names.contains(&absent),
            "{absent} is advertised but nothing in this port serves it"
        );
    }
    // RM counts a runlist per host-driven entry; SOFTWARE is the one that is not.
    let host_driven = chip()
        .engines
        .iter()
        .filter(|e| e.engine_data[engine_info_type::IS_HOST_DRIVEN_ENGINE] != 0)
        .count();
    assert_eq!(host_driven, 5);
    assert_eq!(
        chip()
            .engines
            .iter()
            .find(|e| e.name == "SOFTWARE")
            .expect("the pseudo-engine RM keeps its own bookkeeping on")
            .engine_data[engine_info_type::IS_HOST_DRIVEN_ENGINE],
        0
    );
}

/// Build the `fn=76` payload a guest sends: the 40-byte control header, then a zeroed
/// `[OUT]` params buffer — which is exactly what makes an echoed reply useless.
fn control_payload(cmd: u32, params_size: usize) -> Vec<u8> {
    let mut p = vec![0u8; 40 + params_size];
    p[0..4].copy_from_slice(&0xc1ee_0000u32.to_le_bytes()); // hClient
    p[4..8].copy_from_slice(&0xc1ee_0001u32.to_le_bytes()); // hObject
    p[8..12].copy_from_slice(&cmd.to_le_bytes());
    p[16..20].copy_from_slice(&(params_size as u32).to_le_bytes());
    p
}

fn command(cmd: u32, params_size: usize) -> RpcCommand {
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 7,
        payload: control_payload(cmd, params_size),
        elements: 1,
        delivered: Vec::new(),
    }
}

fn policy() -> InitTablePolicy {
    let abi = kayfabe_device::abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER)
        .expect("the bench driver has a wire table");
    InitTablePolicy::new(chip(), abi.driver)
}

#[test]
fn the_policy_answers_both_controls_with_a_populated_table() {
    let mut p = policy();

    let dev = command(
        NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE,
        DEVICE_INFO_PARAMS_SIZE,
    );
    let r = p.respond(&dev).expect("answered");
    assert_eq!(r.rpc_result, 0);
    assert_eq!(r.body.len(), dev.payload.len());
    // `status` (NV_OK) and `paramsSize` are the two fields a GSP owns on the reply.
    assert_eq!(u32::from_le_bytes(r.body[12..16].try_into().unwrap()), 0);
    assert_eq!(
        u32::from_le_bytes(r.body[16..20].try_into().unwrap()) as usize,
        DEVICE_INFO_PARAMS_SIZE
    );
    // hClient / hObject / cmd are echoed, as on a real reply.
    assert_eq!(&r.body[0..12], &dev.payload[0..12]);
    // ★ The number RM allocates against. Zero here is the whole bug this rung fixes.
    assert_eq!(u32::from_le_bytes(r.body[44..48].try_into().unwrap()), 6);

    let intr = command(
        NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE,
        INTR_PARAMS_SIZE,
    );
    let r = p.respond(&intr).expect("answered");
    assert_eq!(r.rpc_result, 0);
    // ★ `tableLen`, the argument to `vectReserve`, which asserts `n > 0`.
    assert_eq!(u32::from_le_bytes(r.body[40..44].try_into().unwrap()), 24);
}

#[test]
fn every_other_command_still_falls_through_to_the_baseline() {
    let mut p = policy();
    // A control this policy does not model.
    let other = command(0x2080_012b, 560);
    assert!(p.respond(&other).is_none());
    assert!(p.wanted(&other).is_none());
    // A function that is not a control at all.
    let mut alloc = command(NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE, 64);
    alloc.function = RpcFunction::RmAlloc;
    assert!(p.respond(&alloc).is_none());
    // A payload too short to even be a control header.
    let mut stub = command(NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE, 0);
    stub.payload.truncate(8);
    assert!(p.respond(&stub).is_none());
}

#[test]
fn a_size_this_port_does_not_encode_is_refused_loudly_not_answered() {
    let mut p = policy();
    // A guest whose struct is a different size is a guest whose struct is not ours.
    let wrong = command(NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE, 128);
    let r = p.respond(&wrong).expect("refused, not ignored");
    assert_eq!(r.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
    assert!(r.body.is_empty());
    assert_eq!(p.wanted(&wrong), Some(WantedTable::DeviceInfo));

    // A payload that declares our size but cannot hold it.
    let mut short = command(
        NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE,
        INTR_PARAMS_SIZE,
    );
    short.payload.truncate(100);
    let r = p.respond(&short).expect("refused");
    assert_eq!(r.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);

    // A FINN-serialized payload is not the flat struct these encoders produce.
    let mut ser = command(
        NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE,
        DEVICE_INFO_PARAMS_SIZE,
    );
    ser.payload[20..24].copy_from_slice(&2u32.to_le_bytes()); // RMAPI_RPC_FLAGS_SERIALIZED
    let r = p.respond(&ser).expect("refused");
    assert_eq!(r.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
}

#[test]
fn every_variant_of_the_served_universe_round_trips_through_its_own_control_id() {
    // ★★ [`WantedTable::ALL`] exists so a *consumer* — `kayfabe-crec`'s reply-plane
    // differential — can quantify over "every control this port serves" without writing
    // the list a second time. A list that could silently shrink would weaken that gate
    // with zero red tests, which is this repository's most-repeated defect shape.
    //
    // ⊘ CORRECTED. This comment used to claim the round trip meant "a variant that has an
    // id but is missing from `ALL` fails here". It could not: the loop below iterates
    // `ALL`, so a variant absent from `ALL` is precisely the one it never visits. The claim
    // was false in the direction that flattered it — the same shape as PC-D6 — and it was
    // guarding the two gates that quantify over `ALL` (the sticky-answer property, and
    // `kayfabe-crec`'s reply-plane differential).
    //
    // ★★ What is true now is stronger than a test: `WantedTable::from_cmd` is a lookup
    // THROUGH `ALL`, so "in `ALL`" and "served" are one fact. A variant left out of the
    // array is not served at all, and the guest gets the ordinary named refusal. This test
    // pins the two things that remain assertions rather than construction — the size, and
    // that no two variants claim one id.
    assert_eq!(WantedTable::ALL.len(), 20, "the served universe's size");
    let mut ids = std::collections::BTreeSet::new();
    for w in WantedTable::ALL {
        let id = w.cmd_id();
        assert!(ids.insert(id), "two variants claim control 0x{id:08x}");
        assert_eq!(
            WantedTable::from_cmd(id),
            Some(w),
            "0x{id:08x} does not classify back to {w:?}"
        );
        assert!(w.params_size() > 0, "{w:?} has no [OUT] size");
    }
    // And the negative: an id nothing serves must classify as nothing, or the universe is
    // not a universe.
    assert_eq!(WantedTable::from_cmd(0x2080_0000), None);

    // ★ The construction, asserted from the OUTSIDE rather than read off the source: every
    // id `from_cmd` accepts is an id some row of `ALL` states. Swept over the whole
    // 16-bit index space of each class prefix `ALL` names — 0x2080_xxxx today — which is
    // where every control this policy could plausibly grow into lives.
    let served: std::collections::BTreeSet<u32> =
        WantedTable::ALL.iter().map(|w| w.cmd_id()).collect();
    let prefixes: std::collections::BTreeSet<u32> =
        served.iter().map(|id| id & 0xffff_0000).collect();
    assert!(!prefixes.is_empty(), "no class prefix to sweep");
    let mut accepted = std::collections::BTreeSet::new();
    for p in prefixes {
        for i in 0..=0xffffu32 {
            if WantedTable::from_cmd(p | i).is_some() {
                accepted.insert(p | i);
            }
        }
    }
    assert_eq!(
        accepted, served,
        "`from_cmd` accepts an id no row of `ALL` states, so the served universe and the \
         gates' universe have parted"
    );
}

#[test]
fn no_control_this_port_serves_can_be_cached_permanently_by_the_guest() {
    // ★★ PC-D6's neighbour, and the one the audit called "inert today, nothing checks
    // that". Our reply keeps the request's whole control header, `rmctrlFlags` included,
    // and those flags decide whether the guest puts the answer in its control cache
    // FOREVER — `rmapiControlCacheSetUnchecked` (`ogkm-580: rpc.c:11096-11103`), reached
    // only when `IsGssLegacyCall(cmd)` holds, i.e. `cmd & RM_GSS_LEGACY_MASK`
    // (`rmapi_deprecated.h:41`, `rmapi_deprecated_control.c:95-98`).
    //
    // ⊘ Quantified over `WantedTable::ALL`, so a served control ADDED tomorrow is checked
    // tomorrow. A list written here would have to be remembered.
    for w in WantedTable::ALL {
        assert_eq!(
            w.cmd_id() & 0x0000_8000,
            0,
            "{w:?} (0x{:08x}) is a GSS-legacy call: the guest would cache our answer \
             permanently, so serving it is a decision that needs its own reasoning",
            w.cmd_id()
        );
    }

    // ★ The predicate the serve-site guard rests on, checked against a REAL GSS-legacy
    // control rather than a synthesised one: `NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2_PHYSICAL`
    // is `0x20808513` (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080thermal.h:137`),
    // built from a `LEGACY_NON_PRIVILEGED_INTERFACE_ID`. The guard inside `respond` is
    // unreachable while the assertion above holds — an unreachable branch cannot be bitten,
    // so the mechanism is exposed and tested here instead of argued for in a comment.
    assert!(kayfabe_device::inittables::is_gss_legacy(0x2080_8513));
    assert!(!kayfabe_device::inittables::is_gss_legacy(
        NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE
    ));
    assert!(kayfabe_device::inittables::is_gss_legacy(
        NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE | 0x0000_8000
    ));
}

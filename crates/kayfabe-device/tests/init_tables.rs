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

/// The deepest byte of the C capture the `0x20801112` argument rests on: the 12-byte header
/// plus all `numEntries = 11` entries it declares, at 100 bytes each.
///
/// ★ [`ORACLE_ENGINE_ENTRIES`] is six of those eleven — entries 0, 4, 5, 6, 7 and 10 — so
/// the deepest byte it touches is the end of entry 10, which is this. `0x20801112`'s row is
/// TRUNCATED (`dlen` 3208 of `psize` 3212) and the four bytes it drops are the last of
/// `entries[31]`'s `engineName`, an entry the count puts out of reach.
const ORACLE_DEEPEST_BYTE: usize = 12 + 11 * 100;

/// ★★★ Nothing this file reads of the oracle's reply is missing from the capture.
#[test]
fn every_oracle_byte_this_file_reads_is_inside_what_the_recorder_kept() {
    let cmd = NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE;
    let row = kayfabe_abi::oracle::truncated_row(cmd).expect("0x20801112 is a truncated row");
    let r = kayfabe_abi::oracle::capture_reliance(cmd).expect("and it carries a reliance");
    assert_eq!(
        r.read_end, ORACLE_DEEPEST_BYTE,
        "this file and kayfabe_abi::oracle must agree on how deep the argument reaches"
    );
    assert!(
        kayfabe_abi::oracle::field_is_captured(0, ORACLE_DEEPEST_BYTE, row.kept),
        "reads [0,{ORACLE_DEEPEST_BYTE}) of a capture that kept {} of {}",
        row.kept,
        row.psize
    );
    assert!(12 + unhex(ORACLE_ENGINE_ENTRIES).len() <= ORACLE_DEEPEST_BYTE);
    assert!(!kayfabe_abi::oracle::field_is_captured(
        0, row.psize, row.kept
    ));
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
    // ★ 21 -> 22 at `#151`: `0x90f10106`, the client-context arm of the same page-directory
    // publication `0x20800a9f` carries. Two ids, one struct, one decode — see
    // `kayfabe_abi::gvaspacepdes`.
    // ★ 22 -> 23 at the `irq1` rung: `0x20802a08` CE_GET_FAULT_METHOD_BUFFER_SIZE, the
    // first served control whose reply is a number MEASURED on a real GA106 rather than
    // read out of a tree or a capture — see `kayfabe_abi::fmbsize`.
    // ★ 23 -> 24 at the `GR-info` rung: `0x20800a2a` INTERNAL_STATIC_KGR_GET_INFO. Its
    // 3712-byte reply is the SECOND measured on a real GA106, and the measurement went the
    // other way from `0x20802a08`'s: the C's captured row is FULL (`dlen == psize`) and
    // hardware CORROBORATED it byte for byte — see `kayfabe_abi::grinfo`.
    // ★★★ 24 -> 25 at the `cuInit` rung (`execution_plane_increments.md` §14.28):
    // `0x20800102` GPU_GET_INFO_V2, and it is the first of the twenty-five whose reply is a
    // **function of the request** rather than of the chip row. It is also the first admitted
    // by an *injection experiment* — one status forced to `0x56` at a time on a real GA106 —
    // rather than by a boot log. ⊘ And the eleven-row table that experiment published is at
    // the ioctl boundary: ten of those indices are answered by the guest's own kernel and
    // never reach a GSP. See `kayfabe_abi::gpuinfo`.
    // ★★★★ 25 -> 26 at §14.29: `0x20800a4c` INTERNAL_GPU_GET_SMC_MODE. Attributed, not
    // ratcheted — this is the id an in-guest bisect named as THE reason `cuInit` returned
    // 100, and the bisect is the evidence: of libcuda's eleven `GPU_GET_INFO_V2` indices,
    // exactly one (`0x2a`) failed alone and the prefix sweep broke at exactly its position.
    // Its arm forwards to this control and propagates the status to the whole call.
    // ⊘ It is NOT admitted by the injection matrix, which is structurally incapable of
    // finding it (§14.28: injection subtracts an answer from a system that works, and every
    // one of the sixteen ids it cleared was cleared on real firmware).
    // ★★★ 26 -> 27 at §14.30: `0x20801823` BUS_GET_INFO_V2, and it is the first of the
    // twenty-seven whose VALUE is **derived** rather than transcribed. Attributed, not
    // ratcheted: `[measured 2026-08-08, boot `v1429_49b182a`]` `cuInit` stops at this
    // control's second call with `0x56`, and `rmladder --bus-info-sweep` (R22) measured the
    // one index of six that reaches a GSP. ⊘ And it is the first row admitted with a
    // measurement that FORBIDS a chip constant: the same physical GA106 answered
    // `0x00302000` idle and `0x00322000` under load, so the served word comes off one enum
    // (`ChipProfile::pcie_max_gen`) through `PcieGenInfo::fully_trained`, never a table.
    // ★★★★ 27 -> 28 at §14.31: `0x2080182a` BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS, and it is
    // the first of the twenty-eight whose **refusal** is a measured hardware behaviour
    // rather than a gap. Attributed, not ratcheted: `[measured 2026-08-08, boot
    // `gt1430_0dbbabc`]` `cuInit` stops here with `0x56` and the ledger carries
    // `unserviced fn 76 cmd 0x2080182a` once; `[measured 2026-08-08, real GA106, `rmladder
    // --atomics-probe` (R23)]` a real part answers `capType=SYSMEM(0)` with thirteen
    // `bSupported=FALSE` written into a `0xCD`-seeded buffer, and refuses `_GPU(1)`,
    // `_P2P(2)` and every undeclared captype with `0x56` — so this row refuses them too.
    // ⊘ It is also the row that REFUTED the instrument: §14.30 read `--probe-ctrl`'s `0x56`
    // as caller-dependence, when `capType` is an `[IN]` field and the probe's own `0xCD`
    // seed was the invalid captype. See `kayfabe_abi::gpuatomics`.
    // ★★★★ 28 -> 29 at §14.32: `0x20801303` FB_GET_INFO_V2, and it is the first of the
    // twenty-nine that states **no new number at all** — all four forwarded indices are
    // projections of `ChipProfile::memory_system`, the row already served to `0x20800a1c`.
    // Attributed, not ratcheted: `[measured 2026-08-08, boot `gt1431_ff7a0ea`]` `cuInit`
    // stops at this control's FOURTH call (the first three are answered by the guest's own
    // kernel) with `0x56`, on a request a real GA106 answers `NV_OK`
    // (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:50`).
    // ⊘ It is also the row that REFUTED the instrument a second time: §14.31 read its
    // absence from both boot ledgers as *"the command never reaches the emulated GSP"*.
    // `[measured 2026-08-09]` both ledgers were SATURATED at their 32-entry caps in that
    // boot, so the absence meant nothing. See `kayfabe_abi::fbinfo`.
    // ★★★★ 29 -> 30 at §14.33: `0x20802a0b` CE_GET_ALL_PHYSICAL_CAPS, and it is the first
    // of the thirty whose reply is **constructed rather than the request edited** — the
    // struct is `[OUT]`-only and the guest has already zeroed it. Attributed, not ratcheted:
    // `[measured 2026-08-08, boot `gt1432_20e319b`]` `cuInit` stops at `0x20802a0a` with
    // `0x56`, and `0x20802a0b` — the id that control forwards to the PHYSICAL RMAPI under
    // `NV_ASSERT_OK_OR_RETURN`, i.e. the one that actually reaches us — is in that boot's
    // unserviced ledger. Nothing new is stated: `present` is `ChipProfile::engines`' own
    // `DEV_TYPE_ENUM_LCE` rows.
    // ⊘ It is also the row that REFUTED the instrument a THIRD time, twice over: §14.32
    // specified `rmladder --probe-ctrl 0x20802a0b:136` as the cheap way to settle the zero
    // tail. `[measured 2026-08-09, real GA106,
    // `traces/real_ga106/rmladder_r18_cecaps_real_ga106.txt`]` that control is
    // KERNEL_PRIVILEGED and refuses every usermode caller, and the reachable sibling
    // `portMemSet`s the probe's own seed away before forwarding. See `kayfabe_abi::cecaps`.
    // ★★★★ 30 -> 31 at §14.34: `0x20803801` GRMGR_GET_GR_FS_INFO, and it is the first of
    // the thirty-one whose errors are PER-ITEM rather than per-call — RM logs a bad query in
    // that query's own `status` and marches on (`ogkm-580: ctrl2080grmgr.h:42-50`), which is
    // the exact opposite of `FbGetInfoV2`'s rule one row above. Attributed, not ratcheted:
    // `[measured 2026-08-09, boot `gt1433_0de5ddb`]` `cuInit` stops at this control's only
    // call with `0x56` on a request a real GA106 answers `NV_OK`
    // (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:64`), and it is in that boot's
    // unserviced ledger. Nothing new is stated: the one query `cuInit` asks is answered from
    // `ChipProfile::gr_static`'s own GPC rows.
    // ⚠ It is also the first row whose WRONG answer would have been INVISIBLE: a query type
    // this port does not model could have been given a per-query `NV_ERR_NOT_SUPPORTED`
    // inside an `NV_OK` reply, which reaches neither ledger. `kayfabe_abi::grfsinfo` refuses
    // per-query only where RM itself does and takes the whole control down otherwise.
    // ★★★★ 31 -> 32 at §14.35: `0x20803601` GSP_GET_FEATURES, and it is the first of the
    // thirty-two whose reply is a fact about the **guest** rather than about the silicon —
    // three header constants plus a `firmwareVersion` latched from the guest's own
    // `SET_GUEST_SYSTEM_INFO` (`ogkm-580: rpc.c:8724-8727`). Attributed, not ratcheted:
    // `[measured 2026-08-09, boot `gt1434_373c145`]` `unserviced fn 76 cmd 0x20803601`,
    // against a real GA106 that answers `NV_OK` with all four fields set
    // (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:73`). Nothing new is tabulated
    // at all — which is a stronger statement than the "no new number" the last two rungs
    // made, because there is no per-chip row to be wrong.
    // ⚠ It is also the FIRST served row `crate::sticky::BRANCH_A_CACHEABLE` covers (flags
    // `0x40549` carry `RMCTRL_FLAGS_CACHEABLE`), so the guard at the serve site stops being
    // unreachable and the cache-lifetime decision is made rather than inherited.
    // ⊘ And the first whose two plausible constant sources are both WRONG: the host
    // driver's version is another machine's fact, and this policy's own
    // `DriverAbiTable::version()` is `[measured]` `580.65.06` where hardware says
    // `580.159.04` — the reading §14.35's own wording invited. See `kayfabe_abi::gspfeatures`.
    // ★★★★ 32 -> 33 at §14.36: `0x20808159`, the FIRST GSS-legacy id this port answers and
    // the first whose reply is the request VERBATIM. Attributed, not ratcheted:
    // `[measured 2026-08-09, boot `gf1435` at `d24ad77`]` `cuInit` stops at it as row 80 of
    // 87 and every row after is this port's teardown, against a real GA106 that answers
    // `NV_OK` and runs eight further calls
    // (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:80`).
    // ⊘ Nothing is tabulated: the reply IS the request. And that is a measurement rather
    // than an echo only because this path's copy-out is unconditional on `NV_OK` with no
    // `SKIP_COPYOUT` to hide behind — see `kayfabe_abi::gsslegacy`, which also carries why
    // this does NOT relax `kayfabe-rmrpc`'s refusal of GSS-legacy commands in general.
    // ★★★★ 33 -> 35 at §14.37: `0x20808162` (the SECOND GSS-legacy id) and `0x2080182b`
    // BUS_GET_C2C_INFO. `[measured 2026-08-09, boot `gf1436` at `ec434b8`]` `cuInit` stops
    // at rows 85 and 86 of 87 with `0x56` where a real GA106 answers `NV_OK`.
    // ⚠ The two are NOT the same kind, and the difference is the point: `0x20808162`'s
    // value is captured (`in=00 out=01`, double-sourced with the C), while `0x2080182b`'s
    // is an ARGUMENT — a GA106 has no chip-to-chip fabric, so `bIsLinkUp = false` is true
    // of the silicon and the all-zero capture is corroboration rather than the source.
    // ★★★★ 35 -> 36 at §14.41: `0x20800a9b` INTERNAL_GMMU_REGISTER_FAULT_BUFFER, and it is
    // a THIRD kind again. `0x20808159`'s reply is the request verbatim because the copy-out
    // is unconditional; this one's is the request verbatim because **the params are pure
    // `[IN]`** (`ogkm-580: ctrl2080internal.h:1792-1823`) — a real GSP writes nothing back,
    // so the identity is not an echo standing in for an unknown, it is the correct answer.
    // ⊘ Nothing is tabulated and nothing COULD be: there is no `[OUT]` field to be right or
    // wrong about, and no captured row for the id exists anywhere in the tree.
    // `[measured 2026-08-09, boot `pu1448` at `ef20ccc`]` refusing it fails
    // `faultbufConstruct_IMPL` -> `UVM_REGISTER_GPU` -> `cuInit`. The honesty of answering
    // `NV_OK` with no fault-delivery plane is decided, with evidence, in
    // `kayfabe_abi::faultbuffer`'s module docs, and the unbuilt half is printed in every
    // boot report that serves the control (`DELIVERY_UNBUILT`).
    // ★★★★ 36 -> 37 at §14.41's second rung: `0x20800a9d`
    // INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER, exposed by serving the first. Same
    // reply shape (identity on 24 032 pure-`[IN]` bytes, nothing tabulated) and a DIFFERENT
    // argument for it: here the GSP is the declared WRITER of a queue in the guest's own
    // sysmem (`ogkm-580: kern_gmmu.c:1589-1593`), so `NV_OK` promises more than it does for
    // `0x20800a9b`. ⊘ Which is why its unbuilt-half sentence is a different string and names
    // the substitute this port DOES build — an RC plus an error notifier
    // (`simulated_gpu_fault.md` §5.2). `[measured 2026-08-09, boot `fb1503` at `3afa896`]`
    // this is the id the guest asked for next.
    // ★★★★ 37 -> 38 at §14.41's third rung: `0x20800a1d`
    // INTERNAL_UVM_REGISTER_ACCESS_CNTR_BUFFER. ⊘ The only one of the three that was
    // UNREACHABLE until this port stopped serving zero at BAR0 `0xB83110` — its absence from
    // every prior unserviced ledger was evidence of nothing.
    // ★★★★ 38 -> 40 at §14.42, and the pair is one rung on purpose: `0x20802a07`
    // CE_GET_PHYSICAL_CAPS and `0x20802a02` CE_GET_CE_PCE_MASK are issued by ONE loop in
    // `queryCopyEngines` six lines apart, each under a hard `goto done`, so serving either
    // alone moves the wall and buys nothing.
    // ⊘ They are in OPPOSITE epistemic positions and the difference is one flag bit:
    // `0x20802a07` is `KERNEL_PRIVILEGED` and had to be DERIVED — it is a projection of the
    // very `CeGeometry` `0x20802a0b` already serves, stating no new number — while
    // `0x20802a02` carries `NON_PRIVILEGED`, so a real GA106 was simply asked (`R24`), and
    // its LCE4 refusal corroborates `present = 0x0f` from a third independent control.
    // ★★★★ 40 -> 41 at §14.43: `0xa06c010a`
    // NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS, the wall §14.42's rung exposed
    // and the FIRST row in this universe that is not a subdevice control. It is
    // `KERNEL_PRIVILEGED` like `0x20802a07`, so it cannot be measured — and unlike
    // `0x20802a07` it needs no derivation either, because EVERY field is `[input]`. The
    // reply is the guest's own facts re-encoded from what the decoder accepted; this port
    // states no number of its own anywhere in it. See `kayfabe_abi::fmbpromote`.
    assert_eq!(WantedTable::ALL.len(), 41, "the served universe\'s size");
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
    //
    // ★★★ §14.36 made this test's original form FALSE, and deliberately. It used to demand
    // that **no** served id be GSS-legacy, with the reason written into its own failure
    // message: *"serving it is a decision that needs its own reasoning"*. `0x20808159` is
    // that decision, and the reasoning is in `kayfabe_abi::gsslegacy`. So the property
    // narrows from "none" to "exactly the ones that carry an argument", which is the same
    // shape `gates_quantified_over_a_list` warns about — the list must not be allowed to
    // grow silently, so it is pinned as a set and not as a count.
    let gss_legacy_served: std::collections::BTreeSet<u32> = WantedTable::ALL
        .iter()
        .map(|w| w.cmd_id())
        .filter(|id| id & 0x0000_8000 != 0)
        .collect();
    assert_eq!(
        gss_legacy_served,
        kayfabe_abi::gsslegacy::SERVED
            .iter()
            .map(|(c, _)| *c)
            .collect::<std::collections::BTreeSet<u32>>(),
        "the guest caches a GSS-legacy answer from OUR reply's flags \
         (`rmapiControlCacheSetUnchecked`), so every id here has to carry its own argument. \
         Adding one means writing that argument, not extending this set"
    );
    // ⚠ And the argument for the one, restated as an executable check rather than a
    // sentence: its answer is the IDENTITY on the guest's own buffer, so even a cache that
    // did persist it would replay to the guest exactly what the guest sent. That is what
    // makes this the only kind of GSS-legacy answer that is safe under branch (b).
    let probe: Vec<u8> = (0..kayfabe_abi::gsslegacy::GSS_LEGACY_0X8159_PARAMS_SIZE)
        .map(|i| u8::try_from(i % 251).expect("fits"))
        .collect();
    assert_eq!(
        kayfabe_abi::gsslegacy::answer_gss_legacy(
            kayfabe_abi::gsslegacy::GSS_LEGACY_0X8159,
            &probe
        ),
        Ok(probe.clone()),
        "…8159's answer must be the identity, which is what makes caching it \
         indistinguishable from re-executing it"
    );
    // ⚠⚠ And the SECOND served GSS-legacy id does NOT have that property — it writes a byte
    // the guest did not send, so its branch-(b) safety rests entirely on
    // `crate::sticky::StickyAnswerGuard`. Asserted here so the two arguments cannot be
    // conflated by a reader who saw only the line above.
    assert_ne!(
        kayfabe_abi::gsslegacy::answer_gss_legacy(kayfabe_abi::gsslegacy::GSS_LEGACY_0X8162, &[0]),
        Ok(vec![0u8]),
        "…8162 is not an identity, and must not be argued about as if it were"
    );

    // ★ The predicate the serve-site guard rests on, checked against a REAL GSS-legacy
    // control rather than a synthesised one: `NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2_PHYSICAL`
    // is `0x20808513` (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080thermal.h:137`),
    // built from a `LEGACY_NON_PRIVILEGED_INTERFACE_ID`.
    //
    // ⊘ **The serve-site guard is STILL structurally unreachable, and §14.36 did not change
    // that** — it is worth saying plainly rather than letting the narrowing read as though a
    // branch became live. `from_cmd` gates entry to the serve site, so the only GSS-legacy id
    // that can arrive is the one the guard now exempts; `0x20808513` never reaches it and is
    // refused one level earlier, as an unserviced command. The guard is defence-in-depth
    // against a future row, not a check that runs today, and the real closure of branch (b)
    // is `crate::sticky::StickyAnswerGuard` zeroing the flag words on every accepted reply.
    // An unreachable branch cannot be bitten, so the predicate is exposed and tested here.
    assert!(kayfabe_device::inittables::is_gss_legacy(0x2080_8513));
    assert!(!kayfabe_device::inittables::is_gss_legacy(
        NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE
    ));
    assert!(kayfabe_device::inittables::is_gss_legacy(
        NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE | 0x0000_8000
    ));
}

/// ★★★ **`#151`: both ids of the page-directory publication reach one decode, and the
/// client-context one is the one a real boot needs.**
///
/// `gvaspaceCopyServerRmReservedPdesToServerRm_IMPL` branches on the resserv call context
/// (`ogkm-580: gpu_vaspace.c:4058`): no context sends `0x20800a9f`, a context sends
/// `0x90f10106` directly (`:5160-5190`). ⚠ Serving only the first looked complete and was
/// not — `[measured]` run `stateload2` at `7819839` shows `0x90f10106` refused and the
/// device VA space, the CE utility channel and the framebuffer scrubber all lost with it
/// (`/workspace/bench/run_stateload2_dmesg.log:12-30`).
///
/// ⊘ The test is written against the *equivalence*, not against one id answering: two
/// arms that decoded the same bytes differently would be a port whose page-directory
/// publication depended on which caller made it, which is exactly the class of bug two
/// copies of a decoder produce.
#[test]
fn both_ids_of_the_page_directory_publication_answer_identically() {
    use kayfabe_abi::gvaspacepdes as g;

    // The publication RM actually makes: `SPLIT_VAS_SERVER_RM_MANAGED_VA_START` = 0x1_0000_0000
    // over 512 MiB at `pageSize = NVBIT64(21)` (`ogkm-580: g_gpu_vaspace_nvoc.h:99-100`,
    // `gpu_vaspace.c:64`), with the four levels a GA106 publishes.
    let mut params = vec![0u8; g::COPY_SERVER_RESERVED_PDES_PARAMS_SIZE];
    params[0x08..0x10].copy_from_slice(&(1u64 << 21).to_le_bytes()); // pageSize
    params[0x10..0x18].copy_from_slice(&0x1_0000_0000u64.to_le_bytes()); // virtAddrLo
    params[0x18..0x20].copy_from_slice(&0x1_1FFF_FFFFu64.to_le_bytes()); // virtAddrHi
    params[0x20..0x24].copy_from_slice(&4u32.to_le_bytes()); // numLevelsToCopy
    for (i, shift) in [47u8, 38, 29, 21].into_iter().enumerate() {
        let at = 0x28 + i * g::LEVEL_SIZE;
        params[at..at + 8].copy_from_slice(&(0x0300_0000u64 + (i as u64) * 0x1000).to_le_bytes());
        params[at + 8..at + 16].copy_from_slice(&4096u64.to_le_bytes());
        params[at + 20] = shift;
    }

    let ids = [
        g::NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
        g::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
    ];
    let mut bodies = Vec::new();
    for cmd in ids {
        let mut c = command(cmd, params.len());
        c.payload[40..40 + params.len()].copy_from_slice(&params);
        let r = policy().respond(&c).expect("both ids are served");
        assert_eq!(r.rpc_result, 0, "cmd {cmd:#x} was refused");
        // The re-encoded params, which must be the guest's own publication back.
        assert_eq!(&r.body[40..40 + params.len()], &params[..], "cmd {cmd:#x}");
        bodies.push(r.body[40..40 + params.len()].to_vec());
    }
    assert_eq!(
        bodies[0], bodies[1],
        "the two ids decoded the same bytes differently"
    );

    // ⊘ And the decode is still load-bearing on BOTH: a publication that contradicts
    // `ctrl90f1.h`'s own alignment rule is refused, whichever caller made it. Without this
    // the shared arm would be a fall-through `NV_OK` for any 184 bytes.
    let mut bad = params.clone();
    bad[0x10..0x18].copy_from_slice(&0x1_0000_0001u64.to_le_bytes()); // virtAddrLo misaligned
    for cmd in ids {
        let mut c = command(cmd, bad.len());
        c.payload[40..40 + bad.len()].copy_from_slice(&bad);
        let r = policy().respond(&c).expect("refused, not ignored");
        assert_ne!(r.rpc_result, 0, "cmd {cmd:#x} accepted a misaligned range");
    }
}

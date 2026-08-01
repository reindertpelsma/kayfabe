//! `NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE` (`0x20800a40`) — the DEVICE_INFO2
//! table, and the first control this port answers by **deriving** rather than by stating.
//!
//! ## What this file establishes, in the order it matters
//!
//! 1. **The projection is right**, checked against silicon: driven with this chip's own
//!    FIFO engine table, the encoder reproduces the five rows the oracle's captured reply
//!    carries for those five engines, byte for byte.
//! 2. **Every field really is derived** — quantified over the whole of `GA106_ENGINES`,
//!    slot by slot, so a second hand-written table cannot creep back in without going red.
//! 3. **`SOFTWARE` is excluded by its marking, not by its name**, and the same test shows
//!    what would have gone on the wire if it were not: `runlistPriBase = 0x77f2058f`,
//!    capture noise, into a field RM files as a register base.
//! 4. **The count that outruns its buffer is unencodable**, and the test that says so pins
//!    the oracle's own window: a 24580-byte declared struct delivered in 16384 bytes covers
//!    entries `0..=340` and no more.
//!
//! ## Provenance
//!
//! `[measured]` [`ORACLE_FIVE`] is the concatenation of entries **0, 4, 5, 6, 7** of the
//! C's `ctl_20800a40` (`C: src/qemu/mode2_initctrl_ga106.h:4028`) — `GR0` and `CE0..CE3`,
//! in the order `GA106_ENGINES` lists them — extracted from that header's byte array. That
//! blob is in turn a replay of a real RTX 3060's own GSP reply.
//!
//! ⊘ The capture settles the **projection and the layout**. Which rows to serve is decided
//! in `kayfabe_device::ga10x::GA106_ENGINES`, one layer up, and it decides against six of
//! the oracle's twelve.

use kayfabe_abi::deviceinfo::{
    self, DEV_TYPE_ENUM_LCE, DEVICE_BROADCAST_PRI_BASE_OFF, DEVICE_INFO_TABLE_OFF,
    DEVICE_PRI_BASE_OFF, DeviceInfoError, DeviceInfoRow, DevicePriBase, EnginePriBase,
    FAULT_ID_OFF, GIN_TARGET_ID_OFF, GROUP_ID_OFF, GROUP_LOCAL_INSTANCE_ID_OFF, INSTANCE_ID_OFF,
    INTERNAL_DEVICE_INFO_MAX_ENTRIES, INTERNAL_DEVICE_INFO_PARAMS_SIZE,
    INTERNAL_DEVICE_INFO_STRIDE, IS_ENGINE_OFF, NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE,
    PRI_REGISTER_ALIGN, PriBaseField, RESET_ID_OFF, RL_ENG_ID_OFF, RUNLIST_PRI_BASE_OFF,
    TYPE_ENUM_OFF, engine_info_type,
};
use kayfabe_abi::inittables::{ENGINE_DATA_TYPES, FifoDeviceEntry};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::ChipProfile;
use kayfabe_device::ga10x;
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// GA106's register aperture — 16 MiB, spelled as a literal so the bound under test is not
/// derived from the same place the encoder reads it.
const APERTURE: u64 = 0x0100_0000;

/// ★ The five 48-byte entries the oracle's GA106 reported for `GR0` and `CE0..CE3` —
/// entries 0, 4, 5, 6 and 7 of its twelve, concatenated in `GA106_ENGINES`' order.
const ORACLE_FIVE: &str = concat!(
    "4000000000000000000000000c0000000000400001000000000000000000c000",
    "000000000000000000000000000000000f000000000000001300000002000000",
    "0040100001000000010000000000c00000000000000000000000000000000000",
    "100000000100000013000000030000000040100001000000020000000000c000",
    "0000000000000000000000000100000011000000020000001300000004000000",
    "0040100001000000000000000004c00000000000000000000000000002000000",
    "120000000300000013000000050000000040100001000000000000000008c000",
    "00000000000000000000000003000000",
);

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "hex must be whole bytes");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

/// The whole 24580-byte reply this device must produce: a count of five, the oracle's five
/// entries, then zero.
fn oracle_params() -> Vec<u8> {
    let five = unhex(ORACLE_FIVE);
    assert_eq!(five.len(), 240, "five entries of forty-eight bytes");
    let mut b = vec![0u8; 24580]; // literal — RM's own `sizeof *pParams`
    b[0..4].copy_from_slice(&5u32.to_le_bytes());
    b[4..244].copy_from_slice(&five);
    b
}

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

fn encode(engines: &[FifoDeviceEntry], row: &DeviceInfoRow) -> Result<Vec<u8>, DeviceInfoError> {
    deviceinfo::encode_internal_device_info_table(engines, row, APERTURE)
}

/// Read one field of one encoded entry.
fn field(params: &[u8], entry: usize, off: usize) -> u32 {
    let at = DEVICE_INFO_TABLE_OFF + entry * INTERNAL_DEVICE_INFO_STRIDE + off;
    u32::from_le_bytes(params[at..at + 4].try_into().expect("4 bytes"))
}

fn num_entries(params: &[u8]) -> u32 {
    u32::from_le_bytes(params[0..4].try_into().expect("4 bytes"))
}

// ── Synthetic silicon, for the refusals ────────────────────────────────────────────

/// A FIFO row with only the slots this reply's projection reads.
fn synth(
    name: &'static str,
    fault_id: u32,
    type_enum: u32,
    host_driven: u32,
    runlist_pri_base: u32,
) -> FifoDeviceEntry {
    let mut engine_data = [0u32; ENGINE_DATA_TYPES];
    engine_data[engine_info_type::MMU_FAULT_ID] = fault_id;
    engine_data[engine_info_type::DEV_TYPE_ENUM] = type_enum;
    engine_data[engine_info_type::IS_HOST_DRIVEN_ENGINE] = host_driven;
    engine_data[engine_info_type::RUNLIST_PRI_BASE] = runlist_pri_base;
    FifoDeviceEntry {
        name,
        engine_data,
        pbdma_ids: [0, 0],
        pbdma_fault_ids: [0, 0],
        num_pbdmas: 1,
    }
}

/// One copy engine, which is the minimum any encodable table must carry.
fn one_ce() -> FifoDeviceEntry {
    synth("CE0", 0x0f, DEV_TYPE_ENUM_LCE, 1, 0x00c0_0000)
}

fn row(bases: Vec<EnginePriBase>) -> DeviceInfoRow {
    DeviceInfoRow {
        pri_bases: Box::leak(bases.into_boxed_slice()),
    }
}

fn at(engine: &'static str, base: u32) -> EnginePriBase {
    EnginePriBase {
        engine,
        pri_base: DevicePriBase::At(base),
    }
}

fn not_a_device(engine: &'static str) -> EnginePriBase {
    EnginePriBase {
        engine,
        pri_base: DevicePriBase::NotADevice,
    }
}

// ── The projection, against silicon ────────────────────────────────────────────────

#[test]
fn the_projection_reproduces_the_oracles_five_rows_byte_for_byte() {
    // ★★★ The provenance test. Two sides that share no source: one is a real GA106's GSP
    // recorded through a driver, the other is `GA106_ENGINES` — itself read out of a
    // *different* control's capture, `0x20801112` — projected through offsets read from
    // `ctrl2080internal.h`.
    let got = encode(ga10x::GA106_ENGINES, &ga10x::GA106_DEVICE_INFO)
        .expect("this chip's engines project");
    assert_eq!(got, oracle_params(), "every byte of the 24580-byte reply");
}

#[test]
fn the_oracle_fixture_is_not_vacuous() {
    // The fixture really does carry five distinct populated entries, so a test that
    // compared against it would not be comparing against zeros.
    let p = oracle_params();
    assert_eq!(num_entries(&p), 5);
    assert_eq!(
        field(&p, 0, DEVICE_PRI_BASE_OFF),
        0x0040_0000,
        "GR0's PRI base"
    );
    for ce in 1..5 {
        assert_eq!(
            field(&p, ce, DEVICE_PRI_BASE_OFF),
            0x0010_4000,
            "every copy engine shares one PRI base"
        );
        assert_eq!(field(&p, ce, TYPE_ENUM_OFF), DEV_TYPE_ENUM_LCE);
    }
    // Fault ids 0x40, 0xf, 0x10, 0x11, 0x12 — five different values.
    let ids: Vec<u32> = (0..5).map(|i| field(&p, i, FAULT_ID_OFF)).collect();
    assert_eq!(ids, vec![0x40, 0x0f, 0x10, 0x11, 0x12]);
    // And entry 5 is where the table stops.
    let sixth = DEVICE_INFO_TABLE_OFF + 5 * INTERNAL_DEVICE_INFO_STRIDE;
    assert_eq!(
        &p[sixth..sixth + INTERNAL_DEVICE_INFO_STRIDE],
        &[0u8; 48][..]
    );
}

#[test]
fn the_layout_constants_are_the_structs_own_arithmetic() {
    // ★ LITERALS. Deriving these from the constants under test would move the check and the
    // encoder together.
    assert_eq!(INTERNAL_DEVICE_INFO_STRIDE, 48, "twelve NvU32, no padding");
    assert_eq!(
        INTERNAL_DEVICE_INFO_MAX_ENTRIES, 512,
        "NV2080_CTRL_CMD_INTERNAL_DEVICE_INFO_MAX_ENTRIES"
    );
    assert_eq!(
        4 + 512 * 48,
        24580,
        "a count then five hundred and twelve entries"
    );
    assert_eq!(INTERNAL_DEVICE_INFO_PARAMS_SIZE, 24580);
    assert_eq!(
        DEV_TYPE_ENUM_LCE, 19,
        "NV_PTOP_DEVICE_INFO2_DEV_TYPE_ENUM_LCE"
    );
    // The twelve field offsets, in declaration order, as the header gives them.
    assert_eq!(
        [
            FAULT_ID_OFF,
            INSTANCE_ID_OFF,
            TYPE_ENUM_OFF,
            RESET_ID_OFF,
            DEVICE_PRI_BASE_OFF,
            IS_ENGINE_OFF,
            RL_ENG_ID_OFF,
            RUNLIST_PRI_BASE_OFF,
            GROUP_ID_OFF,
            GIN_TARGET_ID_OFF,
            DEVICE_BROADCAST_PRI_BASE_OFF,
            GROUP_LOCAL_INSTANCE_ID_OFF,
        ],
        [0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44]
    );
}

#[test]
fn every_field_of_every_entry_comes_from_the_fifo_table_or_the_pri_base_row() {
    // ★★★ **The anti-drift test, and the reason this control has no table of its own.**
    // It is quantified over the whole of `GA106_ENGINES` — not over the five rows we expect
    // — so an engine added there without a projection cannot pass unexamined.
    let p = encode(ga10x::GA106_ENGINES, &ga10x::GA106_DEVICE_INFO).expect("projects");
    let mut emitted = 0usize;
    let mut skipped = 0usize;
    for e in ga10x::GA106_ENGINES {
        match ga10x::GA106_DEVICE_INFO
            .pri_base_for(e.name)
            .unwrap_or_else(|| panic!("{} has no statement", e.name))
        {
            DevicePriBase::NotADevice => {
                skipped += 1;
                continue;
            }
            DevicePriBase::At(base) => {
                let d = &e.engine_data;
                assert_eq!(field(&p, emitted, DEVICE_PRI_BASE_OFF), base, "{}", e.name);
                assert_eq!(
                    field(&p, emitted, FAULT_ID_OFF),
                    d[engine_info_type::MMU_FAULT_ID],
                    "{} faultId",
                    e.name
                );
                assert_eq!(
                    field(&p, emitted, INSTANCE_ID_OFF),
                    d[engine_info_type::INSTANCE_ID],
                    "{} instanceId",
                    e.name
                );
                assert_eq!(
                    field(&p, emitted, TYPE_ENUM_OFF),
                    d[engine_info_type::DEV_TYPE_ENUM],
                    "{} typeEnum",
                    e.name
                );
                assert_eq!(
                    field(&p, emitted, RESET_ID_OFF),
                    d[engine_info_type::RESET],
                    "{} resetId",
                    e.name
                );
                assert_eq!(
                    field(&p, emitted, IS_ENGINE_OFF),
                    d[engine_info_type::IS_HOST_DRIVEN_ENGINE],
                    "{} isEngine",
                    e.name
                );
                assert_eq!(
                    field(&p, emitted, RL_ENG_ID_OFF),
                    d[engine_info_type::RUNLIST_ENGINE_ID],
                    "{} rlEngId",
                    e.name
                );
                assert_eq!(
                    field(&p, emitted, RUNLIST_PRI_BASE_OFF),
                    d[engine_info_type::RUNLIST_PRI_BASE],
                    "{} runlistPriBase",
                    e.name
                );
                // ★ The three the projection zeroes, and the one it aliases.
                assert_eq!(field(&p, emitted, GROUP_ID_OFF), 0, "{} groupId", e.name);
                assert_eq!(field(&p, emitted, GIN_TARGET_ID_OFF), 0, "{}", e.name);
                assert_eq!(
                    field(&p, emitted, DEVICE_BROADCAST_PRI_BASE_OFF),
                    0,
                    "{}",
                    e.name
                );
                assert_eq!(
                    field(&p, emitted, GROUP_LOCAL_INSTANCE_ID_OFF),
                    d[engine_info_type::INSTANCE_ID],
                    "{} groupLocalInstanceId aliases instanceId",
                    e.name
                );
                emitted += 1;
            }
        }
    }
    assert_eq!(
        emitted,
        num_entries(&p) as usize,
        "the count is what was written, always"
    );
    assert_eq!(
        (emitted, skipped),
        (5, 1),
        "GR0 + CE0..CE3, and SOFTWARE dropped"
    );
}

// ── The pseudo-engine ──────────────────────────────────────────────────────────────

#[test]
fn the_pseudo_engine_is_excluded_by_its_marking_and_not_by_its_name() {
    // ★★★ The exclusion is a property of the derivation. Two halves:
    //
    // (a) `GA106_ENGINES` really does carry `SOFTWARE`, and it really is the only row the
    //     chip marks as not a device — so the reply's five-from-six is a decision.
    let software = ga10x::GA106_ENGINES
        .iter()
        .find(|e| e.name == "SOFTWARE")
        .expect("GA106_ENGINES carries RM's pseudo-engine");
    assert_eq!(
        software.engine_data[engine_info_type::IS_HOST_DRIVEN_ENGINE],
        0,
        "the one row with IS_HOST_DRIVEN_ENGINE clear"
    );
    let marked_absent: Vec<&str> = ga10x::GA106_ENGINES
        .iter()
        .filter(|e| {
            ga10x::GA106_DEVICE_INFO.pri_base_for(e.name) == Some(DevicePriBase::NotADevice)
        })
        .map(|e| e.name)
        .collect();
    assert_eq!(marked_absent, vec!["SOFTWARE"]);

    // (b) ⚠ What it would have put on the wire. The row is capture noise from RESET
    //     onward, and `runlistPriBase` is a value RM would file as a register base.
    assert_eq!(
        software.engine_data[engine_info_type::MMU_FAULT_ID],
        0xffff_ffff
    );
    assert_eq!(software.engine_data[engine_info_type::RESET], 0x8230_0810);
    assert_eq!(
        software.engine_data[engine_info_type::RUNLIST_PRI_BASE],
        0x77f2_058f
    );

    // ...and none of it reaches the reply.
    let p = encode(ga10x::GA106_ENGINES, &ga10x::GA106_DEVICE_INFO).expect("projects");
    for i in 0..num_entries(&p) as usize {
        assert_ne!(field(&p, i, FAULT_ID_OFF), 0xffff_ffff);
        assert_ne!(field(&p, i, RESET_ID_OFF), 0x8230_0810);
        assert_ne!(field(&p, i, RUNLIST_PRI_BASE_OFF), 0x77f2_058f);
    }
}

#[test]
fn marking_a_host_driven_engine_not_a_device_is_unencodable() {
    // ★★ What keeps the exclusion honest in the other direction: the marking is not
    // available for silicon the FIFO table says RM will put a runlist on.
    let engines = [one_ce(), synth("GR0", 0x40, 0, 1, 0x00c0_0000)];
    let r = row(vec![at("CE0", 0x0010_4000), not_a_device("GR0")]);
    assert_eq!(
        encode(&engines, &r).expect_err("refused"),
        DeviceInfoError::PriBaseAbsentForHostDrivenEngine { engine: "GR0" }
    );
    // Non-vacuity: the same row with GR0 not host-driven encodes, so the refusal is about
    // the disagreement and not about the marking.
    let engines = [one_ce(), synth("GR0", 0x40, 0, 0, 0x00c0_0000)];
    let p = encode(&engines, &r).expect("a non-host-driven row may be marked absent");
    assert_eq!(num_entries(&p), 1);
}

#[test]
fn a_row_the_fifo_table_does_not_call_host_driven_gets_no_runlist() {
    // ★ `RUNLIST_PRI_BASE` and `RUNLIST_ENGINE_ID` are "valid only for Esched-driven
    // engines" (`ogkm-580: engine_info.h`), so on such a row they are not data — which is
    // exactly where `GA106_ENGINES` keeps its capture noise. The oracle's own two
    // non-engine rows carry zero in both.
    let mut odd = synth("ODD", 0x20, 0x14, 0, 0x77f2_058f);
    odd.engine_data[engine_info_type::RUNLIST_ENGINE_ID] = 0x0bad_0bad;
    let engines = [one_ce(), odd];
    let r = row(vec![at("CE0", 0x0010_4000), at("ODD", 0x0011_0000)]);
    let p = encode(&engines, &r).expect("a non-host-driven device is still a device");
    assert_eq!(num_entries(&p), 2);
    assert_eq!(field(&p, 1, DEVICE_PRI_BASE_OFF), 0x0011_0000);
    assert_eq!(field(&p, 1, IS_ENGINE_OFF), 0);
    assert_eq!(
        field(&p, 1, RUNLIST_PRI_BASE_OFF),
        0,
        "not data on this row"
    );
    assert_eq!(field(&p, 1, RL_ENG_ID_OFF), 0, "nor this one");
}

// ── The count that outruns its buffer ──────────────────────────────────────────────

#[test]
fn the_c_oracles_own_reply_is_an_instance_of_the_fail_open() {
    // ★★★ The pattern, in the oracle's own numbers, as literals: the C registers
    // `{0x20800a40u, 0x0u, 24580u, 16384u, ctl_20800a40}` — a declared params size 8196
    // bytes larger than the data it holds — and RM does NOT zero its buffer first
    // (`portMemAllocNonPaged` at `ogkm-580: gpu_gspclient.c:219`, no portMemSet).
    let declared = 24580usize;
    let delivered = 16384usize;
    assert_eq!(declared - delivered, 8196);
    let covered = (delivered - 4) / 48;
    assert_eq!(covered, 341, "entries 0..=340 arrive in full");
    assert_eq!((delivered - 4) % 48, 12, "and twelve bytes of entry 341");
    assert!(
        covered < 512,
        "so a count in {}..=512 would have RM file entries out of uninitialised heap, and \
         RM's own bound — numEntries <= 512 — passes every one of them",
        covered + 1
    );
    // The C survived only because its count landed far inside the delivered prefix.
    assert!(12 < covered);
}

#[test]
fn the_reply_is_always_the_whole_struct_so_no_declared_count_can_outrun_it() {
    // ★★★ The structural closure, quantified over the WHOLE array rather than sampled: for
    // every encodable count, the buffer we return covers the entry that count names.
    let mut engines: Vec<FifoDeviceEntry> = Vec::new();
    let mut bases: Vec<EnginePriBase> = Vec::new();
    for i in 0..INTERNAL_DEVICE_INFO_MAX_ENTRIES {
        let name: &'static str = Box::leak(format!("CE{i}").into_boxed_str());
        engines.push(synth(
            name,
            u32::try_from(i).expect("fits"),
            DEV_TYPE_ENUM_LCE,
            1,
            0x00c0_0000,
        ));
        bases.push(at(name, 0x0010_4000));
    }
    let all: &'static [EnginePriBase] = Box::leak(bases.into_boxed_slice());
    for n in 1..=INTERNAL_DEVICE_INFO_MAX_ENTRIES {
        let r = DeviceInfoRow {
            pri_bases: &all[..n],
        };
        let p = encode(&engines[..n], &r).expect("inside the array");
        assert_eq!(p.len(), 24580, "the whole struct, always — literal");
        assert_eq!(num_entries(&p) as usize, n, "the count is what was written");
        assert!(
            4 + n * 48 <= p.len(),
            "the declared count never names an entry the reply did not carry"
        );
    }
}

#[test]
fn a_table_longer_than_the_wire_array_is_unencodable() {
    let mut engines: Vec<FifoDeviceEntry> = Vec::new();
    let mut bases: Vec<EnginePriBase> = Vec::new();
    for i in 0..=INTERNAL_DEVICE_INFO_MAX_ENTRIES {
        let name: &'static str = Box::leak(format!("CE{i}").into_boxed_str());
        engines.push(synth(
            name,
            u32::try_from(i).expect("fits"),
            DEV_TYPE_ENUM_LCE,
            1,
            0x00c0_0000,
        ));
        bases.push(at(name, 0x0010_4000));
    }
    let r = row(bases);
    assert_eq!(
        encode(&engines, &r).expect_err("RM asserts numEntries <= 512"),
        DeviceInfoError::TooManyEntries {
            count: 513,
            max: INTERNAL_DEVICE_INFO_MAX_ENTRIES
        }
    );
}

// ── The row type's own refusals ────────────────────────────────────────────────────

#[test]
fn an_engine_with_no_pri_base_statement_is_unencodable() {
    // ★★ The anti-drift refusal: adding an engine and forgetting this reply must not
    // silently drop it from one of the two descriptions of this silicon.
    let engines = [one_ce(), synth("GR0", 0x40, 0, 1, 0x00c0_0000)];
    let r = row(vec![at("CE0", 0x0010_4000)]);
    assert_eq!(
        encode(&engines, &r).expect_err("refused"),
        DeviceInfoError::NoPriBaseForEngine { engine: "GR0" }
    );
    // Non-vacuity: saying either thing about GR0 is enough.
    let r = row(vec![at("CE0", 0x0010_4000), at("GR0", 0x0040_0000)]);
    assert_eq!(num_entries(&encode(&engines, &r).expect("encodes")), 2);
}

#[test]
fn a_pri_base_for_an_engine_the_fifo_table_does_not_carry_is_unencodable() {
    let engines = [one_ce()];
    let r = row(vec![at("CE0", 0x0010_4000), at("NVDEC0", 0x0084_8000)]);
    assert_eq!(
        encode(&engines, &r).expect_err("refused"),
        DeviceInfoError::PriBaseForUnknownEngine { engine: "NVDEC0" }
    );
}

#[test]
fn two_statements_of_one_engines_pri_base_are_unencodable() {
    let engines = [one_ce()];
    let r = row(vec![at("CE0", 0x0010_4000), at("CE0", 0x0011_0000)]);
    assert_eq!(
        encode(&engines, &r).expect_err("refused"),
        DeviceInfoError::DuplicatePriBase { engine: "CE0" }
    );
}

#[test]
fn two_engines_sharing_one_name_are_unencodable() {
    // The lookup is by name, so a name that is not a key silently attributes one base to
    // whichever row comes first.
    let engines = [one_ce(), one_ce()];
    let r = row(vec![at("CE0", 0x0010_4000)]);
    assert_eq!(
        encode(&engines, &r).expect_err("refused"),
        DeviceInfoError::DuplicateEngineName { engine: "CE0" }
    );
}

#[test]
fn a_pri_base_outside_this_devices_aperture_is_unencodable() {
    let engines = [one_ce()];
    // The first aligned base past the end of a 16 MiB aperture.
    let r = row(vec![at("CE0", 0x0100_0000)]);
    assert_eq!(
        encode(&engines, &r).expect_err("refused"),
        DeviceInfoError::PriBaseOutsideAperture {
            engine: "CE0",
            field: PriBaseField::Device,
            pri_base: 0x0100_0000,
            aperture_len: APERTURE
        }
    );
    // The last register that DOES fit, so the bound is exact rather than conservative.
    let r = row(vec![at("CE0", 0x00ff_fffc)]);
    assert!(encode(&engines, &r).is_ok());

    // ★★ And the projected field is bounded too, not just the stated one: a row the FIFO
    // table calls host-driven carries its own `runlistPriBase` onto the wire.
    let noisy = [synth("CE0", 0x0f, DEV_TYPE_ENUM_LCE, 1, 0x7f00_0000)];
    let r = row(vec![at("CE0", 0x0010_4000)]);
    assert_eq!(
        encode(&noisy, &r).expect_err("refused"),
        DeviceInfoError::PriBaseOutsideAperture {
            engine: "CE0",
            field: PriBaseField::Runlist,
            pri_base: 0x7f00_0000,
            aperture_len: APERTURE
        }
    );
}

#[test]
fn a_pri_base_that_names_no_register_is_unencodable() {
    let engines = [one_ce()];
    let r = row(vec![at("CE0", 0x0010_4002)]);
    assert_eq!(
        encode(&engines, &r).expect_err("refused"),
        DeviceInfoError::PriBaseNotRegisterAligned {
            engine: "CE0",
            field: PriBaseField::Device,
            pri_base: 0x0010_4002
        }
    );
    assert_eq!(PRI_REGISTER_ALIGN, 4, "literal — a NvU32 register address");

    // ★★ This is the check that actually catches `SOFTWARE`'s capture noise if the
    // Esched-validity rule ever stops covering it: `0x77f2058f` names no register at all.
    let noisy = [synth("CE0", 0x0f, DEV_TYPE_ENUM_LCE, 1, 0x77f2_058f)];
    let r = row(vec![at("CE0", 0x0010_4000)]);
    assert_eq!(
        encode(&noisy, &r).expect_err("refused"),
        DeviceInfoError::PriBaseNotRegisterAligned {
            engine: "CE0",
            field: PriBaseField::Runlist,
            pri_base: 0x77f2_058f
        }
    );

    // ⚠ Non-vacuity, and the reason this is not a 4 KiB check like the falcon inventory's:
    // the oracle's own `runlistPriBase` values are 0x400-aligned and no more. A 4 KiB
    // granularity here would refuse silicon's own bytes.
    let engines = [synth("CE2", 0x11, DEV_TYPE_ENUM_LCE, 1, 0x00c0_0400)];
    let r = row(vec![at("CE2", 0x0010_4000)]);
    assert!(
        encode(&engines, &r).is_ok(),
        "0xc00400 is a real runlist base"
    );
}

// ── What the guest's only reader of this table requires ────────────────────────────

#[test]
fn a_table_with_no_copy_engine_is_unencodable() {
    // ★★★ `kgmmuInitCeMmuFaultIdRange_GA100` scans for typeEnum == LCE and, finding none,
    // logs "Failed to find any MMU Fault ID", asserts, and returns NV_ERR_OBJECT_NOT_FOUND
    // — two engine-init steps after this reply, attributed to nothing.
    // `gpuConstructDeviceInfoTable_FWCLIENT` itself accepts numEntries == 0 with NV_OK,
    // which is what makes it worth refusing here.
    let engines = [synth("GR0", 0x40, 0, 1, 0x00c0_0000)];
    let r = row(vec![at("GR0", 0x0040_0000)]);
    assert_eq!(
        encode(&engines, &r).expect_err("refused"),
        DeviceInfoError::NoCopyEngineAdvertised
    );
    // The empty table is the same refusal, not a different one.
    assert_eq!(
        encode(&[], &row(vec![])).expect_err("refused"),
        DeviceInfoError::NoCopyEngineAdvertised
    );
    // Non-vacuity: one copy engine is enough.
    let engines = [synth("GR0", 0x40, 0, 1, 0x00c0_0000), one_ce()];
    let r = row(vec![at("GR0", 0x0040_0000), at("CE0", 0x0010_4000)]);
    assert!(encode(&engines, &r).is_ok());
}

#[test]
fn copy_engine_fault_ids_that_are_not_one_run_are_unencodable() {
    // ★★ RM reduces the advertised set to [min, max] and then classifies a fault as a copy
    // engine's by RANGE MEMBERSHIP (`ogkm-580: kern_gmmu_gv100.c:739-740`). A gap is an id
    // the guest attributes to a copy engine this device never advertised.
    let engines = [
        synth("CE0", 0x0f, DEV_TYPE_ENUM_LCE, 1, 0x00c0_0000),
        synth("CE1", 0x11, DEV_TYPE_ENUM_LCE, 1, 0x00c0_0000), // 0x10 is nobody's
    ];
    let r = row(vec![at("CE0", 0x0010_4000), at("CE1", 0x0010_4000)]);
    assert_eq!(
        encode(&engines, &r).expect_err("refused"),
        DeviceInfoError::CopyEngineFaultIdsNotContiguous {
            first: 0x0f,
            last: 0x11,
            count: 2
        }
    );
    // Two copy engines claiming one fault id is the same refusal — the span is one, the
    // count is two — and it is just as much an aliasing of engines.
    let engines = [
        synth("CE0", 0x0f, DEV_TYPE_ENUM_LCE, 1, 0x00c0_0000),
        synth("CE1", 0x0f, DEV_TYPE_ENUM_LCE, 1, 0x00c0_0000),
    ];
    assert_eq!(
        encode(&engines, &r).expect_err("refused"),
        DeviceInfoError::CopyEngineFaultIdsNotContiguous {
            first: 0x0f,
            last: 0x0f,
            count: 2
        }
    );
    // ⊘ Non-vacuity, and the measurement the policy rests on: GA106's four are 0xf..=0x12.
    let p = encode(ga10x::GA106_ENGINES, &ga10x::GA106_DEVICE_INFO).expect("projects");
    let ce_ids: Vec<u32> = (0..num_entries(&p) as usize)
        .filter(|i| field(&p, *i, TYPE_ENUM_OFF) == DEV_TYPE_ENUM_LCE)
        .map(|i| field(&p, i, FAULT_ID_OFF))
        .collect();
    assert_eq!(ce_ids, vec![0x0f, 0x10, 0x11, 0x12]);
}

// ── The serve site ─────────────────────────────────────────────────────────────────

/// `RpcControlReq::HEADER`, as the sibling controls' captures give it.
const PARAMS_AT: usize = 40;

/// A `GSP_RM_CONTROL` whose header asks for `cmd` with `params_size` bytes of params.
///
/// ★★ The request body is `0xAA`. It matters more here than on the falcon inventory:
/// `gpuConstructDeviceInfoTable_FWCLIENT` does **not** `portMemSet` its params
/// (`ogkm-580: gpu_gspclient.c:219` allocates and goes straight to the control), so on the
/// bench an echoed or short reply would hand RM a `numEntries` out of the guest kernel
/// heap. A poisoned request is what tells a served reply from a reflected one.
fn device_info_command(cmd: u32, params_size: u32, serialized: bool) -> RpcCommand {
    let mut payload = vec![0xAAu8; PARAMS_AT + params_size as usize];
    payload[0..4].copy_from_slice(&0xc200_0006u32.to_le_bytes()); // hClient, as captured
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject, as captured
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    let flags: u32 = if serialized { 1 << 1 } else { 0 };
    payload[20..24].copy_from_slice(&flags.to_le_bytes());
    payload[24..40].fill(0);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 5,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

#[test]
fn the_policy_answers_the_control_without_reflecting_one_byte_of_the_request() {
    let mut p = policy();
    let reply = p
        .respond(&device_info_command(
            NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE,
            24580,
            false,
        ))
        .expect("this port serves the DEVICE_INFO2 table");
    assert_eq!(reply.rpc_result, 0, "NV_OK in the envelope");
    let params = &reply.body[PARAMS_AT..PARAMS_AT + 24580];
    assert_eq!(
        params,
        &oracle_params()[..],
        "the oracle's five rows, in full"
    );
    assert!(
        !params.contains(&0xAA),
        "not one poisoned request byte survived into the reply"
    );
    assert_eq!(
        &reply.body[12..16],
        &0u32.to_le_bytes()[..],
        "status = NV_OK"
    );
    assert_eq!(
        &reply.body[16..20],
        &24580u32.to_le_bytes()[..],
        "paramsSize, rewritten to what we encoded"
    );
}

#[test]
fn the_serve_site_refuses_when_the_projection_declines() {
    // ★★★ The fail-open guard at the layer that would actually widen: a chip whose
    // projection the encoder refuses must produce a REFUSAL, not a best-effort reply.
    //
    // The chip carries two engines and says where only one of them is — the drift
    // `NoPriBaseForEngine` exists for.
    let engines: &'static [FifoDeviceEntry] =
        Box::leak(Box::new([one_ce(), synth("GR0", 0x40, 0, 1, 0x00c0_0000)]));
    let bad = bad_chip(engines, row(vec![at("CE0", 0x0010_4000)]));
    let mut p = InitTablePolicy::new(bad, *table_for(BENCH_DRIVER).expect("bench ABI"));
    let reply = p
        .respond(&device_info_command(
            NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE,
            24580,
            false,
        ))
        .expect("answers, loudly");
    assert_ne!(reply.rpc_result, 0, "NV_ERR_NOT_SUPPORTED in the envelope");
    assert!(reply.body.is_empty(), "and no body to misread");
}

#[test]
fn a_declared_params_size_that_is_not_ours_is_refused_rather_than_answered() {
    for size in [0u32, 4, 16384, 24576, 24584] {
        let mut p = policy();
        let reply = p
            .respond(&device_info_command(
                NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE,
                size,
                false,
            ))
            .expect("answers");
        assert_ne!(
            reply.rpc_result, 0,
            "a guest declaring {size} bytes is not a guest whose struct we encode"
        );
        assert!(reply.body.is_empty());
    }
}

#[test]
fn a_serialized_request_is_refused_rather_than_answered_flat() {
    let mut p = policy();
    let reply = p
        .respond(&device_info_command(
            NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE,
            24580,
            true,
        ))
        .expect("answers");
    assert_ne!(
        reply.rpc_result, 0,
        "FINN-serialized is not our flat layout"
    );
    assert!(reply.body.is_empty());
}

#[test]
fn the_classifier_names_this_control_and_its_size() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE),
        Some(WantedTable::InternalDeviceInfo)
    );
    assert_eq!(
        WantedTable::InternalDeviceInfo.cmd_id(),
        0x2080_0a40,
        "literal — the id the capture carries"
    );
    assert_eq!(WantedTable::InternalDeviceInfo.params_size(), 24580);
    assert!(
        WantedTable::ALL.contains(&WantedTable::InternalDeviceInfo),
        "and it is in the universe every coverage gate quantifies over"
    );
    // ⊘ And it is NOT the other device-info control, which describes the same silicon.
    assert_ne!(
        WantedTable::InternalDeviceInfo.cmd_id(),
        WantedTable::DeviceInfo.cmd_id()
    );
}

// ── The bad chip, built from the real one ─────────────────────────────────────────

/// GA106 in every respect the test does not override.
///
/// ★ Built at run time from [`chip`] rather than spelled as a `const fn`, so a field added
/// to `ChipProfile` does not have to be restated here with a value invented for it. The two
/// overridden fields are the whole of what this test is about.
fn bad_chip(
    engines: &'static [FifoDeviceEntry],
    device_info: DeviceInfoRow,
) -> &'static ChipProfile {
    let g = chip();
    Box::leak(Box::new(ChipProfile {
        name: "TEST-BAD-DEVICE-INFO",
        pci_device_id: g.pci_device_id,
        pci_revision: g.pci_revision,
        pci_subsystem_vendor_id: g.pci_subsystem_vendor_id,
        pci_subsystem_id: g.pci_subsystem_id,
        regs_aperture_len: g.regs_aperture_len,
        pci_bars: g.pci_bars,
        boot_regs: g.boot_regs,
        ptimer: g.ptimer,
        rom_window: g.rom_window,
        pramin_window: g.pramin_window,
        bar0_window_reg: g.bar0_window_reg,
        vbios_wire: g.vbios_wire,
        msix_vectors: g.msix_vectors,
        gsp_model: g.gsp_model,
        engines,
        intr_table: g.intr_table,
        intr_subtree_map: g.intr_subtree_map,
        fb_regions: g.fb_regions,
        chip_info: g.chip_info,
        user_register_access_map: g.user_register_access_map,
        memory_system: g.memory_system,
        constructed_falcons: g.constructed_falcons,
        device_info,
        conf_compute: g.conf_compute,
        bif_static: g.bif_static,
        fifo_channels: g.fifo_channels,
        gmmu_static: g.gmmu_static,
        gr_static: g.gr_static,
        gr_context_buffers: g.gr_context_buffers,
        fb_length: g.fb_length,
    }))
}

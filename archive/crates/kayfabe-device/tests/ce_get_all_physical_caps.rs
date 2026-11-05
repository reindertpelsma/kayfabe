//! `NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS` (`0x20802a0b`) at the **reply-plane
//! boundary**, and against the committed real-GA106 captures.
//!
//! ## ⊘ What this file is for, beyond "the control is served"
//!
//! Three things `kayfabe-abi`'s own unit tests structurally cannot say:
//!
//! 1. That `InitTablePolicy` puts a **constructed** `[OUT]` body at the right offset inside
//!    the right envelope with the right inner status. This is the first arm that ignores the
//!    request entirely, so "the reply is at `params_at`" is a new claim, not an inherited one.
//! 2. ★★★ That the served bytes equal the ones a **real GA106 put on the wire**, read out of
//!    the committed trace files rather than out of a hex literal somebody typed. The ABI
//!    crate cannot reach `traces/`, and its own first draft of that literal was sixteen bytes
//!    short — so the artifact, not a transcription, is the authority here.
//! 3. ★★ That `present` really is a projection of the **same engine slice**
//!    `FIFO_GET_DEVICE_INFO_TABLE` serves, by driving both controls through one policy. That
//!    is a claim about two controls agreeing, so no single-control test can hold it — the
//!    shape `fb_get_info_v2.rs` established at §14.32.
//!
//! ⚠ **Two independent captures are compared, and the fact that they agree is the point.**
//! `cuinit_ioctl_trace_real_ga106.txt:62` is libcuda's `cuInit` through a full CUDA context;
//! `rmladder_r18_cecaps_real_ga106.txt` is a bare `Subdevice` with no channel, no CE object
//! and a `0xCD`-seeded buffer. They carry the identical 136 bytes. ⊘ Three instruments
//! agreeing is not corroboration when they share a defect (§14.26) — these two share only
//! the hardware, which is the thing being measured.

use kayfabe_abi::cecaps::{
    self, CE_GET_ALL_CAPS_PARAMS_SIZE, CeCaps, GA10X_EXPOSED_LCE_MASK_IS_NOT_A_SOURCE,
    NV2080_CTRL_CMD_CE_GET_ALL_CAPS, NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS, cap,
};
use kayfabe_abi::deviceinfo::DEV_TYPE_ENUM_LCE;
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_device::{ChipProfile, chip_for_device_id};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER`.
const PARAMS_AT: usize = 40;
/// Byte offset of `status` in the reply's control header.
const CONTROL_STATUS_OFF: usize = 12;

fn chip() -> &'static ChipProfile {
    chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// A `GSP_RM_CONTROL` carrying `cmd_id` with a params body seeded `0xCD`.
///
/// ★ The seed is the assertion: this control's reply is **constructed**, so every byte that
/// comes back must be one this port wrote. A single surviving `0xCD` would mean the arm
/// echoed the request somewhere, which for an `[OUT]`-only struct is a leak of guest bytes
/// back to the guest under the name of a hardware answer.
fn ce_command(cmd_id: u32) -> RpcCommand {
    let mut payload = vec![0xCDu8; PARAMS_AT + CE_GET_ALL_CAPS_PARAMS_SIZE];
    payload[0..4].copy_from_slice(&0xc1e0_0004u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject
    payload[8..12].copy_from_slice(&cmd_id.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(
        &u32::try_from(CE_GET_ALL_CAPS_PARAMS_SIZE)
            .expect("fits")
            .to_le_bytes(),
    );
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat, not FINN
    payload[24..40].fill(0);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x23,
        sequence: 40,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// The reply's `(inner control status, params)`; a refusal reports its `rpc_result` and an
/// empty body.
fn reply_params(cmd: &RpcCommand) -> Option<(u32, Vec<u8>)> {
    let reply = policy().respond(cmd)?;
    if reply.body.is_empty() {
        assert_ne!(
            reply.rpc_result, 0,
            "an empty body must never travel with NV_OK"
        );
        return Some((reply.rpc_result, Vec::new()));
    }
    let status = u32::from_le_bytes(
        reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
            .try_into()
            .expect("4 bytes"),
    );
    Some((
        status,
        reply.body[PARAMS_AT..PARAMS_AT + CE_GET_ALL_CAPS_PARAMS_SIZE].to_vec(),
    ))
}

/// Read a committed trace file, or panic naming it.
fn trace(rel: &str) -> (std::path::PathBuf, String) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is a committed artifact: {e}", path.display()));
    (path, text)
}

/// A hex run, checked to be exactly the struct, decoded to bytes.
///
/// ⊘ **The parse is asserted, not assumed.** A `grep` that finds nothing returns nothing,
/// and a test that silently compares zero captures against zero captures is the
/// `gate_read_through_grep_cannot_fail` shape. ★ It earned this on its first run: the two
/// traces are in **different formats** — the interposer writes `out=<hex>` and the ladder
/// writes `= NV_OK, 136 bytes: <hex>` — and one parser applied to both found nothing in the
/// second and said so, loudly, instead of comparing a capture against itself.
fn decode_hex(path: &std::path::Path, needle: &str, hex: &str) -> Vec<u8> {
    assert_eq!(
        hex.len(),
        CE_GET_ALL_CAPS_PARAMS_SIZE * 2,
        "{}: {needle} captured {} bytes, the struct is {CE_GET_ALL_CAPS_PARAMS_SIZE}",
        path.display(),
        hex.len() / 2
    );
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hex digits"))
        .collect()
}

/// libcuda's `cuInit`, through a full CUDA context on a real GA106—
/// `cuda_ioctl_trace.c`'s interposer format, `… out=<hex>`.
fn libcuda_capture() -> Vec<u8> {
    let (path, text) = trace("traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt");
    let needle = format!("cmd={NV2080_CTRL_CMD_CE_GET_ALL_CAPS:#010x}");
    let hex = text
        .lines()
        .find(|l| l.contains(&needle) && l.contains("out="))
        .and_then(|l| l.rsplit("out=").next())
        .map(|t| t.split_whitespace().next().unwrap_or(t).to_owned())
        .unwrap_or_else(|| panic!("no `out=` record for {needle} in {}", path.display()));
    decode_hex(&path, &needle, &hex)
}

/// A bare `Subdevice` with no channel and a `0xCD`-seeded buffer, same physical part —
/// `rmladder --probe-ctrl`'s R18 format, `★ R18 <cmd> = NV_OK, <n> bytes: <hex>`.
fn rmladder_capture() -> Vec<u8> {
    let (path, text) = trace("traces/real_ga106/rmladder_r18_cecaps_real_ga106.txt");
    let needle = format!("{NV2080_CTRL_CMD_CE_GET_ALL_CAPS:#010x}");
    let hex = text
        .lines()
        // ⚠ `contains("NV_OK")` is NOT enough: the file's own provenance header carries the
        // line `# H2 0x20802a0a … is reachable and answers NV_OK`, which matches the id and
        // the status and has no hex at all. A trace annotated for a human reader is a trace
        // with decoys in it; match on the field that only a data row has.
        .find(|l| l.contains(&needle) && l.contains("bytes: "))
        .and_then(|l| l.rsplit("bytes: ").next())
        .map(|t| t.split_whitespace().next().unwrap_or(t).to_owned())
        .unwrap_or_else(|| panic!("no served R18 row for {needle} in {}", path.display()));
    decode_hex(&path, &needle, &hex)
}

/// ⊘⊘ And the same file records that `0x20802a0b` itself is **unreachable** from usermode —
/// §14.32 specified a probe of it as this rung's cheap instrument. Pinned so the refutation
/// is a test and not only a paragraph: if a future driver makes that control probeable, this
/// goes red and the module header needs rewriting rather than quietly rotting.
#[test]
fn the_physical_control_is_refused_to_every_usermode_prober() {
    let (path, text) = trace("traces/real_ga106/rmladder_r18_cecaps_real_ga106.txt");
    let needle = format!("{NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS:#010x}");
    let row = text
        .lines()
        .find(|l| l.contains(&needle) && l.starts_with("info"))
        .unwrap_or_else(|| panic!("no R18 row for {needle} in {}", path.display()));
    assert!(
        row.contains("refused"),
        "{needle} was expected refused (KERNEL_PRIVILEGED | INTERNAL), got: {row}"
    );
}

/// ★★★ Two callers with nothing in common but the hardware, byte for byte.
#[test]
fn both_real_ga106_captures_carry_the_identical_reply() {
    assert_eq!(libcuda_capture(), rmladder_capture());
}

/// ★★★ The rung: what this port puts on the wire for `0x20802a0b` is what the physical
/// layer put on the wire for a real GA106.
#[test]
fn the_served_reply_is_the_real_ga106s_reply() {
    let (status, params) =
        reply_params(&ce_command(NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS)).expect("served");
    assert_eq!(status, 0, "NV_OK");
    assert_eq!(params, libcuda_capture());
}

/// ⊘ Not one byte of the request survives. On an `[OUT]`-only struct an echo is guest bytes
/// handed back as a hardware answer.
#[test]
fn no_seed_byte_survives_into_the_reply() {
    let (_, params) =
        reply_params(&ce_command(NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS)).expect("served");
    assert!(
        !params.contains(&0xCD),
        "the reply carries a byte from the request buffer"
    );
}

/// ⊘⊘ **The id that fails is not the id served.** `0x20802a0a` is the guest kernel's own
/// control; this policy must not answer it, or it would be answering a boundary the guest
/// never asks an emulated GSP about.
#[test]
fn the_kernel_side_caps_control_is_not_served() {
    assert_eq!(WantedTable::from_cmd(NV2080_CTRL_CMD_CE_GET_ALL_CAPS), None);
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS),
        Some(WantedTable::CeGetAllPhysicalCaps)
    );
    assert_ne!(
        NV2080_CTRL_CMD_CE_GET_ALL_CAPS,
        NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS
    );
}

/// ★★ The design claim, executed rather than asserted: `present` is the engine list this
/// device already advertises, not a number stated twice.
#[test]
fn present_is_the_same_engine_slice_the_device_info_table_serves() {
    let (_, params) =
        reply_params(&ce_command(NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS)).expect("served");
    let (present, tbl) = cecaps::decode_ce_get_all_physical_caps(&params).expect("decode");

    // The same slice, read the same way `encode_internal_device_info_table` reads it.
    let mut from_engines = 0u64;
    let mut names = Vec::new();
    for e in chip().engines {
        if e.engine_data[kayfabe_abi::deviceinfo::engine_info_type::DEV_TYPE_ENUM]
            == DEV_TYPE_ENUM_LCE
        {
            from_engines |=
                1u64 << e.engine_data[kayfabe_abi::deviceinfo::engine_info_type::INSTANCE_ID];
            names.push(e.name);
        }
    }
    assert_eq!(
        present, from_engines,
        "present must be exactly the LCE rows of chip.engines ({names:?})"
    );
    assert_eq!(names.len(), present.count_ones() as usize);

    // Every present CE carries caps; every absent one carries none.
    for (i, caps) in tbl.iter().enumerate() {
        let has = present & (1u64 << i) != 0;
        assert_eq!(
            *caps != CeCaps::NONE,
            has,
            "CE{i}: present={has} but caps={:#06x}",
            caps.as_u16()
        );
    }
}

/// ⊘ The HAL constant says five LCEs; the hardware exposes four. Pinned at the reply plane
/// too, because this is the value a "simplification" would reach for.
#[test]
fn present_is_not_the_allowed_lce_mask() {
    let (_, params) =
        reply_params(&ce_command(NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS)).expect("served");
    let (present, tbl) = cecaps::decode_ce_get_all_physical_caps(&params).expect("decode");
    assert_eq!(present, 0x0f);
    assert_ne!(present, GA10X_EXPOSED_LCE_MASK_IS_NOT_A_SOURCE);
    assert_eq!(tbl[4], CeCaps::NONE);
    let (real_present, real_tbl) =
        cecaps::decode_ce_get_all_physical_caps(&libcuda_capture()).expect("decode");
    assert_eq!(real_present, present);
    assert_eq!(real_tbl[4], tbl[4]);
}

/// ★ The only per-CE difference, checked against the capture rather than against ourselves.
#[test]
fn grce_is_the_only_bit_that_varies_and_it_matches_hardware() {
    let (_, params) =
        reply_params(&ce_command(NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS)).expect("served");
    let (_, ours) = cecaps::decode_ce_get_all_physical_caps(&params).expect("decode");
    let (_, real) = cecaps::decode_ce_get_all_physical_caps(&libcuda_capture()).expect("decode");
    for i in 0..4 {
        assert_eq!(ours[i], real[i], "CE{i}");
        assert_eq!(ours[i].has(cap::GRCE), i < 2, "CE{i} GRCE");
    }
}

/// ⊘ A `0x20802a0b` whose params are shorter than the struct must not be served from a
/// truncated envelope. The arm builds its own body, so the risk is the opposite of the usual
/// one: a reply longer than the request's declared `paramsSize`.
#[test]
fn the_reply_is_exactly_the_declared_struct_size() {
    assert_eq!(
        WantedTable::CeGetAllPhysicalCaps.params_size(),
        CE_GET_ALL_CAPS_PARAMS_SIZE
    );
    let reply = policy()
        .respond(&ce_command(NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS))
        .expect("served");
    assert_eq!(reply.body.len(), PARAMS_AT + CE_GET_ALL_CAPS_PARAMS_SIZE);
    let size = u32::from_le_bytes(reply.body[16..20].try_into().expect("4 bytes"));
    assert_eq!(size as usize, CE_GET_ALL_CAPS_PARAMS_SIZE);
}

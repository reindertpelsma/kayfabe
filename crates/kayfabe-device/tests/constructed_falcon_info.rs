//! `NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO` (`0x208001b0`) — the sixth rung, and
//! the first control this port answers by naming **nothing**.
//!
//! ## What this file establishes, in the order it matters
//!
//! 1. **The layout is right**, checked against silicon: the encoder, driven with the eight
//!    falcons a real GA106's GSP reported, reproduces the captured reply byte for byte.
//! 2. **The empty inventory this device ships is a CHOICE, not an incapacity** — the same
//!    encoder expresses the oracle's eight, and the GA106 row declines them. That
//!    distinction is the whole reason this control is not
//!    [`kayfabe_device::inert`]-eligible: the caller pre-zeroes its params
//!    (`ogkm-580: gpu.c:5366`), so an inert reply and our served reply are **the same 1284
//!    bytes**, and only the ability to have said otherwise makes ours a statement.
//! 3. **The count that fails open is unencodable**, and the test that says so names the
//!    exact window: 65..=71.
//!
//! ## Provenance
//!
//! `[measured]` [`ORACLE_PREFIX`] is the reply at record **142009** of
//! `traces/cap1b_coldboot_hermetic_d6.rec`, read at payload offset 120 — the C's own
//! `resp + 120`. The same bytes are the C's `ctl_208001b0`
//! (`C: src/qemu/mode2_initctrl_ga106.h:3878`), and the guest asks twice: record 142016 is
//! byte-identical at `rpc.sequence` 6.
//!
//! ⊘ The capture settles **layout only**. Which falcons to name is decided in
//! [`kayfabe_device::ga10x::GA106_CONSTRUCTED_FALCONS`], against this port's register
//! model, and it decides against all eight.

use kayfabe_abi::falconinfo::{
    self, CONSTRUCTED_FALCON_STRIDE, ConstructedFalcon, FALCON_INFO_PARAMS_SIZE,
    FALCON_REGISTER_BLOCK_LEN, FalconInfoError, FalconInventoryRow, MAX_CONSTRUCTED_FALCONS,
    MAX_CTX_BUFFER_SIZE, NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO,
    RM_DESTINATION_FALCON_SLOTS,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::ChipProfile;
use kayfabe_device::ga10x;
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// The 163 non-zero bytes an RTX 3060's GSP answered: `numConstructedFalcons = 8` and the
/// eight 20-byte entries. Everything after is zero.
const ORACLE_PREFIX: &str = concat!(
    "0800000000bf208a02000000001000000300000000409a0000dce85e02000000",
    "00100000030000000090400000e8814702000000001000000300000000a04100",
    "00e1998f020000000010000003000000008084000022d7f30100000000100000",
    "0300000000a01000006c7be902000000001000000300000000801c000008c428",
    "0200000000000100030000000000840000ab7bdd020000000010000003000000",
    "004084",
);

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "hex must be whole bytes");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

/// The oracle's whole 1284-byte reply: the captured prefix, zero-extended.
fn oracle_params() -> Vec<u8> {
    let mut b = unhex(ORACLE_PREFIX);
    assert_eq!(b.len(), 163, "the captured non-zero prefix");
    b.resize(1284, 0); // literal — the capture's own `paramsSize`
    b
}

/// The deepest byte of the C capture this file's argument rests on: the 4-byte count plus
/// all `numConstructedFalcons = 8` entries it declares, at 20 bytes each.
///
/// ★ `0x208001b0`'s row is TRUNCATED — `dlen` 1280 of `psize` 1284 — so the four bytes it
/// does not carry are the last word of `constructedFalconsTable[63]`, an entry the count
/// puts out of reach. This constant is what makes that sentence fail a build instead of
/// reading well: it is checked against `kayfabe_abi::oracle`'s censused `kept`, and against
/// the reliance statement that module publishes.
const ORACLE_DEEPEST_BYTE: usize = 4 + 8 * 20;

/// ★★★ Nothing this file reads of the oracle's reply is missing from the capture.
///
/// ⊘ Not a claim the bytes are right — [`ORACLE_PREFIX`] is checked against the encoder
/// elsewhere in this file for that. It is the narrower claim the truncation makes
/// necessary: that they came off the recorder rather than out of the zero-extension
/// `oracle_params` performs two functions above.
#[test]
fn every_oracle_byte_this_file_reads_is_inside_what_the_recorder_kept() {
    let cmd = NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO;
    let row = kayfabe_abi::oracle::truncated_row(cmd).expect("0x208001b0 is a truncated row");
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
    // …and the bytes actually embedded here end inside that.
    assert!(unhex(ORACLE_PREFIX).len() <= ORACLE_DEEPEST_BYTE);
    // ⊘ The zero-extension `oracle_params` applies is THIS FILE's, not the recorder's: the
    // capture stops at `kept` and everything after is supplied here.
    assert!(!kayfabe_abi::oracle::field_is_captured(
        0, row.psize, row.kept
    ));
}

/// The eight falcons the oracle's board reported, transcribed from
/// `ogkm-580: g_eng_desc_nvoc.h` class ids rather than from the byte string above.
///
/// ⊘ **Not a row this port would ever ship** — see
/// [`kayfabe_device::ga10x::GA106_CONSTRUCTED_FALCONS`]. It exists so the encoder can be
/// driven over a *populated* table and checked against silicon, and so the emptiness of the
/// shipped row is visibly a decision.
const ORACLE_FALCONS: [ConstructedFalcon; 8] = [
    // `OBJFBFLCN` — the FB falcon.
    falcon(0x8a20_bf00, 2, 0x1000, 3, 0x009a_4000),
    // `FECS` — the GR front-end context-switch microcontroller.
    falcon(0x5ee8_dc00, 2, 0x1000, 3, 0x0040_9000),
    // `GPCCS` — the GPC context-switch microcontroller.
    falcon(0x4781_e800, 2, 0x1000, 3, 0x0041_a000),
    // `OBJBSP` — NVDEC0.
    falcon(0x8f99_e100, 2, 0x1000, 3, 0x0084_8000),
    // `Pmu` — and the only entry whose `ctxAttr` is 1.
    falcon(0xf3d7_2200, 1, 0x1000, 3, 0x0010_a000),
    // `OBJMSENC` — NVENC0.
    falcon(0xe97b_6c00, 2, 0x1000, 3, 0x001c_8000),
    // `OBJSEC2` — and the only entry with a 64 KiB context buffer. RM skips this row
    // wherever a `KernelSec2` exists (`ogkm-580: gpu.c:5384-5390`).
    falcon(0x28c4_0800, 2, 0x1_0000, 3, 0x0084_0000),
    // `OBJOFA`.
    falcon(0xdd7b_ab00, 2, 0x1000, 3, 0x0084_4000),
];

const fn falcon(
    eng_desc: u32,
    ctx_attr: u32,
    ctx_buffer_size: u32,
    addr_space_list: u32,
    register_base: u32,
) -> ConstructedFalcon {
    ConstructedFalcon {
        eng_desc,
        ctx_attr,
        ctx_buffer_size,
        addr_space_list,
        register_base,
    }
}

const ORACLE_ROW: FalconInventoryRow = FalconInventoryRow {
    falcons: &ORACLE_FALCONS,
};

/// GA106's register aperture — 16 MiB, spelled as a literal so the bound under test is not
/// derived from the same place the encoder reads it.
const APERTURE: u64 = 0x0100_0000;

// ── Fixtures needing `'static`: a `const fn` call is not const-promoted. ───────────
/// The same, but one byte off a 4 KiB boundary.
const ONE_UNALIGNED: [ConstructedFalcon; 1] = [falcon(1, 0, 0x1000, 3, 0x0040_9004)];
/// The same, but its register block starts exactly at the end of the aperture.
const ONE_PAST_APERTURE: [ConstructedFalcon; 1] = [falcon(1, 0, 0x1000, 3, 0x0100_0000)];
/// The last 4 KiB block that still fits inside a 16 MiB aperture.
const ONE_LAST_BLOCK: [ConstructedFalcon; 1] = [falcon(1, 0, 0x1000, 3, 0x00ff_f000)];
/// A context buffer one byte above this port's bound.
const ONE_OVERSIZE_CTX: [ConstructedFalcon; 1] =
    [falcon(1, 0, MAX_CTX_BUFFER_SIZE + 1, 3, 0x0040_9000)];
/// The oracle's own largest context buffer (SEC2's 64 KiB), which must still encode.
const ONE_SEC2_SIZED_CTX: [ConstructedFalcon; 1] = [falcon(1, 0, 0x1_0000, 3, 0x0040_9000)];
/// Sixty-five: inside RM's 71-slot check, outside the 64-entry table it indexes.
const SIXTY_FIVE: [ConstructedFalcon; 65] = [falcon(1, 0, 0x1000, 3, 0x0040_9000); 65];

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// `RpcControlReq::HEADER`, as the capture's own arithmetic gives it: the reply's declared
/// `length` was 1356 and `1356 - 32 - 1284 = 40`.
const PARAMS_AT: usize = 40;

/// A `GSP_RM_CONTROL` whose header asks for `cmd` with `params_size` bytes of params.
///
/// ★★ The request body is `0xAA`, not zeros — which matters more here than anywhere else in
/// this port. `gpuBuildGenericKernelFalconList` `portMemSet`s its params to zero before
/// sending (`ogkm-580: gpu.c:5366`), so on the bench a reply that merely *reflected* the
/// request would be indistinguishable from the correct all-zero one. Only a poisoned
/// request can tell them apart.
fn falcon_command(cmd: u32, params_size: u32, serialized: bool) -> RpcCommand {
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

// ── The layout, against silicon ────────────────────────────────────────────────────

#[test]
fn the_encoder_reproduces_the_oracles_layout_byte_for_byte() {
    // ★★★ The provenance test. Two sides that share no source: one is a real GA106's GSP
    // recorded through a driver, the other is a table transcribed from `ogkm-580`'s NVOC
    // class ids and encoded at offsets read from `ctrl2080gpu.h`.
    let got = falconinfo::encode_constructed_falcon_info(&ORACLE_ROW, APERTURE)
        .expect("the oracle's own inventory encodes");
    assert_eq!(got, oracle_params(), "every byte of the 1284-byte reply");
}

#[test]
fn the_layout_constants_are_the_structs_own_arithmetic() {
    // ★ LITERALS. Deriving these from the constants under test would move the check and
    // the encoder together — the defect shape that made #125's and #126's own tests pass
    // while measuring nothing.
    assert_eq!(
        MAX_CONSTRUCTED_FALCONS, 64,
        "NV2080_CTRL_GPU_MAX_…_FALCONS = 0x40"
    );
    assert_eq!(CONSTRUCTED_FALCON_STRIDE, 20, "five NvU32, no padding");
    assert_eq!(
        4 + 64 * 20,
        1284,
        "the capture's own paramsSize: a count then sixty-four entries"
    );
    assert_eq!(FALCON_INFO_PARAMS_SIZE, 1284);
    assert_eq!(
        32 + 40 + 1284,
        1356,
        "and the rpc.length the capture declared"
    );
}

#[test]
fn the_oracles_count_and_its_table_agree_in_the_capture() {
    // Non-vacuity for the fixture: the byte string really does carry eight populated
    // entries and a ninth that is zero, so a test that read the count alone would not be
    // reading a constant it had just written.
    let p = oracle_params();
    assert_eq!(&p[0..4], &8u32.to_le_bytes()[..], "numConstructedFalcons");
    assert_ne!(&p[4..24], &[0u8; 20][..], "entry 0 is populated");
    assert_ne!(&p[4 + 7 * 20..4 + 8 * 20], &[0u8; 20][..], "entry 7 too");
    assert_eq!(&p[4 + 8 * 20..4 + 9 * 20], &[0u8; 20][..], "entry 8 is not");
}

// ── The decision: an empty inventory, and it is a choice ───────────────────────────

#[test]
fn this_device_names_no_falcon_and_the_encoder_could_have_named_eight() {
    // ★★★ **The load-bearing test of this rung.** The served reply is 1284 zero bytes,
    // which is also what an inert reply would be, which is also what the caller's own
    // `portMemSet` already put there. What makes it a *statement* rather than a
    // coincidence is that the same encoder, unchanged, expresses the oracle's eight — so
    // the emptiness is a row's decision and not the encoder's limit.
    assert_eq!(chip().constructed_falcons.count(), 0, "GA106 names none");
    let served = falconinfo::encode_constructed_falcon_info(&chip().constructed_falcons, APERTURE)
        .expect("the empty inventory encodes");
    assert_eq!(served, vec![0u8; 1284], "and it is entirely zero");

    // The other half, without which the assertion above is vacuous.
    let eight = falconinfo::encode_constructed_falcon_info(&ORACLE_ROW, APERTURE)
        .expect("a populated inventory encodes too");
    assert_ne!(eight, served, "the encoder is not a zero generator");
    assert_eq!(&eight[0..4], &8u32.to_le_bytes()[..]);
}

#[test]
fn no_falcon_the_oracle_names_has_a_register_model_this_port_could_serve() {
    // ★★★ **The measurement the decision rests on** — and it is THIS test, not a prose
    // claim: `tests/constructed_falcon_info.rs` is where the number comes from, so it goes
    // red rather than stale. `decode_reg` returning `None` is what `RegPlane` answers with
    // a defaulted zero, and a `GenericKernelFalcon` is precisely an object that reads
    // those registers (`ogkm-580: kernel_falcon_tu102.c:45-76`).
    //
    // ⚠ **This test corrected the claim it was written to confirm.** The first version
    // asserted zero modelled registers for all eight and went red on `NV_PSEC`: this tree
    // decodes THREE offsets of the SEC2 falcon block. So the true statement is narrower
    // than "none is modelled" — it is *"none is modelled as a falcon"*. The three are the
    // FWSEC secure-booter handshake and nothing else; they are pinned by name below, so
    // modelling a fourth turns this red rather than silently widening what the row above
    // could honestly advertise.
    //
    // ⊘ Quantified over `ORACLE_FALCONS`, not over a shorter hand-written list.
    let model = (chip().gsp_model)();
    // The published `NV_PFALCON_FALCON_*` block, `0x000..=0x130`
    // (`ogkm-580: dev_falcon_v4.h`), at dword steps.
    let claimed = |base: u64| -> usize {
        (0..=0x130u64)
            .step_by(4)
            .filter(|o| model.decode_reg(0, base + o).is_some())
            .count()
    };
    let mut examined = 0;
    for f in &ORACLE_FALCONS {
        let base = u64::from(f.register_base);
        let n = claimed(base);
        if base == 0x0084_0000 {
            assert_eq!(
                n, 3,
                "NV_PSEC models exactly the three FWSEC offsets; a fourth means this port \
                 now has a falcon model and the GA106 row's reasoning must be revisited"
            );
        } else {
            assert_eq!(
                n, 0,
                "the falcon block at {base:#x} has no model, so every register RM read \
                 through it would be a defaulted zero"
            );
        }
        examined += 1;
    }
    assert_eq!(examined, 8, "non-vacuity: all eight were examined");

    // ★ What the three SEC2 offsets ARE, so "not a falcon model" is checked and not
    // asserted: a mailbox, a CPU control and a DMA transfer command — the FWSEC
    // secure-booter handshake. None of `_HWCFG2`, `_IRQSTAT`, `_IRQMASK` or `_IRQDEST`,
    // which is what an `IntrService`-registered GenericKernelFalcon would read first.
    for off in [0x0f4u64, 0x008, 0x018, 0x01c] {
        assert!(
            model.decode_reg(0, 0x0084_0000 + off).is_none(),
            "NV_PSEC+{off:#x} is one of the registers a constructed falcon reads, and this \
             port does not model it"
        );
    }

    // And the model is not simply refusing everything — it claims the GSP block it does
    // own, so the zeros above are a fact about those windows and not about the probe.
    assert!(
        model.decode_reg(0, 0x0011_0c00).is_some(),
        "NV_PGSP_QUEUE_HEAD(0) is modelled; if it were not, the loop above would pass \
         against a model that decodes nothing at all"
    );
}

// ── The field combination that fails open ──────────────────────────────────────────

#[test]
fn a_count_rms_own_bounds_check_would_let_through_is_unencodable() {
    // ★★★ **The fail-open.** `gpuBuildGenericKernelFalconList` bounds the count against
    // its 71-slot DESTINATION array (`ogkm-580: gpu.c:5375-5377`) while the SOURCE table it
    // then indexes holds 64. So 65..=71 passes RM's assert and reads up to 140 bytes past
    // the 1284-byte params buffer, constructing falcons — with their unvalidated
    // `registerBase` — out of adjacent guest kernel heap.
    //
    // Quantified over the WHOLE window, not sampled at one end.
    for n in (MAX_CONSTRUCTED_FALCONS + 1)..=RM_DESTINATION_FALCON_SLOTS {
        let rows: Vec<ConstructedFalcon> = vec![falcon(1, 0, 0x1000, 3, 0x0040_9000); n];
        let row = FalconInventoryRow {
            falcons: Box::leak(rows.into_boxed_slice()),
        };
        assert_eq!(
            falconinfo::encode_constructed_falcon_info(&row, APERTURE)
                .expect_err("RM would accept this count and read past its own buffer"),
            FalconInfoError::TooManyFalcons {
                count: n,
                max: MAX_CONSTRUCTED_FALCONS
            }
        );
    }
    // Non-vacuity at the boundary: 64 is inside the struct and must still encode, or the
    // test above would pass for a trivially refusing encoder.
    let ok: Vec<ConstructedFalcon> =
        vec![falcon(1, 0, 0x1000, 3, 0x0040_9000); MAX_CONSTRUCTED_FALCONS];
    let row = FalconInventoryRow {
        falcons: Box::leak(ok.into_boxed_slice()),
    };
    let bytes = falconinfo::encode_constructed_falcon_info(&row, APERTURE)
        .expect("a full table is inside the struct");
    assert_eq!(&bytes[0..4], &64u32.to_le_bytes()[..]);
    assert_eq!(bytes.len(), 1284, "and it still fits exactly");
}

#[test]
fn a_declared_count_cannot_disagree_with_the_table_because_there_is_no_declared_count() {
    // ★★ The structural half, and the reason the check above is a bound rather than a
    // cross-check. `FalconInventoryRow` carries no count field: the encoder derives
    // `numConstructedFalcons` from the slice. A reply claiming eight entries while
    // carrying one is not rejected — it cannot be written down.
    for n in [0usize, 1, 3, 8, 64] {
        let rows: Vec<ConstructedFalcon> = vec![falcon(7, 0, 0x1000, 3, 0x0040_9000); n];
        let row = FalconInventoryRow {
            falcons: Box::leak(rows.into_boxed_slice()),
        };
        let b = falconinfo::encode_constructed_falcon_info(&row, APERTURE).expect("encodes");
        assert_eq!(
            u32::from_le_bytes(b[0..4].try_into().expect("4 bytes")),
            n as u32,
            "the count is the table's length, always"
        );
        // and the entry after the last one is zero, so the table's extent is unambiguous
        if n < 64 {
            let at = 4 + n * 20;
            assert_eq!(&b[at..at + 20], &[0u8; 20][..]);
        }
    }
}

#[test]
fn a_register_base_outside_this_devices_aperture_is_unencodable() {
    // RM validates `registerBase` nowhere — it goes straight to BAR0 MMIO
    // (`ogkm-580: kernel_falcon_tu102.c:45-76`), reachable from the interrupt path.
    let row = FalconInventoryRow {
        falcons: &ONE_PAST_APERTURE,
    };
    assert_eq!(
        falconinfo::encode_constructed_falcon_info(&row, APERTURE).expect_err("refused"),
        FalconInfoError::RegisterBaseOutsideAperture {
            register_base: 0x0100_0000,
            aperture_len: APERTURE
        }
    );
    // The last block that DOES fit, so the bound is exact rather than merely conservative.
    let last = FalconInventoryRow {
        falcons: &ONE_LAST_BLOCK,
    };
    assert!(falconinfo::encode_constructed_falcon_info(&last, APERTURE).is_ok());
}

#[test]
fn an_unaligned_register_base_is_unencodable() {
    let row = FalconInventoryRow {
        falcons: &ONE_UNALIGNED,
    };
    assert_eq!(
        falconinfo::encode_constructed_falcon_info(&row, APERTURE).expect_err("refused"),
        FalconInfoError::RegisterBaseUnaligned {
            register_base: 0x0040_9004
        }
    );
    assert_eq!(FALCON_REGISTER_BLOCK_LEN, 0x1000, "literal");
}

#[test]
fn a_context_buffer_larger_than_this_port_will_request_is_unencodable() {
    // `ctxBufferSize` reaches `memdescCreate` unvalidated
    // (`ogkm-580: kernel_falcon.c:144-155`), so it is a guest-kernel allocation size a GSP
    // chooses. ⚠ The bound is this port's policy, not a driver limit — see the constant.
    let row = FalconInventoryRow {
        falcons: &ONE_OVERSIZE_CTX,
    };
    assert_eq!(
        falconinfo::encode_constructed_falcon_info(&row, APERTURE).expect_err("refused"),
        FalconInfoError::ContextBufferTooLarge {
            ctx_buffer_size: MAX_CTX_BUFFER_SIZE + 1,
            max: MAX_CTX_BUFFER_SIZE
        }
    );
    // Non-vacuity: the oracle's own largest (SEC2's 64 KiB) is comfortably inside.
    let ok = FalconInventoryRow {
        falcons: &ONE_SEC2_SIZED_CTX,
    };
    assert!(falconinfo::encode_constructed_falcon_info(&ok, APERTURE).is_ok());
}

// ── The serve site ─────────────────────────────────────────────────────────────────

#[test]
fn the_policy_answers_the_control_without_reflecting_one_byte_of_the_request() {
    let mut p = policy();
    let reply = p
        .respond(&falcon_command(
            NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO,
            1284,
            false,
        ))
        .expect("this port serves the falcon inventory");
    assert_eq!(reply.rpc_result, 0, "NV_OK in the envelope");
    let params = &reply.body[PARAMS_AT..PARAMS_AT + 1284];
    assert_eq!(params, &vec![0u8; 1284][..], "an empty inventory, in full");
    assert!(
        !params.contains(&0xAA),
        "not one poisoned request byte survived into the reply"
    );
    // The header the guest reads before it copies anything out.
    assert_eq!(
        &reply.body[12..16],
        &0u32.to_le_bytes()[..],
        "status = NV_OK"
    );
    assert_eq!(
        &reply.body[16..20],
        &1284u32.to_le_bytes()[..],
        "paramsSize, rewritten to what we encoded"
    );
}

#[test]
fn the_serve_site_refuses_when_the_encoder_declines() {
    // ★★★ The fail-open guard, at the layer that would actually widen. A chip row whose
    // inventory the encoder refuses must produce a REFUSAL, not a best-effort reply —
    // answering anyway is exactly the out-of-bounds construction the encoder exists to
    // prevent.
    static BAD: ChipProfile = ChipProfile {
        memory_system: kayfabe_device::ga10x::GA106_MEMORY_SYSTEM,
        device_info: kayfabe_device::ga10x::GA106_DEVICE_INFO,
        conf_compute: kayfabe_device::ga10x::GA106_CONF_COMPUTE,
        bif_static: kayfabe_device::ga10x::GA106_BIF_STATIC,
        fifo_channels: kayfabe_device::ga10x::GA106_FIFO_CHANNELS,
        gmmu_static: kayfabe_device::ga10x::GA106_GMMU_STATIC,
        gr_static: kayfabe_abi::grstatic::GA106_GR_STATIC,
        gr_info: kayfabe_abi::grinfo::GA106_GR_INFO,
        gr_context_buffers: kayfabe_abi::grstatic::GA106_CONTEXT_BUFFERS,
        forwarded_gpu_info: kayfabe_abi::gpuinfo::GA106_FORWARDED_GPU_INFO,
        constructed_falcons: FalconInventoryRow {
            // Inside RM's 71-slot check, outside the 64-entry table it indexes.
            falcons: &SIXTY_FIVE,
        },
        ..copy_of_ga106()
    };
    let mut p = InitTablePolicy::new(&BAD, *table_for(BENCH_DRIVER).expect("bench ABI"));
    let reply = p
        .respond(&falcon_command(
            NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO,
            1284,
            false,
        ))
        .expect("answers, loudly");
    assert_ne!(reply.rpc_result, 0, "NV_ERR_NOT_SUPPORTED in the envelope");
    assert!(reply.body.is_empty(), "and no body to misread");
}

#[test]
fn a_declared_params_size_that_is_not_ours_is_refused_rather_than_answered() {
    for size in [0u32, 4, 1280, 1288, 8204] {
        let mut p = policy();
        let reply = p
            .respond(&falcon_command(
                NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO,
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
        .respond(&falcon_command(
            NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO,
            1284,
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
        WantedTable::from_cmd(NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO),
        Some(WantedTable::ConstructedFalconInfo)
    );
    assert_eq!(
        WantedTable::ConstructedFalconInfo.cmd_id(),
        0x2080_01b0,
        "literal — the id the capture carries"
    );
    assert_eq!(WantedTable::ConstructedFalconInfo.params_size(), 1284);
    assert!(
        WantedTable::ALL.contains(&WantedTable::ConstructedFalconInfo),
        "and it is in the universe every coverage gate quantifies over"
    );
}

/// A `ChipProfile` identical to GA106 in everything the test above does not override.
const fn copy_of_ga106() -> ChipProfile {
    ChipProfile {
        name: "TEST-BAD-FALCONS",
        pci_device_id: 0x2504,
        pci_revision: 0xa1,
        pci_subsystem_vendor_id: 0x1462,
        pci_subsystem_id: 0x397D,
        regs_aperture_len: APERTURE,
        pci_bars: &[kayfabe_abi::pcibars::PciBarRow {
            name: "registers",
            size_bytes: APERTURE,
        }],
        boot_regs: &[],
        ptimer: kayfabe_device::PtimerRegs {
            lo_off: 0x9400,
            hi_off: 0x9410,
        },
        rom_window: kayfabe_device::RomWindow {
            base: 0x30_0000,
            len: 0x2_0000,
        },
        pramin_window: kayfabe_device::RegSpan {
            base: 0x0070_0000,
            len: 0x0010_0000,
        },
        bar0_window_reg: 0x0000_1700,
        vbios_wire: kayfabe_abi::vbios::VbiosWire::Tu102Bit,
        msix_vectors: 1,
        ce_fault_method_buffer_size: kayfabe_abi::fmbsize::GA106_CE_FAULT_METHOD_BUFFER_SIZE,
        gsp_model: || Box::new(ga10x::Ga10xGspModel::new()),
        engines: &[],
        intr_table: &[],
        intr_subtree_map: [0; kayfabe_abi::inittables::INTR_CATEGORY_COUNT],
        fb_regions: &[],
        chip_info: kayfabe_abi::chipinfo::ChipInfoRow {
            chip_sub_rev: 0,
            is_cmp_sku: false,
            reg_bases: &[],
        },
        user_register_access_map: kayfabe_abi::regaccessmap::RegisterAccessMapRow::NOT_PUBLISHED,
        memory_system: kayfabe_device::ga10x::GA106_MEMORY_SYSTEM,
        device_info: kayfabe_device::ga10x::GA106_DEVICE_INFO,
        conf_compute: kayfabe_device::ga10x::GA106_CONF_COMPUTE,
        bif_static: kayfabe_device::ga10x::GA106_BIF_STATIC,
        fifo_channels: kayfabe_device::ga10x::GA106_FIFO_CHANNELS,
        gmmu_static: kayfabe_device::ga10x::GA106_GMMU_STATIC,
        gr_static: kayfabe_abi::grstatic::GA106_GR_STATIC,
        gr_info: kayfabe_abi::grinfo::GA106_GR_INFO,
        gr_context_buffers: kayfabe_abi::grstatic::GA106_CONTEXT_BUFFERS,
        forwarded_gpu_info: kayfabe_abi::gpuinfo::GA106_FORWARDED_GPU_INFO,
        constructed_falcons: FalconInventoryRow::NONE,
        fb_length: 0,
    }
}

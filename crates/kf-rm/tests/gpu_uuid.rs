// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The per-VM GPU UUID (`kf_rm::gpuuid`, `docs/design/V3_GPU_UUID.md`): the modes, the exact `auto`
//! construction, the hostile-string refusals, the host query, and — the property the whole feature
//! is for — that EVERY site that puts the UUID in front of the guest puts the same bytes there.
//!
//! GPU-free: the host is a scripted [`HostControls`], the device a [`StaticInfoPolicy`].

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::DriverVersion;
use kf_abi::gspstaticinfo::{GID_INFO_SIZE, GpuGid, GpuGidTextError, RM_SHA1_GID_SIZE};
use kf_abi::matrix::Resolved;
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::BoardFacts;
use kf_rm::gpuuid::{
    AUTO_TAG, Basis, GpuUuidError, GpuUuidMode, Inputs, auto_gid, os_entropy, parse_vm_id, resolve,
};
use kf_rm::hostquery::{HostControls, HostRefusal};
use kf_rm::staticinfo::StaticInfoPolicy;
use std::sync::Arc;

const VM_A: [u8; 16] = [0x11; 16];
const VM_B: [u8; 16] = [0x22; 16];

fn gid(b: u8) -> GpuGid {
    GpuGid::from_bytes([b; 16]).expect("non-zero")
}

/// An entropy source that must not be called.
fn no_entropy(_: &mut [u8; 16]) -> Result<(), String> {
    panic!("entropy asked for by a mode that is not random")
}

fn auto(vm: Option<[u8; 16]>, slot: u8, host: Option<GpuGid>) -> Inputs {
    Inputs {
        mode: GpuUuidMode::Auto,
        vm_id: vm,
        slot,
        host_gid: host,
    }
}

// ── the construction ────────────────────────────────────────────────────────────────

/// ★ The exact construction, pinned to a value computed OUTSIDE this repository (Python's
/// `hashlib.sha256(tag + vm + host + slot)[:16]`), so a refactor of the hash, the tag, the field
/// order or the slot width cannot silently re-identify every VM.
#[test]
fn auto_is_the_documented_sha256_construction() {
    assert_eq!(AUTO_TAG, b"kayfabe/gpu-uuid/auto/v1\0");
    let vm = [
        0x11, 0x11, 0x11, 0x11, 0x22, 0x22, 0x33, 0x33, 0x44, 0x44, 0x55, 0x55, 0x55, 0x55, 0x55,
        0x55,
    ];
    let host = GpuGid::parse("GPU-51b08678-7828-4015-1962-a65a7a488e3c").expect("text");
    let got = auto_gid(&vm, &host, 0x18).expect("non-zero");
    assert_eq!(got.to_string(), "GPU-0dd6c866-82ed-89af-c122-52cc0dae4c6f");
}

#[test]
fn auto_is_stable_and_every_input_matters() {
    let h = gid(0xAA);
    let base = auto_gid(&VM_A, &h, 3).unwrap();
    assert_eq!(
        auto_gid(&VM_A, &h, 3).unwrap(),
        base,
        "stable across calls (boots)"
    );
    assert_ne!(auto_gid(&VM_B, &h, 3).unwrap(), base, "another VM");
    assert_ne!(
        auto_gid(&VM_A, &gid(0xBB), 3).unwrap(),
        base,
        "another host GPU"
    );
    assert_ne!(auto_gid(&VM_A, &h, 4).unwrap(), base, "another slot");
    // The fields are fixed-width and in a fixed order: swapping vm and host is a different id.
    assert_ne!(
        auto_gid(h.as_bytes(), &GpuGid::from_bytes(VM_A).unwrap(), 3).unwrap(),
        base
    );
    // And the result is never the host's own UUID (the point of `auto` is not to leak it).
    assert_ne!(base, h);
}

// ── the modes ───────────────────────────────────────────────────────────────────────

#[test]
fn auto_resolves_without_entropy_and_without_warning() {
    let r = resolve(&auto(Some(VM_A), 3, Some(gid(0xAA))), &mut no_entropy).unwrap();
    assert_eq!(r.basis, Basis::Auto);
    assert_eq!(r.gid, auto_gid(&VM_A, &gid(0xAA), 3).unwrap());
    assert!(r.warnings.is_empty());
}

/// Two devices of the SAME chip row on the same host GPU, in two VMs: different UUIDs (this was
/// the collision). Same VM and host GPU on a second boot: the same UUID.
#[test]
fn two_vms_on_one_chip_row_differ_and_one_vm_is_stable_across_boots() {
    let host = Some(gid(0xAA)); // the one host GPU both VMs run on
    let vm1 = resolve(&auto(Some(VM_A), 3, host), &mut no_entropy)
        .unwrap()
        .gid;
    let vm2 = resolve(&auto(Some(VM_B), 3, host), &mut no_entropy)
        .unwrap()
        .gid;
    let vm1_again = resolve(&auto(Some(VM_A), 3, host), &mut no_entropy)
        .unwrap()
        .gid;
    assert_ne!(vm1, vm2);
    assert_eq!(vm1, vm1_again);
    // The old derivation gave both the same value: that is the defect.
    let chip = StaticInfoPolicy::gid_for_board(&ga106::board());
    assert_ne!(vm1, chip);
    assert_ne!(vm2, chip);
}

#[test]
fn auto_without_a_vm_identity_is_random_and_says_so() {
    let mut n = 0u8;
    let mut counter = |b: &mut [u8; 16]| {
        n += 1;
        *b = [n; 16];
        Ok(())
    };
    let a = resolve(&auto(None, 3, Some(gid(0xAA))), &mut counter).unwrap();
    let b = resolve(&auto(None, 3, Some(gid(0xAA))), &mut counter).unwrap();
    assert_eq!(a.basis, Basis::RandomNoVmId);
    assert_ne!(a.gid, b.gid);
    assert!(a.warnings.len() == 1 && a.warnings[0].contains("no identity"));
    // It does not need the host UUID, either: the fallback has no inputs to hash.
    assert!(resolve(&auto(None, 3, None), &mut counter).is_ok());
}

#[test]
fn auto_and_host_without_a_host_uuid_refuse_by_name() {
    let e = resolve(&auto(Some(VM_A), 3, None), &mut no_entropy).unwrap_err();
    assert_eq!(e, GpuUuidError::HostUuidUnavailable { mode: "auto" });
    assert!(e.to_string().contains("gpu-uuid=auto"));
    let host = Inputs {
        mode: GpuUuidMode::Host,
        vm_id: Some(VM_A),
        slot: 0,
        host_gid: None,
    };
    let e = resolve(&host, &mut no_entropy).unwrap_err();
    assert!(e.to_string().contains("gpu-uuid=host"), "{e}");
}

#[test]
fn random_differs_between_boots_and_uses_only_the_entropy_it_is_given() {
    let mode = |m| Inputs {
        mode: m,
        vm_id: Some(VM_A),
        slot: 3,
        host_gid: None, // random must not need the host's
    };
    let mut seq = 0u8;
    let mut src = |b: &mut [u8; 16]| {
        seq += 1;
        *b = [seq; 16];
        Ok(())
    };
    let boot1 = resolve(&mode(GpuUuidMode::Random), &mut src).unwrap();
    let boot2 = resolve(&mode(GpuUuidMode::Random), &mut src).unwrap();
    assert_eq!(boot1.basis, Basis::Random);
    assert_ne!(boot1.gid, boot2.gid);
    // The real source: two draws differ (2^-128 false-failure), and are never zero.
    let (mut x, mut y) = ([0u8; 16], [0u8; 16]);
    os_entropy(&mut x).unwrap();
    os_entropy(&mut y).unwrap();
    assert_ne!(x, y);
}

#[test]
fn random_refuses_a_failed_or_all_zero_source() {
    let inp = Inputs {
        mode: GpuUuidMode::Random,
        vm_id: None,
        slot: 0,
        host_gid: None,
    };
    let e = resolve(&inp, &mut |_| Err("no urandom".into())).unwrap_err();
    assert_eq!(e, GpuUuidError::Entropy("no urandom".into()));
    let e = resolve(&inp, &mut |b| {
        *b = [0; 16];
        Ok(())
    })
    .unwrap_err();
    assert_eq!(e, GpuUuidError::ZeroResult);
}

#[test]
fn host_serves_the_host_uuid_with_a_warning() {
    let h = gid(0xAA);
    let inp = Inputs {
        mode: GpuUuidMode::Host,
        vm_id: Some(VM_A),
        slot: 3,
        host_gid: Some(h),
    };
    let r = resolve(&inp, &mut no_entropy).unwrap();
    assert_eq!(r.gid, h);
    assert_eq!(r.basis, Basis::Host);
    assert!(r.warnings[0].contains("gpu-uuid=host") && r.warnings[0].contains("HOST"));
}

#[test]
fn explicit_round_trips_including_the_gpu_text_format() {
    let text = "GPU-51b08678-7828-4015-1962-a65a7a488e3c";
    let mode = GpuUuidMode::parse(Some(text)).unwrap();
    assert_eq!(mode.label(), "explicit");
    let inp = Inputs {
        mode,
        vm_id: None,
        slot: 0,
        host_gid: None,
    };
    let r = resolve(&inp, &mut no_entropy).unwrap();
    assert_eq!(r.basis, Basis::Explicit);
    assert_eq!(r.gid.to_string(), text);
    assert!(r.warnings.is_empty());
    // The bare and the upper-case spellings are the same value.
    for alt in [
        "51b08678-7828-4015-1962-a65a7a488e3c",
        "51b08678782840151962a65a7a488e3c",
        "GPU-51B08678-7828-4015-1962-A65A7A488E3C",
    ] {
        assert_eq!(GpuUuidMode::parse(Some(alt)), Ok(mode), "{alt}");
    }
}

// ── hostile property strings ────────────────────────────────────────────────────────

#[test]
fn the_mode_words_are_exact() {
    assert_eq!(GpuUuidMode::parse(None), Ok(GpuUuidMode::Auto));
    assert_eq!(GpuUuidMode::parse(Some("")), Ok(GpuUuidMode::Auto));
    assert_eq!(GpuUuidMode::parse(Some("auto")), Ok(GpuUuidMode::Auto));
    assert_eq!(GpuUuidMode::parse(Some("random")), Ok(GpuUuidMode::Random));
    assert_eq!(GpuUuidMode::parse(Some("host")), Ok(GpuUuidMode::Host));
    for bad in [
        "Auto", "RANDOM", "host ", " host", "random\n", "autos", "on", "off", "none",
    ] {
        assert!(GpuUuidMode::parse(Some(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn hostile_strings_are_refused_without_being_echoed() {
    let huge = "A".repeat(1 << 20);
    let hostile = [
        "GPU-00000000-0000-0000-0000-000000000000",
        "00000000000000000000000000000000",
        "GPU-51b08678-7828-4015-1962-a65a7a488e3",
        "GPU-51b08678-7828-4015-1962-a65a7a488e3cc",
        "GPU-51b08678-7828-4015-1962-a65a7a488e3z",
        "GPU-51b08678_7828_4015_1962_a65a7a488e3c",
        "GPU-51b08678-7828-4015-1962-a65a7a488e3c\n",
        "GPU-51b08678-7828-4015-1962-a65a7a488e3c\0",
        "-GPU-51b08678-7828-4015-1962-a65a7a488e3c",
        "GPU-GPU-51b08678-7828-4015-1962-a65a7a488e3c",
        "0x51b08678782840151962a65a7a488e3c",
        "../../etc/passwd",
        "%n%n%n%n",
        "\u{ff10}\u{ff10}\u{ff10}",
        &huge,
    ];
    for h in hostile {
        let e = GpuUuidMode::parse(Some(h)).expect_err(&h[..h.len().min(48)]);
        let msg = e.to_string();
        assert!(msg.contains("gpu-uuid="), "names the property: {msg}");
        assert!(
            msg.len() < 400,
            "a refusal is bounded, however long the input: {}",
            msg.len()
        );
        assert!(
            !msg.contains("etc/passwd") && !msg.contains("%n"),
            "input is not echoed: {msg}"
        );
    }
    // All-zero is refused as such, not as "malformed".
    assert!(matches!(
        GpuUuidMode::parse(Some("GPU-00000000-0000-0000-0000-000000000000")),
        Err(GpuUuidError::BadMode {
            why: GpuGidTextError::AllZero,
            ..
        })
    ));
}

#[test]
fn vm_id_parses_refuses_and_treats_zero_as_no_identity() {
    assert_eq!(parse_vm_id(None), Ok(None));
    assert_eq!(parse_vm_id(Some("")), Ok(None));
    // QEMU's -uuid unset is all-zero: it identifies no VM.
    assert_eq!(
        parse_vm_id(Some("00000000-0000-0000-0000-000000000000")),
        Ok(None)
    );
    assert_eq!(
        parse_vm_id(Some("11111111-1111-1111-1111-111111111111")),
        Ok(Some(VM_A))
    );
    for bad in [
        "vm1",
        "GPU-11111111-1111-1111-1111-111111111111",
        "1111",
        "11111111-1111-1111-1111-11111111111g",
    ] {
        let e = parse_vm_id(Some(bad)).expect_err(bad);
        assert!(e.to_string().contains("vm-id="), "{e}");
    }
}

// ── the host query ──────────────────────────────────────────────────────────────────

/// A host that answers `GPU_GET_GID_INFO` as the real RM does: `length = 16`, `data[0..16]`.
struct Host {
    uuid: [u8; 16],
    length: u32,
    refuse: bool,
    asked: Vec<(u32, u32, u32)>,
}

impl HostControls for Host {
    fn control(&mut self, cmd: u32, p: &mut [u8]) -> Result<(), HostRefusal> {
        assert_eq!(cmd, 0x2080_014a, "only the GID control is asked");
        assert_eq!(p.len(), GID_INFO_SIZE);
        let w = |o: usize| u32::from_le_bytes(p[o..o + 4].try_into().unwrap());
        self.asked.push((w(0), w(4), w(8)));
        if self.refuse {
            return Err(HostRefusal {
                status: Some(0x56),
                detail: "refused".into(),
            });
        }
        p[8..12].copy_from_slice(&self.length.to_le_bytes());
        p[12..28].copy_from_slice(&self.uuid);
        Ok(())
    }
}

#[test]
fn the_host_uuid_is_asked_unprivileged_as_binary_sha1_and_decoded() {
    let mut h = Host {
        uuid: [0xAA; 16],
        length: 16,
        refuse: false,
        asked: vec![],
    };
    let got = kf_rm::hostquery::query_host_gid(&mut h).unwrap();
    assert_eq!(got, Some(gid(0xAA)));
    // index 0, flags FORMAT_BINARY | TYPE_SHA1 = 2, length untouched.
    assert_eq!(h.asked, vec![(0, 2, 0)]);
}

#[test]
fn a_host_that_refuses_or_misreports_the_uuid_yields_none_not_a_realize_failure() {
    for (refuse, length, uuid) in [
        (true, 16, [0xAA; 16]),
        (false, 8, [0xAA; 16]),
        (false, 256, [0xAA; 16]),
        (false, 16, [0; 16]),
    ] {
        let mut h = Host {
            uuid,
            length,
            refuse,
            asked: vec![],
        };
        assert_eq!(
            kf_rm::hostquery::query_host_gid(&mut h).unwrap(),
            None,
            "refuse={refuse} length={length} uuid={uuid:?}"
        );
    }
}

#[test]
fn the_host_uuid_has_exactly_one_provenance_row() {
    let rows: Vec<_> = kf_rm::hostfacts::PROVENANCE
        .iter()
        .filter(|(n, _)| *n == "host_gid")
        .collect();
    assert_eq!(rows.len(), 1);
    assert!(matches!(
        rows[0].1,
        kf_rm::hostfacts::Source::HostControl {
            cmd: 0x2080_014a,
            ..
        }
    ));
}

// ── ★ the sites agree ───────────────────────────────────────────────────────────────

fn board_with(g: Option<GpuGid>) -> Arc<BoardFacts> {
    Arc::new(BoardFacts {
        gpu_gid: g,
        ..ga106::board_at(ga106::FB_SIZE_MB)
    })
}

fn fn65(v: DriverVersion) -> RpcCommand {
    let size = Resolved::of(&kf_abi::generated::matrix::GSPSTATICCONFIGINFO, v)
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

/// The 16 bytes at `gidInfo.data` of `body`, at `v`'s own layout.
fn gid_in(body: &[u8], v: DriverVersion) -> [u8; 16] {
    let at = Resolved::of(&kf_abi::generated::matrix::GSPSTATICCONFIGINFO, v)
        .unwrap()
        .need("gidInfo.data")
        .unwrap()
        .off();
    body[at..at + RM_SHA1_GID_SIZE].try_into().unwrap()
}

/// ★ The sites a guest can read the UUID from, as this port serves it:
///
/// 1. [`StaticInfoPolicy::body`] — the bench-layout encoder;
/// 2. [`StaticInfoPolicy::body_measured`] — the encoder at the layout of the guest's driver version;
/// 3. the fn-65 reply (`respond`), which is the only way the guest receives it;
/// 4. the full production chain (`served_policy`: census → sticky guard → `served_chain`),
///    which is what the device installs;
/// 5. [`StaticInfoPolicy::gid`].
///
/// Everything the guest then shows (`NV2080_CTRL_CMD_GPU_GET_GID_INFO`, NVML's `GPU-…`,
/// `nvidia-smi -L`, UVM's GPU UUID, `GET_UUID_FROM_GPU_ID`) is computed by the guest's own RM
/// from site 3 — the closed physical RM is the only other producer of a UUID and a GSP client
/// has none (`kf_abi::gspstaticinfo::GpuGid`) — so there is no second place for this port to
/// state it. This test pins that all of OUR sites carry the declared bytes, at every driver
/// version the matrix covers, and that the chip-row value and the host's value are absent.
#[test]
fn every_site_carries_the_one_declared_uuid_at_every_measured_version() {
    let declared = auto_gid(&VM_A, &gid(0xAA), 3).unwrap();
    let host = gid(0xAA);
    let chip_row = StaticInfoPolicy::gid_for_board(&ga106::board());
    let board = board_with(Some(declared));
    let mut checked = 0;
    for &v in kf_abi::generated::matrix::MEASURED {
        let (Ok(table), Ok(_)) = (
            table_for(v),
            Resolved::of(&kf_abi::generated::matrix::GSPSTATICCONFIGINFO, v),
        ) else {
            continue;
        };
        let mut p = StaticInfoPolicy::new(board.clone(), *table);
        assert_eq!(p.gid(), declared, "{v}: gid()");

        let measured = p.body_measured().expect("encodes");
        assert_eq!(
            gid_in(&measured, v),
            *declared.as_bytes(),
            "{v}: body_measured"
        );

        let reply = p.respond(&fn65(v)).expect("fn 65 is answered");
        assert_eq!(reply.rpc_result, 0, "{v}");
        assert_eq!(
            gid_in(&reply.body, v),
            *declared.as_bytes(),
            "{v}: the fn-65 reply"
        );

        // The production chain, built the way the device builds it.
        let mut chain = kf_rm::served_policy(
            board.clone(),
            ga106::host(),
            *table,
            kf_rm::ChainLogs::default(),
            kf_rm::census::ControlCensusLog::new(),
            kf_rm::ObjectLinks::default(),
        );
        let served = chain.respond(&fn65(v)).expect("the chain answers fn 65");
        assert_eq!(
            gid_in(&served.body, v),
            *declared.as_bytes(),
            "{v}: the production chain"
        );
        // (The chain's body is not the bare policy's: the chain also states the host's engine
        // caps. The UUID is what must be equal.)

        // Neither the chip-row value nor the host GPU's own UUID is anywhere in the body.
        for (what, g) in [("chip-row", chip_row), ("host", host)] {
            assert!(
                !served
                    .body
                    .windows(RM_SHA1_GID_SIZE)
                    .any(|w| w == g.as_bytes()),
                "{v}: the {what} UUID leaked into fn 65"
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 3,
        "the loop must cover several layouts, covered {checked}"
    );

    // The bench-layout encoder (site 1) at the bench driver.
    let p = StaticInfoPolicy::new(board, *table_for(BENCH_DRIVER).unwrap());
    let body = p.body().unwrap();
    assert_eq!(
        &body[kf_abi::gspstaticinfo::GID_DATA_OFF..kf_abi::gspstaticinfo::GID_DATA_OFF + 16],
        declared.as_bytes(),
        "body()"
    );
}

/// Two devices of the same chip row in two VMs serve different fn-65 bodies, which differ only in
/// the UUID; an undeclared board keeps today's per-chip-row value byte for byte.
#[test]
fn two_vms_on_one_chip_row_serve_different_bodies_and_an_undeclared_board_is_unchanged() {
    let t = *table_for(BENCH_DRIVER).unwrap();
    let a = auto_gid(&VM_A, &gid(0xAA), 3).unwrap();
    let b = auto_gid(&VM_B, &gid(0xAA), 3).unwrap();
    let body_a = StaticInfoPolicy::new(board_with(Some(a)), t)
        .body()
        .unwrap();
    let body_b = StaticInfoPolicy::new(board_with(Some(b)), t)
        .body()
        .unwrap();
    assert_ne!(body_a, body_b);
    let diff: Vec<_> = (0..body_a.len())
        .filter(|&i| body_a[i] != body_b[i])
        .collect();
    assert!(
        diff.iter().all(|&i| (36..52).contains(&i)),
        "only gidInfo.data[0..16] may differ: {diff:?}"
    );

    let undeclared = StaticInfoPolicy::new(board_with(None), t);
    assert_eq!(
        undeclared.gid(),
        StaticInfoPolicy::gid_for_board(&ga106::board())
    );
    assert_eq!(
        undeclared.body().unwrap(),
        StaticInfoPolicy::new(ga106::board(), t).body().unwrap()
    );
}

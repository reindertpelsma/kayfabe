use kf_abi::{
    DriverVersion,
    grbind::{Kind, Request, decode},
};
fn version(major: u16) -> DriverVersion {
    DriverVersion {
        major,
        minor: if major == 580 { 159 } else { 84 },
        patch: if major == 580 { 4 } else { 0 },
    }
}
fn set(p: &mut [u8], at: usize, v: u32) {
    p[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
#[test]
fn preemption_union_decodes_gfxp_mode_and_all_nine_pointers() {
    for major in [580, 595] {
        let mut p = vec![0; 256];
        set(&mut p, 0, 2);
        set(&mut p, 4, 11);
        set(&mut p, 8, 21);
        let buffers = std::array::from_fn(|i| 0x10000 + i as u64 * 4096);
        for (i, va) in buffers.iter().enumerate() {
            p[16 + i * 8..24 + i * 8].copy_from_slice(&va.to_le_bytes());
        }
        set(&mut p, 88, 1);
        assert_eq!(
            decode(version(major), 0x20801211, &p),
            Some(Request {
                client: 11,
                channel: 21,
                kind: Kind::Preemption {
                    flags: 2,
                    gfxp: 1,
                    cilp: 0,
                    buffers
                },
            })
        );
    }
}
#[test]
fn zcull_union_retains_the_logical_client_instead_of_discarding_it() {
    let mut p = vec![0; 256];
    set(&mut p, 0, 11);
    set(&mut p, 4, 21);
    set(&mut p, 8, 0x10000);
    set(&mut p, 16, 2);
    assert_eq!(
        decode(version(580), 0x20801208, &p),
        Some(Request {
            client: 11,
            channel: 21,
            kind: Kind::Zcull {
                va: 0x10000,
                mode: 2
            },
        })
    );
}
#[test]
fn pm_union_decodes_ordinary_context_switch_mode() {
    let mut p = vec![0; 256];
    set(&mut p, 0, 11);
    set(&mut p, 4, 21);
    set(&mut p, 8, 0x10000);
    set(&mut p, 16, 1);
    assert_eq!(
        decode(version(595), 0x20801209, &p),
        Some(Request {
            client: 11,
            channel: 21,
            kind: Kind::Pm {
                va: 0x10000,
                mode: 1
            },
        })
    );
}
#[test]
fn short_and_unknown_layouts_do_not_decode_as_bind_intent() {
    assert_eq!(decode(version(580), 0x20801211, &[0; 111]), None);
    let v = DriverVersion {
        major: 1,
        minor: 0,
        patch: 0,
    };
    assert_eq!(decode(v, 0x20801211, &[0; 256]), None);
}
#[test]
fn nonzero_routes_are_not_silently_rewritten_to_another_engine() {
    for (cmd, at) in [(0x20801211, 96), (0x20801209, 24)] {
        let mut p = [0; 256];
        set(&mut p, at, 1);
        assert_eq!(decode(version(580), cmd, &p), None);
    }
}

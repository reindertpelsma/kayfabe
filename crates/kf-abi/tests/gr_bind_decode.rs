//! Production GR bind contracts with bounded fake host effects.
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

#[test]
fn authored_preemption_bytes_use_owned_fields_and_zero_route_padding() {
    use kf_abi::grbind::encode;
    for major in [580, 595] {
        let mut buffers = [0; 9];
        buffers[0] = 0x12345000;
        buffers[8] = 0x22345000;
        let r = Request {
            client: 61,
            channel: 81,
            kind: Kind::Preemption {
                flags: 3,
                gfxp: 1,
                cilp: 2,
                buffers,
            },
        };
        let (cmd, p) = encode(version(major), r).unwrap();
        assert_eq!(p.len(), 112);
        assert_eq!(&p[4..8], &61u32.to_le_bytes());
        assert_eq!(&p[8..12], &81u32.to_le_bytes());
        assert_eq!(&p[12..16], &[0; 4]);
        assert_eq!(&p[96..], &[0; 16]);
        assert_eq!(decode(version(major), cmd, &p), Some(r));
        assert_eq!(&p[16..24], &0x12345000u64.to_le_bytes());
    }
}
#[test]
fn authored_pm_and_zcull_bytes_preserve_va_but_never_guest_handles_or_routes() {
    use kf_abi::grbind::encode;
    for kind in [
        Kind::Pm {
            va: 0x12345000,
            mode: 1,
        },
        Kind::Zcull {
            va: 0x12345000,
            mode: 2,
        },
    ] {
        let r = Request {
            client: 61,
            channel: 81,
            kind,
        };
        let (cmd, p) = encode(version(595), r).unwrap();
        assert_eq!(&p[..4], &61u32.to_le_bytes());
        assert_eq!(&p[4..8], &81u32.to_le_bytes());
        assert_eq!(&p[8..16], &0x12345000u64.to_le_bytes());
        assert_eq!(decode(version(595), cmd, &p), Some(r));
        if p.len() == 40 {
            assert_eq!(&p[24..], &[0; 16]);
        }
    }
}
#[test]
fn an_old_host_layout_cannot_truncate_a_nonzero_setup_pointer() {
    use kf_abi::grbind::encode;
    let old = DriverVersion {
        major: 535,
        minor: 309,
        patch: 1,
    };
    let mut buffers = [0; 9];
    buffers[0] = 0x10000;
    buffers[8] = 0x20000;
    let mut r = Request {
        client: 61,
        channel: 81,
        kind: Kind::Preemption {
            flags: 2,
            gfxp: 1,
            cilp: 0,
            buffers,
        },
    };
    assert!(encode(old, r).is_none());
    if let Kind::Preemption { buffers, .. } = &mut r.kind {
        buffers[8] = 0;
    }
    let (cmd, p) = encode(old, r).unwrap();
    assert_eq!(p.len(), 104);
    assert_eq!(decode(old, cmd, &p), Some(r));
}
#[test]
fn property_query_never_sends_a_prepopulated_size_or_route() {
    use kf_abi::grbind::{properties_query, properties_reply};
    for major in [580, 595] {
        let v = version(major);
        let (cmd, mut p) = properties_query(v, 11).unwrap();
        assert_eq!(cmd, 0x2080122d);
        assert_eq!(&p[..16], &[0; 16]);
        assert_eq!(&p[20..], &[0; 12]);
        assert_eq!(properties_reply(v, &p), None);
        set(&mut p, 20, 256);
        set(&mut p, 24, 8192);
        p[28] = 1;
        assert_eq!(properties_reply(v, &p), Some((8192, 256)));
    }
}
#[test]
fn route_flags_zero_does_not_authorize_a_nonzero_route_address() {
    for (cmd, at) in [(0x20801211, 104), (0x20801209, 32)] {
        let mut p = [0; 256];
        set(&mut p, at, 0x31);
        assert_eq!(decode(version(595), cmd, &p), None);
    }
}
#[test]
fn an_unpopulated_query_reply_is_not_a_known_extent() {
    use kf_abi::grbind::{properties_query, properties_reply};
    let (_, mut p) = properties_query(version(595), 8).unwrap();
    set(&mut p, 20, 4096);
    set(&mut p, 24, 8192);
    assert_eq!(properties_reply(version(595), &p), None);
    p[28] = 1;
    assert_eq!(properties_reply(version(595), &p), Some((8192, 4096)));
}

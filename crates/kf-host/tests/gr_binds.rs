use kf_abi::grbind::{Kind, Request};
use kf_host::Channel;
use kf_host::grbind::{Host, execute};

#[derive(Default)]
struct Fake {
    queries: Vec<u32>,
    binds: Vec<Request>,
    props: Option<Result<(u64, u64), u32>>,
    status: Option<u32>,
}
impl Host for Fake {
    fn properties(&mut self, engine: u32) -> Result<(u64, u64), u32> {
        self.queries.push(engine);
        self.props.unwrap_or(Ok((4096, 256)))
    }
    fn bind(&mut self, request: Request) -> Result<(), u32> {
        self.binds.push(request);
        self.status.map_or(Ok(()), Err)
    }
}
fn channel() -> Channel {
    Channel {
        tsg: 71,
        chan: 81,
        token: 91,
        born_user: None,
    }
}
fn preempt() -> Request {
    let mut buffers = [0; 9];
    buffers[0] = 0x10000;
    buffers[1] = 0x20000;
    Request {
        client: 11,
        channel: 21,
        kind: Kind::Preemption {
            flags: 2,
            gfxp: 1,
            cilp: 0,
            buffers,
        },
    }
}
fn single(pm: bool) -> Request {
    Request {
        client: 11,
        channel: 21,
        kind: if pm {
            Kind::Pm {
                va: 0x10000,
                mode: 1,
            }
        } else {
            Kind::Zcull {
                va: 0x10000,
                mode: 2,
            }
        },
    }
}
fn covered(va: u64, len: u64) -> bool {
    [0x10000..0x11000, 0x20000..0x21000]
        .iter()
        .any(|r| va >= r.start && va.checked_add(len).is_some_and(|e| e <= r.end))
}

#[test]
fn gfxp_bind_reaches_host_with_owned_handles_and_unchanged_addresses() {
    let mut h = Fake::default();
    assert_eq!(execute(&mut h, preempt(), 61, channel(), covered), Ok(8192));
    assert_eq!(h.queries, [0xb, 0xc]);
    assert_eq!(
        h.binds,
        [Request {
            client: 61,
            channel: 81,
            ..preempt()
        }]
    );
}
#[test]
fn zcull_bind_reaches_owned_channel_without_a_mapping_or_allocation() {
    let mut h = Fake::default();
    assert_eq!(
        execute(&mut h, single(false), 61, channel(), covered),
        Ok(4096)
    );
    assert_eq!(h.queries, [8]);
    assert_eq!(
        h.binds,
        [Request {
            client: 61,
            channel: 81,
            ..single(false)
        }]
    );
}
#[test]
fn pm_bind_uses_native_pm_extent_and_same_guest_va() {
    let mut h = Fake::default();
    assert_eq!(
        execute(&mut h, single(true), 61, channel(), covered),
        Ok(4096)
    );
    assert_eq!(h.queries, [9]);
    assert_eq!(
        h.binds,
        [Request {
            client: 61,
            channel: 81,
            ..single(true)
        }]
    );
}
#[test]
fn compute_cilp_and_graphics_query_both_main_requirements() {
    let mut r = preempt();
    if let Kind::Preemption { flags, cilp, .. } = &mut r.kind {
        *flags = 3;
        *cilp = 2;
    }
    let mut h = Fake::default();
    assert_eq!(execute(&mut h, r, 61, channel(), covered), Ok(8192));
    assert_eq!(h.queries, [0xb, 0xa, 0xc]);
    assert_eq!(h.binds[0].kind, r.kind);
}
#[test]
fn missing_coverage_blocks_graphics_bind_before_any_native_bind() {
    let mut h = Fake::default();
    assert!(execute(&mut h, preempt(), 61, channel(), |_, _| false).is_err());
    assert!(h.binds.is_empty());
}
#[test]
fn a_hole_or_read_only_zcull_buffer_never_binds() {
    let mut h = Fake::default();
    assert!(execute(&mut h, single(false), 61, channel(), |_, _| false).is_err());
    assert!(h.binds.is_empty());
}
#[test]
fn unknown_zero_and_oversized_preempt_extents_fail_closed() {
    for props in [
        Ok((0, 256)),
        Ok((u32::MAX as u64, 4096)),
        Ok((65 << 20, 4096)),
        Err(0x56),
    ] {
        let mut h = Fake {
            props: Some(props),
            ..Fake::default()
        };
        assert!(execute(&mut h, preempt(), 61, channel(), |_, _| true).is_err());
        assert!(h.binds.is_empty());
    }
}
#[test]
fn bad_zcull_extent_and_alignment_never_become_zero_size_success() {
    for props in [
        Ok((0, 4096)),
        Ok((4096, 0)),
        Ok((4096, 3)),
        Ok((65 << 20, 256)),
        Err(0x1a),
    ] {
        let mut h = Fake {
            props: Some(props),
            ..Fake::default()
        };
        assert!(execute(&mut h, single(false), 61, channel(), |_, _| true).is_err());
        assert!(h.binds.is_empty());
    }
}
#[test]
fn native_preemption_status_is_returned_without_ack_or_rewrite() {
    for st in [0x1a, 0x40] {
        let mut h = Fake {
            status: Some(st),
            ..Fake::default()
        };
        assert_eq!(execute(&mut h, preempt(), 61, channel(), covered), Err(st));
        assert_eq!(h.binds.len(), 1);
    }
}
#[test]
fn native_zcull_status_is_returned_without_ack_or_rewrite() {
    for st in [0x1a, 0x40] {
        let mut h = Fake {
            status: Some(st),
            ..Fake::default()
        };
        assert_eq!(
            execute(&mut h, single(false), 61, channel(), covered),
            Err(st)
        );
        assert_eq!(h.binds.len(), 1);
    }
}
#[test]
fn cpu_pointer_domain_is_refused_without_host_effects() {
    let mut r = preempt();
    if let Kind::Preemption { buffers, .. } = &mut r.kind {
        buffers[7] = 0x10000;
    }
    let mut h = Fake::default();
    assert!(execute(&mut h, r, 61, channel(), covered).is_err());
    assert!(h.binds.is_empty());
}
#[test]
fn unknown_flags_and_modes_never_bind() {
    let bad = [
        Kind::Zcull {
            va: 0x10000,
            mode: 3,
        },
        Kind::Pm {
            va: 0x10000,
            mode: 3,
        },
        Kind::Preemption {
            flags: 4,
            gfxp: 1,
            cilp: 0,
            buffers: [0x10000; 9],
        },
    ];
    for kind in bad {
        let mut h = Fake::default();
        assert!(
            execute(
                &mut h,
                Request { kind, ..preempt() },
                61,
                channel(),
                covered
            )
            .is_err()
        );
        assert!(h.binds.is_empty());
    }
}
#[test]
fn buffer_end_overflow_is_refused_even_if_mapping_predicate_accepts_it() {
    let mut h = Fake::default();
    let r = Request {
        kind: Kind::Pm {
            va: u64::MAX - 255,
            mode: 1,
        },
        ..single(true)
    };
    assert!(execute(&mut h, r, 61, channel(), |_, _| true).is_err());
    assert!(h.binds.is_empty());
}
#[test]
fn misaligned_zcull_va_is_refused_before_bind() {
    let mut h = Fake::default();
    let r = Request {
        kind: Kind::Zcull {
            va: 0x10001,
            mode: 2,
        },
        ..single(false)
    };
    assert!(execute(&mut h, r, 61, channel(), |_, _| true).is_err());
    assert!(h.binds.is_empty());
}

#[test]
fn zcull_no_ctxsw_needs_no_save_buffer_and_still_reaches_host() {
    let mut h = Fake::default();
    let r = Request {
        kind: Kind::Zcull { va: 0, mode: 1 },
        ..single(false)
    };
    assert_eq!(
        execute(&mut h, r, 61, channel(), |_, _| panic!("no buffer")),
        Ok(0)
    );
    assert!(h.queries.is_empty());
    assert_eq!(h.binds.len(), 1);
}
#[test]
fn zcull_no_ctxsw_does_not_require_an_available_size_query() {
    let mut h = Fake {
        props: Some(Err(0x56)),
        ..Fake::default()
    };
    let r = Request {
        kind: Kind::Zcull { va: 0, mode: 1 },
        ..single(false)
    };
    assert_eq!(execute(&mut h, r, 61, channel(), |_, _| false), Ok(0));
    assert!(h.queries.is_empty());
    assert_eq!(h.binds[0].channel, 81);
}

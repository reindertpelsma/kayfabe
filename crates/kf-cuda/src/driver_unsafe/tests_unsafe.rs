// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The perimeter's PURE validation sites, tested without a GPU (`v3-sec-rawaddr`, design §8).
//!
//! Every runtime check of the raw tier is a pure function over numbers or states — `sub_range`,
//! `ctx_matches`, `import_len`, `check_args`, `stage_layout`, `Flight::step`, `drop_disposition` —
//! called first by the handle method it guards. Each test below refuses a violating input and
//! accepts the exact fit; each was paired with the mutation that turns it red (recorded in the
//! commit message).
//! No `unsafe` and no address here: a child module cannot see the raw tier's fields.

use super::raw::{
    ArgDesc, ArgKind, Disposition, Flight, FlightEvent, KERNEL_SIGS, KF_ARGS_BYTES, Region,
    check_args, ctx_matches, drop_disposition, import_len, stage_layout, sub_range,
};

/// ★ T2 — V2: an import maps at least one byte. The descriptor and the length cross only inside
/// the `kf_host::RmExport` token (trybuild rows `import_takes_an_rm_export` and
/// `rm_export_is_minted_only_by_kf_host`; the session record behind it is kf-host's
/// `an_export_length_comes_only_from_the_sessions_record`).
#[test]
fn an_import_maps_at_least_one_byte() {
    assert!(import_len(0).is_err(), "a zero-length import");
    assert_eq!(import_len(1), Ok(1), "one byte");
    assert_eq!(import_len(2 << 20), Ok(2 << 20), "a 2 MiB object");
}

/// ★ T1 — V1: a range inside a `len`-byte allocation; overflow checked before the bound.
#[test]
fn a_range_is_inside_its_allocation_or_refused() {
    let len = 4096;
    assert_eq!(sub_range(len, 0, 4096), Some(0), "the exact fit is inside");
    assert_eq!(sub_range(len, 4095, 1), Some(4095), "the last byte");
    assert_eq!(sub_range(len, 4095, 2), None, "off + n == len + 1");
    assert_eq!(sub_range(len, 4096, 1), None, "starts at the end");
    assert_eq!(sub_range(len, u64::MAX, 2), None, "off + n overflows");
    assert_eq!(sub_range(len, 2, u64::MAX), None, "n overflows");
    assert_eq!(sub_range(len, 0, 0), None, "a range is never empty");
}

/// ★ T1b — V1b: an async operation's ranges and stream share one context.
#[test]
fn an_async_operation_stays_in_one_context() {
    assert!(ctx_matches(7, &[7, 7]));
    assert!(
        !ctx_matches(7, &[8]),
        "a range of context B on a stream of context A"
    );
    assert!(!ctx_matches(7, &[7, 8]), "a destination of another context");
}

fn p(len: u64, ctx: u64) -> ArgDesc {
    ArgDesc::Ptr {
        addr: 0x1000,
        len,
        ctx,
    }
}

fn sig(name: &str) -> &'static [ArgKind] {
    KERNEL_SIGS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
        .expect("a pinned kernel")
}

/// The compose kernel's 18 arguments, all of context 1.
fn compose_args() -> Vec<ArgDesc> {
    let mut a = vec![p(4096, 1), p(4096, 1)];
    a.extend(std::iter::repeat_n(ArgDesc::U32, 12));
    a.extend(std::iter::repeat_n(ArgDesc::I32, 4));
    a
}

/// ★★ T3 — V3: count, kinds, contexts, minimum extents, capacities, geometry.
#[test]
fn a_launch_is_refused_unless_every_argument_matches_the_pinned_signature() {
    let compose = sig("kf_compose");
    let ok = compose_args();
    assert_eq!(check_args(1, 1, compose, &ok, 1080, 256, false), Ok(()));
    assert!(
        check_args(1, 1, compose, &ok[..17], 1080, 256, false).is_err(),
        "17 of 18"
    );
    let mut a = ok.clone();
    a[1] = ArgDesc::U32;
    assert!(
        check_args(1, 1, compose, &a, 1080, 256, false).is_err(),
        "a u32 for a pointer"
    );
    let mut a = ok.clone();
    a[0] = p(4096, 2);
    assert!(
        check_args(1, 1, compose, &a, 1080, 256, false).is_err(),
        "a range of another context"
    );
    assert!(
        check_args(1, 2, compose, &ok, 1080, 256, false).is_err(),
        "a stream of another context"
    );
    assert!(
        check_args(1, 1, compose, &ok, 1080, 2048, false).is_err(),
        "block 2048 outside the probe"
    );
    assert_eq!(
        check_args(1, 1, compose, &ok, 1, 2048, true),
        Ok(()),
        "the probe waives the block limit only"
    );
    assert!(
        check_args(1, 1, compose, &ok, 0, 256, false).is_err(),
        "grid 0"
    );
    assert!(
        check_args(1, 1, compose, &ok, 1, 0, false).is_err(),
        "block 0"
    );
    let mut a = ok.clone();
    a[0] = p(3, 1);
    assert!(
        check_args(1, 1, compose, &a, 1, 256, false).is_err(),
        "a pointer below its minimum"
    );

    // A block-taking kernel: only an ArgBlock of the kernel's context.
    let begin = sig("_Z15kf_begin_kernel6KfArgs");
    assert_eq!(
        check_args(1, 1, begin, &[ArgDesc::Block { ctx: 1 }], 1, 1, false),
        Ok(())
    );
    assert!(check_args(1, 1, begin, &[ArgDesc::Block { ctx: 2 }], 1, 1, false).is_err());
    assert!(
        check_args(1, 1, begin, &[p(4096, 1)], 1, 1, false).is_err(),
        "a pointer for the block"
    );

    // ★ A capacity must be OF the range it bounds: kf_par_compact's `cap` bounds `dst`.
    let compact = sig("_Z14kf_par_compact6KfArgsPK5KfEntPKjS4_S4_S4_PS0_j");
    let f40 = 131_072 * 40;
    let dst = ArgDesc::Ptr {
        addr: 0x9000,
        len: f40,
        ctx: 1,
    };
    let mk = |cap: ArgDesc| {
        vec![
            ArgDesc::Block { ctx: 1 },
            p(f40, 1),
            p(131_072 * 4, 1),
            p(131_072 * 4, 1),
            p(131_072 * 4, 1),
            p(4, 1),
            dst,
            cap,
        ]
    };
    let good = ArgDesc::Cap {
        addr: 0x9000,
        len: f40,
        ctx: 1,
        elem: 40,
    };
    assert_eq!(
        check_args(1, 1, compact, &mk(good), 128, 128, false),
        Ok(())
    );
    let other = ArgDesc::Cap {
        addr: 0xA000,
        len: f40,
        ctx: 1,
        elem: 40,
    };
    assert!(
        check_args(1, 1, compact, &mk(other), 128, 128, false).is_err(),
        "the capacity of another range"
    );
    let longer = ArgDesc::Cap {
        addr: 0x9000,
        len: f40 * 2,
        ctx: 1,
        elem: 40,
    };
    assert!(
        check_args(1, 1, compact, &mk(longer), 128, 128, false).is_err(),
        "a capacity larger than its buffer"
    );
    let elem = ArgDesc::Cap {
        addr: 0x9000,
        len: f40,
        ctx: 1,
        elem: 8,
    };
    assert!(
        check_args(1, 1, compact, &mk(elem), 128, 128, false).is_err(),
        "the wrong element size"
    );
    assert!(
        check_args(1, 1, compact, &mk(ArgDesc::U32), 128, 128, false).is_err(),
        "a free u32 as a capacity"
    );
}

/// One `.param` of a PTX entry, as V3 sees it.
fn ptx_kind(decl: &str) -> &'static str {
    if decl.contains(".b8") && decl.contains(&format!("[{KF_ARGS_BYTES}]")) {
        "block"
    } else if decl.contains(".u64") {
        "ptr"
    } else if decl.contains(".u32") {
        "u32"
    } else if decl.contains(".s32") {
        "s32"
    } else {
        "?"
    }
}

/// ★★ T4 — `KERNEL_SIGS` IS the committed PTX: every `.entry` of both PTX files is in the table
/// and every one of the table's kernels has exactly the PTX's parameter kinds, in order.
#[test]
fn the_kernel_table_is_the_ptx() {
    let mut entries = std::collections::BTreeMap::new();
    for ptx in [crate::walk::WALK_PTX, crate::display::SCANOUT_PTX] {
        let text = std::str::from_utf8(ptx).expect("PTX is text");
        for chunk in text.split(".visible .entry ").skip(1) {
            let name = chunk.split('(').next().expect("a name").trim().to_string();
            let params = chunk
                .split('(')
                .nth(1)
                .expect("params")
                .split(')')
                .next()
                .unwrap_or("");
            let kinds: Vec<&str> = params
                .split(',')
                .filter(|s| s.contains(".param"))
                .map(ptx_kind)
                .collect();
            entries.insert(name, kinds);
        }
    }
    assert_eq!(
        entries.len(),
        KERNEL_SIGS.len(),
        "every entry is pinned, and nothing else: {entries:?}"
    );
    for (name, sig) in KERNEL_SIGS {
        let want = entries
            .get(*name)
            .unwrap_or_else(|| panic!("{name} is not an entry of the committed PTX"));
        let have: Vec<&str> = sig
            .iter()
            .map(|k| match k {
                ArgKind::Block => "block",
                ArgKind::Ptr { .. } => "ptr",
                ArgKind::Cap { .. } | ArgKind::U32 => "u32",
                ArgKind::I32 => "s32",
            })
            .collect();
        assert_eq!(&have, want, "{name}");
        for (i, k) in sig.iter().enumerate() {
            if let ArgKind::Cap { of, .. } = k {
                assert!(
                    matches!(sig.get(*of), Some(ArgKind::Ptr { .. })),
                    "{name}: capacity {i} bounds a pointer"
                );
            }
        }
    }
}

/// ★ T21 — V4: the flight state machine.
#[test]
fn the_stage_belongs_to_the_gpu_until_a_proof_returns_it() {
    let idle = Flight::Idle;
    assert_eq!(idle.step(FlightEvent::Access), Ok(Flight::Idle));
    let busy = idle.step(FlightEvent::Begin).expect("begin");
    assert_eq!(busy, Flight::InFlight);
    assert!(
        busy.step(FlightEvent::Access).is_err(),
        "write/read/launch while in flight"
    );
    assert_eq!(
        busy.step(FlightEvent::Begin),
        Ok(Flight::InFlight),
        "a walk is many launches"
    );
    assert_eq!(
        busy.step(FlightEvent::Drained),
        Ok(Flight::Idle),
        "only a proof returns it"
    );
    let bad = busy
        .step(FlightEvent::Error("cuEventQuery: 700".into()))
        .expect("poisons");
    assert!(
        matches!(bad, Flight::Poisoned(_)),
        "a query error is not completion"
    );
    for ev in [FlightEvent::Access, FlightEvent::Begin] {
        assert!(bad.step(ev).is_err(), "poisoned refuses everything");
    }
    assert!(
        matches!(bad.step(FlightEvent::Drained), Ok(Flight::Poisoned(_))),
        "poisoned is final: a later proof does not clear it"
    );
}

/// ★ T22 — F1: a failed drain is never "freed anyway".
#[test]
fn a_failed_drain_leaks_and_never_frees() {
    assert_eq!(drop_disposition::<(), ()>(&Ok(())), Disposition::Release);
    assert_eq!(drop_disposition::<(), ()>(&Err(())), Disposition::Leak);
}

/// ★ V4 — the stage's regions: aligned, monotonic, disjoint, inside the total; the host writes
/// only what the kernels read.
#[test]
fn the_stage_layout_is_aligned_disjoint_and_bounded() {
    let lens = [512, 256, 272, 262_144, 64, 2048, 8 << 20, 2048];
    let (at, total) = stage_layout(&lens).expect("a layout");
    let mut end = 0;
    for (i, (off, len)) in at.iter().enumerate() {
        assert_eq!(off % 64, 0, "region {i} is 64-byte aligned");
        assert!(*off >= end, "region {i} starts after the previous one ends");
        assert_eq!(*len, lens[i]);
        end = off + len;
    }
    assert_eq!(end, total);
    let mut z = lens;
    z[3] = 0;
    assert_eq!(stage_layout(&z), None, "an empty region");
    let mut big = lens;
    big[6] = usize::MAX;
    assert_eq!(stage_layout(&big), None, "an overflow");
    let writable: Vec<Region> = Region::ALL
        .into_iter()
        .filter(|r| r.host_writable())
        .collect();
    assert_eq!(
        writable,
        [
            Region::Pdbs,
            Region::Slots,
            Region::Ack,
            Region::AckCode,
            Region::Lay
        ]
    );
}

/// ★ T18 — no handle of the perimeter prints an address: every type that carries one (a `u64`
/// device address, a `usize` handle or host address, a span) has NO derived `Debug`, and every
/// hand-written `Debug` here names only lengths, counts and names. A source scan, because no
/// handle can be built without a GPU; its known positive is the derive on `Flight` it must allow.
#[test]
fn no_handle_derives_a_pointer_printing_debug() {
    let files = [
        ("driver_unsafe.rs", include_str!("../driver_unsafe.rs")),
        ("walk_gpu_unsafe.rs", include_str!("walk_gpu_unsafe.rs")),
        (
            "display_gpu_unsafe.rs",
            include_str!("display_gpu_unsafe.rs"),
        ),
    ];
    let mut derived = Vec::new();
    for (file, src) in files {
        let lines: Vec<&str> = src.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            if !(l.trim_start().starts_with("#[derive(") && l.contains("Debug")) {
                continue;
            }
            // the item the attribute applies to, and its body up to the closing brace
            let rest = &lines[i + 1..];
            let Some(at) = rest.iter().position(|x| {
                let t = x.trim_start();
                t.starts_with("pub") || t.starts_with("struct") || t.starts_with("enum")
            }) else {
                continue;
            };
            let head = rest[at].trim().to_string();
            let body: String = rest[at..]
                .iter()
                .take_while(|x| !x.starts_with("    }") && !x.starts_with('}'))
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
            for field in ["addr:", "raw:", "host:", "dev:", "base:", "span:", "ptr:"] {
                assert!(
                    !body.contains(field),
                    "{file}: `{head}` derives Debug over an address-carrying field `{field}`"
                );
            }
            derived.push(head);
        }
    }
    assert!(
        derived.iter().any(|h| h.contains("enum Flight")),
        "the scan must SEE a derive it allows, or its silence means nothing: {derived:?}"
    );
}

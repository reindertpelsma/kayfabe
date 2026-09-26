//! ★★★ The generated driver matrix IS the committed measurement (`V3_DRIVER_MATRIX.md` §3–§4).
//!
//! `crates/kf-abi/src/generated/matrix.rs` is emitted from `traces/driver_matrix/ranges.tsv` by
//! `tools/drivermatrix/emit_rust.py`. This test re-reads the TSV and checks every row against
//! the Rust tables, so a hand edit to either side — a "fixed" offset, a dropped run, a tag added
//! to one and not the other — is a red test, not a silently different answer.

use kf_abi::DriverVersion;
use kf_abi::generated::matrix::{ALL_STRUCTS, ALL_VALUES, MEASURED};

const RANGES: &str = include_str!("../../../traces/driver_matrix/ranges.tsv");

fn parse_tag(t: &str) -> DriverVersion {
    DriverVersion::parse(t).unwrap_or_else(|| panic!("ranges.tsv names a non-version {t:?}"))
}

fn tsv_tags() -> Vec<DriverVersion> {
    let line = RANGES
        .lines()
        .find(|l| l.starts_with("# tags\t"))
        .expect("ranges.tsv has a '# tags' line");
    let mut v: Vec<_> = line
        .split('\t')
        .nth(1)
        .expect("tags")
        .split_whitespace()
        .map(parse_tag)
        .collect();
    v.sort();
    v
}

#[test]
fn the_measured_tag_list_is_the_tsvs() {
    assert_eq!(
        MEASURED,
        tsv_tags().as_slice(),
        "MEASURED and ranges.tsv disagree on the tag set"
    );
}

#[test]
fn every_tsv_row_is_what_the_rust_tables_answer() {
    let tags = tsv_tags();
    let mut checked = 0usize;
    for line in RANGES
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 5, "malformed row {line:?}");
        let (kind, item, first, last, value) = (
            cols[0],
            cols[1],
            parse_tag(cols[2]),
            parse_tag(cols[3]),
            cols[4],
        );
        let span: Vec<DriverVersion> = tags
            .iter()
            .copied()
            .filter(|t| first <= *t && *t <= last)
            .collect();
        assert!(!span.is_empty(), "row {line:?} covers no measured tag");
        match kind {
            "value" => {
                let t = ALL_VALUES
                    .iter()
                    .find(|v| v.name == item)
                    .unwrap_or_else(|| panic!("no Rust table for {item}"));
                let want = if value == "ABSENT" {
                    None
                } else {
                    Some(value.parse::<u64>().expect("integer"))
                };
                for v in &span {
                    assert_eq!(t.at(*v).expect("measured"), want, "{item} at {v}");
                    checked += 1;
                }
            }
            "sizeof" | "field" => {
                let (s, path) = match item.split_once('.') {
                    Some((s, p)) if kind == "field" => (s, Some(p)),
                    _ => (item, None),
                };
                let t = ALL_STRUCTS
                    .iter()
                    .find(|x| x.name == s)
                    .unwrap_or_else(|| panic!("no Rust table for {s}"));
                for v in &span {
                    let lay = t.at(*v).expect("measured");
                    match (path, value) {
                        (None, "ABSENT") => assert!(lay.is_none(), "{s} should be absent at {v}"),
                        (None, sz) => {
                            let size: u32 =
                                sz.split('+').nth(1).expect("0+SIZE").parse().expect("int");
                            assert_eq!(lay.expect("present").size, size, "sizeof {s} at {v}");
                        }
                        (Some(p), "ABSENT") => {
                            assert!(
                                lay.is_none_or(|l| l.field(p).is_none()),
                                "{s}.{p} should be absent at {v}"
                            );
                        }
                        (Some(p), at) => {
                            // `OFF+SIZE`, or `OFF+SIZE@ELEM` for an array (ELEM = element bytes,
                            // -1 = multi-dimensional).
                            let (off, rest) = at.split_once('+').expect("OFF+SIZE");
                            let (size, elem) = rest.split_once('@').unwrap_or((rest, "0"));
                            let f = lay
                                .and_then(|l| l.field(p))
                                .unwrap_or_else(|| panic!("{s}.{p} missing at {v}"));
                            assert_eq!(
                                (f.off, f.size, f.elem),
                                (
                                    off.parse().expect("off"),
                                    size.parse().expect("size"),
                                    elem.parse().expect("elem")
                                ),
                                "{s}.{p} at {v}"
                            );
                        }
                    }
                    checked += 1;
                }
            }
            other => panic!("unknown row kind {other:?}"),
        }
    }
    assert!(
        checked > 1000,
        "the TSV was too small to be the committed matrix ({checked} cells)"
    );
}

/// ★ The bench driver is measured — the default guest AND host version must resolve.
#[test]
fn the_bench_driver_is_measured() {
    assert!(kf_abi::matrix::is_measured(kf_abi::versions::BENCH_DRIVER));
}


// =====================================================================================
// ★★★ The transcoder on the REAL layouts (`kf_abi::matrix::transcode`, §4.5)
// =====================================================================================

use kf_abi::matrix::{FieldAt, Layout, Resolved, transcode};

fn parent(path: &str) -> &str {
    path.rfind('.').map_or("", |i| &path[..i])
}

/// A container whose direct children overlap is a union.
fn is_union(l: &Layout, container: &str) -> bool {
    let mut kids: Vec<(u32, i32)> = l
        .fields
        .iter()
        .filter(|(p, _)| *p != "." && !p.ends_with("[]") && parent(p) == container)
        .map(|(_, f)| (f.off, f.size))
        .collect();
    kids.sort_unstable();
    kids.windows(2).any(|w| i64::from(w[0].0) + i64::from(w[0].1.max(0)) > i64::from(w[1].0))
}

/// The fields a round trip can state a value in: scalar leaves (a scalar array counts as a leaf:
/// its element 0 is filled), never inside a union (raw bytes, carried whole), never a bitfield.
fn fillable(l: &Layout) -> Vec<(&'static str, FieldAt)> {
    l.fields
        .iter()
        .filter(|(p, f)| {
            if *p == "." || p.ends_with("[]") || f.size <= 0 {
                return false;
            }
            let has_kids = l.fields.iter().any(|(q, _)| *q != *p && (parent(q) == *p || *q == format!("{p}[]")));
            if has_kids {
                return false;
            }
            // no union anywhere above it
            let mut c = parent(p);
            loop {
                let cc = c.strip_suffix("[]").unwrap_or(c);
                if is_union(l, c) || (cc != c && is_union(l, cc)) {
                    return false;
                }
                if c.is_empty() {
                    break;
                }
                c = parent(c);
            }
            true
        })
        .map(|(p, f)| (*p, *f))
        .collect()
}

/// The absolute offset of a field's element 0 (paths already carry element-0 offsets).
fn width(f: FieldAt) -> usize {
    f.array().map_or(f.bytes().unwrap_or(0), |(e, _)| e)
}

/// ★★★ Every consumed struct, at every measured version whose layout differs from the bench's:
/// a body stating a value in every field BOTH versions have survives bench → version → bench
/// byte for byte. This is what makes a carried control (`kf-rm` `respond_transcoded`) and a
/// carried host ioctl trustworthy on layouts nobody hand-checked: the offsets, element sizes and
/// nesting all come from the measurement, and a shape the transcoder cannot carry must be named
/// in `CANNOT_CARRY` with its reason rather than discovered on a guest.
#[test]
fn every_consumed_struct_round_trips_through_every_measured_layout() {
    // (struct, why) — shapes the generic transcoder refuses by design. Each is served another
    // way or not carried at all; a new entry needs a reason.
    const CANNOT_CARRY: &[(&str, &str)] = &[(
        "GspStaticConfigInfo",
        "served by kf_abi::gspstaticinfo::encode_gsp_static_info_at field by field at the measured \
         layout, never carried; its ecidInfo is ONE struct at 555.42.02 and 595+ and an array of \
         two at 560-590 (array <-> struct: Unsupported by design)",
    )];
    let bench = kf_abi::versions::BENCH_DRIVER;
    let mut carried = 0usize;
    let mut failures = Vec::new();
    for runs in ALL_STRUCTS {
        let Ok(b) = Resolved::of(runs, bench) else { continue };
        for &v in MEASURED {
            let Ok(g) = Resolved::of(runs, v) else { continue };
            if std::ptr::eq(g.layout, b.layout) {
                continue;
            }
            let mut body = vec![0u8; b.size()];
            let mut stated = Vec::new();
            let g_fill = fillable(g.layout);
            for (k, (p, bf)) in fillable(b.layout).into_iter().enumerate() {
                let Some(gf) = g.layout.field(p) else { continue };
                if gf.size <= 0 || !g_fill.iter().any(|(q, _)| *q == p) {
                    continue;
                }
                let o = bf.off();
                let w = width(bf).min(width(gf));
                if w == 0 {
                    continue;
                }
                body[o] = (k % 251 + 1) as u8;
                stated.push((p, o));
            }
            let down = transcode(&b, &g, &body, &[]);
            let back = down.as_ref().map_err(Clone::clone).and_then(|(d, _)| transcode(&g, &b, d, &[]));
            match back {
                Ok((round, _)) => {
                    for (p, o) in &stated {
                        if round[*o] != body[*o] {
                            failures.push(format!("{}.{p} at {v}: {} -> {}", runs.name, body[*o], round[*o]));
                        }
                    }
                    carried += 1;
                }
                Err(e) => {
                    if !CANNOT_CARRY.iter().any(|(n, _)| *n == runs.name) {
                        failures.push(format!("{} at {v}: {e}", runs.name));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} round trip failure(s):\n{}", failures.len(), failures.join("\n"));
    assert!(carried > 100, "only {carried} (struct, version) pairs were carried — the matrix is not the committed one");
}

/// ★ The served controls the kf-rm carry covers: the per-GPC and index-keyed arrays shrink at
/// 575 and widen at 595/610, and every one of them must carry a GA10x-sized body both ways.
#[test]
fn the_reviewed_controls_carry_a_ga10x_body_to_every_measured_version() {
    use kf_abi::generated::matrix as m;
    let bench = kf_abi::versions::BENCH_DRIVER;
    for runs in [
        &m::NV2080_CTRL_INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS_PARAMS,
        &m::NV2080_CTRL_INTERNAL_STATIC_KGR_GET_GLOBAL_SM_ORDER_PARAMS,
        &m::NV2080_CTRL_INTERNAL_STATIC_KGR_GET_INFO_PARAMS,
        &m::NV2080_CTRL_GPU_GET_INFO_V2_PARAMS,
        &m::NV2080_CTRL_FB_GET_INFO_V2_PARAMS,
    ] {
        let b = Resolved::of(runs, bench).expect("bench layout");
        for &v in MEASURED {
            let g = Resolved::of(runs, v).unwrap_or_else(|e| panic!("{} at {v}: {e}", runs.name));
            // A GA102-sized statement: 7 GPCs, 84 SMs — the first entries of every array.
            let mut body = vec![0u8; b.size()];
            for (p, f) in b.layout.fields {
                if let Some((e, n)) = f.array() {
                    for i in 0..n.min(7) {
                        body[f.off() + i * e] = (i + 1) as u8;
                    }
                } else if f.size > 0 && !p.ends_with("[]") && *p != "." {
                    body[f.off()] = 1;
                }
            }
            let (d, _) = transcode(&b, &g, &body, &[]).unwrap_or_else(|e| panic!("{} down to {v}: {e}", runs.name));
            assert_eq!(d.len(), g.size(), "{} at {v}", runs.name);
        }
    }
}

// =====================================================================================
// ★★ INTR_GET_KERNEL_TABLE carried by MEANING (`kf_abi::inittables::intr_kernel_table_at`)
// =====================================================================================

use kf_abi::inittables::{
    INTR_CATEGORY_COUNT, INTR_INVALID_SUBTREE, IntrAtError, IntrTableEntry, encode_intr_kernel_table,
    intr_kernel_table_at,
};

fn mc_idx(name: &str, v: DriverVersion) -> Option<u64> {
    ALL_VALUES
        .iter()
        .find(|r| r.name == format!("mc_engine_idx:{name}"))
        .and_then(|r| r.at(v).ok().flatten())
}

/// ★ The engine numbering is measured at EVERY tag — the carry translates by name, and a tag
/// with no names would compare equal to another tag with no names (see `EngineIdxUnmeasured`).
#[test]
fn mc_engine_idx_is_measured_at_every_tag() {
    for &v in MEASURED {
        let n = ALL_VALUES
            .iter()
            .filter(|r| r.name.starts_with("mc_engine_idx:"))
            .filter(|r| r.at(v).ok().flatten().is_some())
            .count();
        assert!(n > 50, "only {n} MC_ENGINE_IDX values at {v}");
        for e in ["MC_ENGINE_IDX_GR0", "MC_ENGINE_IDX_CE0", "MC_ENGINE_IDX_GSP", "MC_ENGINE_IDX_SEC2"] {
            assert!(mc_idx(e, v).is_some(), "{e} unmeasured at {v}");
        }
    }
}

/// ★★ Every row's engine index lands on the guest version's value for the SAME name, at every
/// measured tag — 535's GR0 is 82 where the bench says 84.
#[test]
fn intr_table_engine_indices_are_carried_by_name_to_every_tag() {
    let bench = kf_abi::versions::BENCH_DRIVER;
    let names = ["MC_ENGINE_IDX_GR0", "MC_ENGINE_IDX_CE0", "MC_ENGINE_IDX_GSP", "MC_ENGINE_IDX_SEC2", "MC_ENGINE_IDX_FIFO"];
    let entries: Vec<IntrTableEntry> = names
        .iter()
        .enumerate()
        .map(|(i, n)| IntrTableEntry {
            engine_idx: u16::try_from(mc_idx(n, bench).expect("bench value")).expect("u16"),
            pmc_intr_mask: 0,
            vector_stall: 0x100 + i as u32,
            vector_non_stall: 0x200 + i as u32,
        })
        .collect();
    // One contiguous run per category (the shape a GA10x host reports), one empty category.
    let mut map = [0u64; INTR_CATEGORY_COUNT];
    for (c, m) in map.iter_mut().enumerate().skip(1) {
        *m = 0b11 << (2 * c);
    }
    let body = encode_intr_kernel_table(&entries, &map).expect("bench table");
    let runs = &kf_abi::generated::matrix::NV2080_CTRL_INTERNAL_INTR_GET_KERNEL_TABLE_PARAMS;
    let mut translated = 0;
    for &v in MEASURED {
        let g = Resolved::of(runs, v).expect("layout");
        let out = intr_kernel_table_at(&body, v).unwrap_or_else(|e| panic!("carry to {v}: {e}"));
        assert_eq!(out.len(), g.size(), "size at {v}");
        let el = g.need("table[]").expect("table[]").bytes().expect("elem");
        let idx = g.need("table[].engineIdx").expect("engineIdx").off();
        let vs = g.need("table[].vectorStall").expect("vectorStall").off();
        for (i, n) in names.iter().enumerate() {
            let want = mc_idx(n, v).expect("measured");
            let got = u16::from_le_bytes([out[idx + i * el], out[idx + i * el + 1]]);
            assert_eq!(u64::from(got), want, "{n} at {v}");
            let stall = u32::from_le_bytes(out[vs + i * el..vs + i * el + 4].try_into().expect("4"));
            assert_eq!(stall, 0x100 + i as u32, "vectorStall row {i} at {v}");
            if u64::from(got) != mc_idx(n, bench).expect("bench") {
                translated += 1;
            }
        }
        if let (Some(s), Some(e)) = (g.maybe("subtreeMap[].subtreeStart"), g.maybe("subtreeMap[].subtreeEnd")) {
            let st = g.need("subtreeMap[]").expect("subtreeMap[]").bytes().expect("elem");
            assert_eq!((out[s.off()], out[e.off()]), (INTR_INVALID_SUBTREE, INTR_INVALID_SUBTREE), "empty category 0 at {v}");
            for c in 1..INTR_CATEGORY_COUNT {
                assert_eq!(
                    (out[s.off() + c * st], out[e.off() + c * st]),
                    ((2 * c) as u8, (2 * c + 1) as u8),
                    "category {c} at {v}"
                );
            }
        }
    }
    assert!(translated > 0, "no tag renumbered any engine — the by-name path was never exercised");
}

/// ⊘ A subtree mask with a hole cannot be said as {start, end}: refused by name at every tag
/// that has the old form, carried as-is where the guest has the mask.
#[test]
fn a_non_contiguous_subtree_mask_is_refused_only_where_the_guest_has_start_end() {
    let mut map = [0u64; INTR_CATEGORY_COUNT];
    map[2] = 0b101;
    let body = encode_intr_kernel_table(&[], &map).expect("bench table");
    let runs = &kf_abi::generated::matrix::NV2080_CTRL_INTERNAL_INTR_GET_KERNEL_TABLE_PARAMS;
    for &v in MEASURED {
        let g = Resolved::of(runs, v).expect("layout");
        let r = intr_kernel_table_at(&body, v);
        if g.maybe("subtreeMap[].subtreeStart").is_some() {
            assert_eq!(r, Err(IntrAtError::NonContiguousSubtree { category: 2, mask: 0b101 }), "{v}");
        } else {
            assert!(r.is_ok(), "{v}: {r:?}");
        }
    }
}

/// ★ 610 renamed the MSENC/BSP caps structs (NVENC/NVDEC; the old names are `#define` aliases
/// DWARF cannot see): the guest's video caps layout still resolves there — never "absent" for a
/// control the guest still sends under its old id.
#[test]
fn the_video_caps_layout_resolves_across_the_610_rename() {
    for &v in MEASURED {
        let Ok(t) = kf_abi::versions::table_for(v) else { continue };
        for cmd in [kf_abi::videocaps::MSENC_GET_CAPS_V2, kf_abi::videocaps::BSP_GET_CAPS_V2] {
            let has_cmd_struct = v >= DriverVersion { major: 550, minor: 40, patch: 7 } || cmd == kf_abi::videocaps::BSP_GET_CAPS_V2;
            let l = t.video_caps_layout(cmd);
            if has_cmd_struct {
                let l = l.unwrap_or_else(|| panic!("{cmd:#x} has no layout at {v}"));
                assert!(l.params_size >= 8 && l.instance_off + 4 <= l.params_size, "{cmd:#x} at {v}: {l:?}");
            }
        }
    }
}

/// ★★ The ≤575.64.05 page-directory carrier (fn 54 / fn 79): at EVERY measured tag the dedicated
/// RPC embeds exactly the control struct — each field at the same offset relative to `params`
/// and the same width — so the control decoder reads the embedded bytes unchanged. And the
/// decode itself returns the namespace and the params window from the measured wrapper.
#[test]
fn the_page_directory_rpcs_embed_the_control_struct_at_every_tag() {
    use kf_abi::generated::matrix as m;
    use kf_abi::matrix::Resolved;
    let pairs = [
        (
            &m::RPC_SET_PAGE_DIRECTORY_V,
            &m::NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS,
        ),
        (
            &m::RPC_UNSET_PAGE_DIRECTORY_V,
            &m::NV0080_CTRL_DMA_UNSET_PAGE_DIRECTORY_PARAMS,
        ),
    ];
    let mut checked = 0;
    for &v in MEASURED {
        for (wrapper, ctrl) in pairs {
            let w = Resolved::of(wrapper, v)
                .unwrap_or_else(|e| panic!("{} at {v}: {e:?}", wrapper.name));
            let c = Resolved::of(ctrl, v).unwrap_or_else(|e| panic!("{} at {v}: {e:?}", ctrl.name));
            let p = w.maybe("params").expect("params");
            assert_eq!(
                p.bytes(),
                Some(c.size()),
                "{} at {v}: params is not the control struct",
                wrapper.name
            );
            for (path, f) in c.layout.fields {
                let g = w
                    .maybe(&format!("params.{path}"))
                    .unwrap_or_else(|| panic!("{}.params.{path} at {v}", wrapper.name));
                assert_eq!(
                    (g.off() - p.off(), g.size),
                    (f.off(), f.size),
                    "{}.params.{path} at {v}",
                    wrapper.name
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 2 * MEASURED.len());

    // The decode, on the measured wrapper of a 575 guest.
    let t = kf_abi::versions::table_for(DriverVersion {
        major: 575,
        minor: 57,
        patch: 8,
    })
    .expect("575 table");
    let mut set = vec![0u8; 48];
    set[0..4].copy_from_slice(&0xc1d0_0001u32.to_le_bytes());
    set[4..8].copy_from_slice(&0xcaf0_0002u32.to_le_bytes());
    set[16..24].copy_from_slice(&0x1_2345_6000u64.to_le_bytes()); // params.physAddress
    set[32..36].copy_from_slice(&0xcaf0_0036u32.to_le_bytes()); // params.hVASpace
    let r = t
        .decode_set_page_directory_rpc(&set)
        .expect("fn 54 decodes");
    assert_eq!(
        (r.client, r.device, r.params.len()),
        (0xc1d0_0001, 0xcaf0_0002, 32)
    );
    let d = t
        .decode_set_page_dir(r.params)
        .expect("the control decoder reads the embedded params");
    assert_eq!((d.phys_address, d.h_vaspace), (0x1_2345_6000, 0xcaf0_0036));
    assert!(
        t.decode_set_page_directory_rpc(&set[..47]).is_err(),
        "a short wrapper is refused"
    );
    let mut unset = vec![0u8; 16];
    unset[8..12].copy_from_slice(&0xcaf0_0036u32.to_le_bytes());
    let u = t
        .decode_unset_page_directory_rpc(&unset)
        .expect("fn 79 decodes");
    assert_eq!(u.params, &unset[8..16]);
}

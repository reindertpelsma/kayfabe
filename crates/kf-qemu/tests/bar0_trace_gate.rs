//! ★★ The BAR0 trace mode (`KF3_BAR0_READ_TRACE`, `x-gsp-observer`) is OFF by default, and off it
//! leaves the device exactly as it was (owner ruling 2026-10-09, `docs/OWNER_RULINGS.md` §X):
//! BAR0's ops are `kf3_piece_ops` (the untouched vCPU functions), every shadow piece keeps ROMD
//! (reads never exit), MSI-X stays on KVM irqfds through the vector notifiers, and no trace event
//! or observer call is reachable.
//!
//! kf3.c is not compiled by CI (it needs a QEMU tree), so this reads its source (as
//! `kf3_realize_discard.rs` does) and checks that every diagnostic path sits behind the realize-time
//! switches, and that the switches come only from Rust's `kf3_trace_mode` (default off, its own
//! unit tests: `readtrace::tests::the_default_configuration_is_off_and_gates_every_path`) and the
//! unset-by-default `x-gsp-observer` property.

use std::path::Path;

fn strip_c_comments(src: &str) -> String {
    let mut clean = String::new();
    let mut rest = src;
    while let Some((before, comment)) = rest.split_once("/*") {
        clean.push_str(before);
        rest = comment.split_once("*/").expect("unterminated C comment").1;
    }
    clean.push_str(rest);
    clean
}

/// The body of the C function `name` (from its definition to the first `}` in column 0).
fn function<'a>(src: &'a str, name: &str) -> &'a str {
    // its definition: a line starting with `static` that names it (` name(` or `*name(`)
    let mut at = 0;
    let start = loop {
        let line_end = src[at..].find('\n').map_or(src.len(), |n| at + n);
        let line = &src[at..line_end];
        if line.starts_with("static")
            && (line.contains(&format!(" {name}(")) || line.contains(&format!("*{name}(")))
        {
            break at;
        }
        assert!(line_end < src.len(), "{name} is not defined");
        at = line_end + 1;
    };
    let len = src[start..]
        .find("\n}\n")
        .unwrap_or_else(|| panic!("{name} has no end"));
    &src[start..start + len + 2]
}

fn kf3_c() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    strip_c_comments(&std::fs::read_to_string(root.join("qemu/hw/misc/kf3/kf3.c")).unwrap())
}

/// Every occurrence of `needle` lies inside one of `allowed` functions.
fn only_in(src: &str, needle: &str, allowed: &[&str]) {
    let bodies: Vec<(usize, usize)> = allowed
        .iter()
        .map(|f| {
            let b = function(src, f);
            let s = src.find(b).unwrap();
            (s, s + b.len())
        })
        .collect();
    for (i, _) in src.match_indices(needle) {
        assert!(
            bodies.iter().any(|&(s, e)| (s..e).contains(&i)),
            "`{needle}` used outside {allowed:?} at byte {i}"
        );
    }
}

#[test]
fn the_switches_come_from_rust_and_the_unset_property_only() {
    let src = kf3_c();
    let realize = function(&src, "kf3_dev_realize");
    assert!(realize.contains("s->tr_on = kf3_trace_mode(s->h) == 1;"));
    assert!(realize.contains("s->diag = s->tr_on || s->gsp_observer_path;"));
    // nothing else assigns them
    assert_eq!(src.matches("s->tr_on =").count(), 1);
    assert_eq!(src.matches("s->diag =").count(), 1);
    // the observer property has no default path
    assert!(src.contains("DEFINE_PROP_STRING(\"x-gsp-observer\", Kf3State, gsp_observer_path)"));
}

#[test]
fn off_bar0_is_built_with_the_untouched_ops_and_romd() {
    let src = kf3_c();
    // the trace ops are chosen in one place, only under the switch
    only_in(&src, "&kf3_piece_ops_trace", &["kf3_piece_ops_for"]);
    assert!(
        function(&src, "kf3_piece_ops_for")
            .contains("return s->diag ? &kf3_piece_ops_trace : &kf3_piece_ops;")
    );
    // the vCPU's own functions carry no diagnostic call
    for f in ["kf3_piece_read", "kf3_piece_write"] {
        let body = function(&src, f);
        for bad in ["trace_", "kf3_obs_mmio", "gsp_observer", "tr_on", "diag"] {
            assert!(!body.contains(bad), "{f} contains {bad}");
        }
    }
    // a piece loses ROMD for the run only in trace mode and only when Rust selects it
    let build = function(&src, "kf3_bar0_build");
    let romd = build
        .find("memory_region_rom_device_set_romd(&p->mr, false);")
        .expect("the trace mode's ROMD switch");
    let guard = build[..romd]
        .rfind("if (s->tr_on && kf3_trace_piece(s->h, r->base, r->len))")
        .expect("ROMD off without the trace-mode guard");
    assert!(!build[guard..romd].contains('}'));
    assert_eq!(build.matches("set_romd").count(), 1);
    // the existing window trap leaves a non-trace piece exactly as before (!on)
    assert!(function(&src, "kf3_read_trap_bh").contains("!(on || s->pieces[i].rtrace)"));
    only_in(&src, "p->rtrace = true", &["kf3_bar0_build"]);
}

#[test]
fn off_msi_stays_on_irqfds_and_no_trace_or_observer_call_is_reachable() {
    let src = kf3_c();
    let realize = function(&src, "kf3_dev_realize");
    assert!(realize.contains(
        "if (!s->diag &&\n            msix_set_vector_notifiers(pci, kf3_vector_use, kf3_vector_release, NULL) < 0)"
    ));
    let fd = realize
        .find("qemu_set_fd_handler(fd, kf3_msi_user")
        .expect("main-loop MSI path");
    let guard = realize[..fd]
        .rfind("if (s->diag) {")
        .expect("unguarded MSI path");
    assert!(!realize[guard..fd].contains('}'));
    assert_eq!(src.matches("kf3_msi_user, NULL").count(), 1);
    // QEMU's vfio trace events: only in the trace helpers
    only_in(&src, "trace_vfio_", &["kf3_trace_rw", "kf3_msi_user"]);
    // the helpers are reached only from the trace ops and the main-loop MSI path
    only_in(
        &src,
        "kf3_trace_rw(",
        &[
            "kf3_trace_rw",
            "kf3_piece_read_trace",
            "kf3_piece_write_trace",
        ],
    );
    only_in(
        &src,
        "kf3_obs_mmio(",
        &[
            "kf3_obs_mmio",
            "kf3_piece_read_trace",
            "kf3_piece_write_trace",
        ],
    );
    only_in(&src, "gsp_observer_mmio(", &["kf3_obs_mmio"]);
    only_in(&src, "gsp_observer_irq(", &["kf3_msi_user"]);
    // the observer opens only for the property
    let open = realize.find("gsp_observer_open(").expect("observer open");
    assert!(
        realize[..open]
            .rfind("if (s->gsp_observer_path) {")
            .is_some()
    );
}

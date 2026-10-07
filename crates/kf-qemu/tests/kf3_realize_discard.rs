//! ★★ kf3's realize takes `ram_block_discard_require(true)` FIRST and gives it back on every
//! failure after it (2026-10-03, third review of `v3-scratch-bound`).
//!
//! The requirement is how kf3 refuses to share a VM with a device that pins guest RAM for DMA
//! (VFIO, iommufd, vfio-user, vhost-vdpa, `nvme://`, libblkio, SEV, COLO; `V3_P4_PORT_MAP.md` Q3).
//! That refusal is an expected outcome, so it must cost nothing: taken AFTER `kf3_realize`, as it
//! first was, every refused realize leaked the host store reservation, the RM client, the threads
//! and the windows for QEMU's life, once per `device_add` retry. And QEMU 10.2.4 calls no `exit`
//! for a failed realize (`hw/pci/pci.c` `pci_qdev_realize` only unregisters the device), so a
//! failure after the requirement that does not give it back keeps every later VFIO device out.
//!
//! kf3.c is not compiled by CI (it needs a QEMU tree), so this reads its source: the order of the
//! two calls, and that no failure path after the requirement leaves without passing the label
//! that releases it.

use std::path::Path;

/// C source without its `/* */` comments (a comment may name either call).
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
    let start = src
        .find(&format!("static void {name}("))
        .unwrap_or_else(|| panic!("{name} is not defined"));
    let len = src[start..]
        .find("\n}\n")
        .unwrap_or_else(|| panic!("{name} has no end"));
    &src[start..start + len + 2]
}

/// The order and release rules, as a list of violations (empty = holds).
fn violations(kf3_c: &str) -> Vec<String> {
    let src = strip_c_comments(kf3_c);
    let realize = function(&src, "kf3_dev_realize");
    let mut bad = Vec::new();
    let (Some(req), Some(build)) = (
        realize.find("ram_block_discard_require(true)"),
        realize.find("kf3_realize("),
    ) else {
        return vec!["kf3_dev_realize calls neither the requirement nor kf3_realize".into()];
    };
    if req > build {
        bad.push("ram_block_discard_require(true) comes after kf3_realize".into());
    }
    let Some(held) = realize.find("s->discard_required = true;") else {
        bad.push("the requirement is never recorded as held".into());
        return bad;
    };
    let Some(label) = realize.find("\nfail:") else {
        bad.push("no `fail:` label".into());
        return bad;
    };
    let (path, fail) = (&realize[held..label], &realize[label..]);
    // Exactly one `return;` after the requirement is held: the success return, last before
    // `fail:`. Any other one leaves a failed realize holding the requirement.
    let returns = path.matches("return;").count();
    if returns != 1 || !path.trim_end().ends_with("return;") {
        bad.push(format!(
            "{returns} `return;` after the requirement is held; only the success return, right \
             before `fail:`, may leave without releasing it"
        ));
    }
    if !path.contains("goto fail;") {
        bad.push("no failure path goes to `fail`".into());
    }
    if !(fail.contains("ram_block_discard_require(false)")
        && fail.contains("s->discard_required = false;"))
    {
        bad.push("`fail:` does not give the requirement back".into());
    }
    let exit = function(&src, "kf3_dev_exit");
    if !exit.contains("ram_block_discard_require(false)") {
        bad.push("kf3_dev_exit does not give the requirement back".into());
    }
    bad
}

#[test]
fn kf3_takes_the_discard_requirement_first_and_releases_it_on_every_failure() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../qemu/hw/misc/kf3/kf3.c");
    let src = std::fs::read_to_string(&path).expect("kf3.c");
    assert_eq!(violations(&src), Vec::<String>::new(), "{}", path.display());
}

/// ★ The known-positives: the shape before this fix (requirement last, bare returns), and one
/// failure path turned back into a bare `return;`. If either reads clean, the check is blind.
#[test]
fn the_check_sees_a_late_requirement_and_a_leaking_return() {
    let late = "static void kf3_dev_realize(PCIDevice *pci, Error **errp)\n{\n\
                if (kf3_realize(a, &s->h) != 0) {\n return;\n }\n\
                if (ram_block_discard_require(true) != 0) {\n return;\n }\n\
                s->discard_required = true;\n info_report(\"x\");\n}\n\
                static void kf3_dev_exit(PCIDevice *pci)\n{\n ram_block_discard_require(false);\n}\n";
    let v = violations(late);
    assert!(
        v.iter().any(|m| m.contains("comes after kf3_realize")),
        "{v:?}"
    );
    assert!(v.iter().any(|m| m.contains("no `fail:` label")), "{v:?}");

    let leaking = "static void kf3_dev_realize(PCIDevice *pci, Error **errp)\n{\n\
                   if (ram_block_discard_require(true) != 0) {\n return;\n }\n\
                   s->discard_required = true;\n\
                   if (kf3_realize(a, &s->h) != 0) {\n goto fail;\n }\n\
                   if (!kf3_bar0_build(s)) {\n return;\n }\n\
                   info_report(\"x\");\n return;\n\nfail:\n\
                   ram_block_discard_require(false);\n s->discard_required = false;\n}\n\
                   static void kf3_dev_exit(PCIDevice *pci)\n{\n ram_block_discard_require(false);\n}\n";
    let v = violations(leaking);
    assert_eq!(v.len(), 1, "{v:?}");
    assert!(v[0].starts_with("2 `return;`"), "{v:?}");
}
